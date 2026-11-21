#[test]
fn local_port_is_the_side_before_the_arrow() {
    assert_eq!(
        crate::local_proc::local_endpoint_port("127.0.0.1:54321->1.2.3.4:443"),
        Some(54321)
    );
    assert_eq!(
        crate::local_proc::local_endpoint_port("[::1]:54321->1.2.3.4:443"),
        Some(54321)
    );
    assert_eq!(
        crate::local_proc::local_endpoint_port("10.0.0.5:8080->1.2.3.4:80"),
        Some(8080)
    );
    assert_eq!(
        crate::local_proc::local_endpoint_port("192.168.1.5:443->192.168.1.9:54321"),
        Some(443)
    );
    assert_eq!(
        crate::local_proc::local_endpoint_port("127.0.0.1:443"),
        Some(443)
    );
}

#[test]
fn forward_errors_include_a_guess() {
    let dead = anyhow::Error::msg("connection was not ready").context("HTTP/2 上游");
    let text = super::explain_flow_error(&dead);
    assert!(text.starts_with("HTTP/2 上游。猜测："), "{text}");
    assert!(text.contains("重新连接"), "{text}");

    let reset = anyhow::Error::from(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "connection reset",
    ))
    .context("HTTP/2 上游");
    assert!(super::explain_flow_error(&reset).contains("重置"));

    let timeout = anyhow::Error::msg("deadline has elapsed").context("连接源站超时");
    assert!(super::explain_flow_error(&timeout).contains("时限"));

    let clear = anyhow::Error::msg("编辑后的 URL 不能重放");
    assert_eq!(super::explain_flow_error(&clear), "编辑后的 URL 不能重放");

    let h1 = anyhow::Error::msg("upstream").context("上游 HTTP/1.1 没有返回");
    assert!(super::explain_flow_error(&h1).contains("没有把响应"));

    let closed = anyhow::Error::msg("gone").context("上游 HTTP/1.1 已经关闭");
    assert!(super::explain_flow_error(&closed).contains("HTTP/1.1 连接已经结束"));

    let mapped = anyhow::Error::msg("gone").context("映射连接未就绪");
    assert!(super::explain_flow_error(&mapped).contains("转发规则"));

    let kept = anyhow::Error::msg("证书被拒绝。猜测：客户端不信任 zniff CA。");
    assert_eq!(super::explain_flow_error(&kept), kept.to_string());
}

#[test]
fn client_tls_reason_guesses_pinning_or_an_untrusted_ca() {
    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
    assert!(super::client_tls_reason(&reset, "www.google.com").contains("证书钉扎"));
    assert!(super::client_tls_reason(&reset, "api.example.com").contains("zniff CA"));
    let alert = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        rustls::Error::AlertReceived(rustls::AlertDescription::UnknownCA),
    );
    assert!(super::client_tls_reason(&alert, "api.example.com").contains("不信任 zniff CA"));
    assert!(!super::likely_pinned("notgoogle.com"));
    assert!(super::likely_pinned("mail.google.com"));
}

#[test]
fn origin_tls_reason_names_the_certificate_or_handshake() {
    let unknown = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
    );
    let unknown = anyhow::Error::from(unknown).context("源站 TLS");
    assert_eq!(
        super::origin_tls_reason(&unknown),
        "源站证书不在内置根证书里，已改为隧道"
    );

    let alert = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure),
    );
    let alert = anyhow::Error::from(alert).context("源站 TLS");
    assert_eq!(
        super::origin_tls_reason(&alert),
        "源站中止握手（HandshakeFailure），已改为隧道"
    );

    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
    let reset = anyhow::Error::from(reset).context("源站 TLS");
    assert_eq!(
        super::origin_tls_reason(&reset),
        "源站 TLS 失败，已改为隧道: connection reset"
    );
}

#[test]
fn origin_tls_reason_differentiates_client_cert_and_preserves_context() {
    // 1. Encrypted key error preservation
    let encrypted_root = anyhow::anyhow!(
        "私钥已加密（ENCRYPTED PRIVATE KEY），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl pkcs8 -in encrypted.key -out decrypted.key）"
    );
    let err = encrypted_root
        .context("解析私钥 /var/certs/badssl.key")
        .context("读取客户端证书 /var/certs/badssl.crt")
        .context("加载源站 mTLS 客户端证书 (client.badssl.com)");

    let reason = super::origin_tls_reason(&err);
    assert!(
        reason.starts_with("客户端证书加载失败: "),
        "must have client cert failure prefix: {reason}"
    );
    assert!(
        reason.contains("client.badssl.com"),
        "must preserve host: {reason}"
    );
    assert!(
        reason.contains("/var/certs/badssl.crt"),
        "must preserve cert path: {reason}"
    );
    assert!(
        reason.contains("/var/certs/badssl.key"),
        "must preserve key path: {reason}"
    );
    assert!(
        reason.contains("ENCRYPTED PRIVATE KEY"),
        "must preserve encrypted key hint: {reason}"
    );
    assert!(
        reason.contains("openssl pkcs8 -in encrypted.key -out decrypted.key"),
        "must preserve openssl remediation command: {reason}"
    );
    assert!(
        !reason.contains("源站 TLS 失败"),
        "must NOT be labeled as origin TLS failure: {reason}"
    );

    // 2. Client auth key mismatch preservation
    let mismatch = rustls::Error::InconsistentKeys(rustls::InconsistentKeys::Unknown);
    let err = anyhow::Error::from(mismatch).context(
            "配置源站 mTLS 客户端证书 (client.badssl.com, cert=/var/certs/badssl.crt, key=/var/certs/badssl.key)",
        );
    let reason = super::origin_tls_reason(&err);
    assert!(
        reason.starts_with("客户端证书加载失败: "),
        "must have client cert failure prefix: {reason}"
    );
    assert!(
        reason.contains("client.badssl.com"),
        "must preserve host: {reason}"
    );
    assert!(
        reason.contains("/var/certs/badssl.crt") && reason.contains("/var/certs/badssl.key"),
        "must preserve cert and key paths on KeyMismatch: {reason}"
    );
    assert!(
        reason.contains("KeyMismatch") || reason.contains("keys may not be consistent"),
        "must preserve mismatch details: {reason}"
    );
    assert!(
        !reason.contains("源站 TLS 失败"),
        "must NOT be labeled as origin TLS failure: {reason}"
    );

    // 3. File not found preservation
    let io_err = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "No such file or directory (os error 2)",
    );
    let err = anyhow::Error::from(io_err)
        .context("读取私钥文件 /var/certs/missing.key")
        .context("读取客户端证书 /var/certs/missing.crt")
        .context("加载源站 mTLS 客户端证书 (api.internal.net)");
    let reason = super::origin_tls_reason(&err);
    assert!(
        reason.starts_with("客户端证书加载失败: "),
        "must have client cert failure prefix: {reason}"
    );
    assert!(
        reason.contains("api.internal.net"),
        "must preserve host: {reason}"
    );
    assert!(
        reason.contains("/var/certs/missing.key"),
        "must preserve missing file: {reason}"
    );
    assert!(
        reason.contains("No such file or directory"),
        "must preserve io error: {reason}"
    );
}

#[tokio::test]
async fn client_cert_loading_failure_records_clear_reason() {
    let dir = std::env::temp_dir().join(format!("zniff-mtls-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let client_certs_dir = dir.join("client-certs");
    std::fs::create_dir_all(&client_certs_dir).unwrap();

    let (_, cert, _) = test_origin_cert();
    std::fs::write(client_certs_dir.join("mtls.badssl.com.crt"), cert.pem()).unwrap();
    std::fs::write(
        client_certs_dir.join("mtls.badssl.com.key"),
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIF\n-----END ENCRYPTED PRIVATE KEY-----\n",
    )
    .unwrap();

    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let res = hub.client_config_for_host("mtls.badssl.com", &[b"http/1.1".to_vec()]);
    assert!(res.is_err());
    let err = res.unwrap_err();
    let reason = super::origin_tls_reason(&err);

    assert!(reason.starts_with("客户端证书加载失败: "));
    assert!(reason.contains("mtls.badssl.com"));
    assert!(reason.contains("mtls.badssl.com.crt"));
    assert!(reason.contains("mtls.badssl.com.key"));
    assert!(reason.contains("ENCRYPTED PRIVATE KEY"));
    assert!(reason.contains("openssl pkcs8 -in encrypted.key -out decrypted.key"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_is_loopback_host() {
    assert!(super::is_loopback_host("localhost"));
    assert!(super::is_loopback_host("localhost:8877"));
    assert!(super::is_loopback_host("LOCALHOST:8877"));
    assert!(super::is_loopback_host("127.0.0.1"));
    assert!(super::is_loopback_host("127.0.0.1:8877"));
    assert!(super::is_loopback_host("127.0.0.2:8877"));
    assert!(super::is_loopback_host("[::1]"));
    assert!(super::is_loopback_host("[::1]:8877"));
    assert!(super::is_loopback_host("::1"));

    assert!(!super::is_loopback_host(""));
    assert!(!super::is_loopback_host("attacker.example"));
    assert!(!super::is_loopback_host("attacker.example:8877"));
    assert!(!super::is_loopback_host("192.168.1.10:8877"));
    assert!(!super::is_loopback_host("0.0.0.0:8877"));
    assert!(!super::is_loopback_host("evil.localhost:8877"));
    assert!(!super::is_loopback_host("localhost.evil.com:8877"));
}

#[tokio::test]
async fn local_mcp_security_checks() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-mcp-sec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

    // 1. Foreign Host (DNS-rebinding attempt) -> 403 Forbidden
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: attacker.example:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        proxy_addr.port(),
        payload.len()
    );
    let mut full_req = req.into_bytes();
    full_req.extend_from_slice(payload);
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(&full_req).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
        "Foreign Host must be rejected with 403: {text}"
    );

    // 2. Cross-origin request with Origin header -> 403 Forbidden
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://attacker.example\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        proxy_addr.port(),
        payload.len()
    );
    let mut full_req = req.into_bytes();
    full_req.extend_from_slice(payload);
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(&full_req).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
        "Request with Origin must be rejected with 403: {text}"
    );

    // 3. Non-JSON Content-Type (CORS simple request with text/plain) -> 415 Unsupported Media Type
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        proxy_addr.port(),
        payload.len()
    );
    let mut full_req = req.into_bytes();
    full_req.extend_from_slice(payload);
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(&full_req).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.starts_with("HTTP/1.1 415 Unsupported Media Type\r\n"),
        "Non-JSON Content-Type must be rejected with 415: {text}"
    );

    // 4. CORS preflight OPTIONS request -> 403 Forbidden
    let req = format!(
        "OPTIONS /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://attacker.example\r\nAccess-Control-Request-Method: POST\r\nConnection: close\r\n\r\n",
        proxy_addr.port()
    );
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.starts_with("HTTP/1.1 403 Forbidden\r\n"),
        "CORS preflight OPTIONS must be rejected with 403: {text}"
    );

    // 5. Valid loopback request with localhost -> 200 OK
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        proxy_addr.port(),
        payload.len()
    );
    let mut full_req = req.into_bytes();
    full_req.extend_from_slice(payload);
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(&full_req).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(
        text.starts_with("HTTP/1.1 200 OK\r\n"),
        "Valid localhost request must succeed: {text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_read_header_block_prefixed() {
    let initial = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
    let mut empty_stream = tokio::io::empty();
    let (head, rest) = super::read_header_block_prefixed(&mut empty_stream, initial)
        .await
        .unwrap();
    assert_eq!(head, b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n");
    assert_eq!(rest, b"hello");

    // Partial in prefix, rest in stream
    let initial = b"HTTP/1.1 200 OK\r\n".to_vec();
    let stream_bytes = b"Content-Length: 0\r\n\r\n";
    let mut stream = stream_bytes.as_slice();
    let (head, rest) = super::read_header_block_prefixed(&mut stream, initial)
        .await
        .unwrap();
    assert_eq!(head, b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(rest, b"");
}

#[tokio::test]
async fn plain_http_expect_100_continue_records_final_response() {
    let dir = std::env::temp_dir().join(format!("zniff-100-continue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let req_body = b"hello-upload-data-12345";
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut total_read = 0;
        while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf[total_read..]).await.unwrap();
            if n == 0 {
                break;
            }
            total_read += n;
        }
        let head = String::from_utf8_lossy(&buf[..total_read]);
        assert!(
            head.to_ascii_lowercase().contains("expect: 100-continue"),
            "Origin must see Expect: 100-continue header"
        );
        // Verify origin has NOT received any body bytes yet because client waits for 100 Continue
        let head_len = buf[..total_read]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap();
        assert_eq!(
            total_read, head_len,
            "Origin must not receive any body before sending 100 Continue"
        );

        // Origin sends 100 Continue interim response
        sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .unwrap();

        // Origin now reads the upload body
        let mut body_buf = Vec::new();
        while body_buf.len() < req_body.len() {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            body_buf.extend_from_slice(&buf[..n]);
        }
        assert_eq!(&body_buf[..req_body.len()], req_body);

        // Origin sends final 200 OK response with body
        let body = b"{\"result\":\"ok\"}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
        let mut drain = [0u8; 64];
        while let Ok(n) = sock.read(&mut drain).await {
            if n == 0 {
                break;
            }
        }
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "POST http://{origin_addr}/upload HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        req_body.len()
    );
    // Client sends headers ONLY
    client.write_all(req.as_bytes()).await.unwrap();

    // Client explicitly awaits HTTP/1.1 100 Continue before sending body
    let mut interim_buf = Vec::new();
    let mut byte = [0u8; 1];
    while !interim_buf.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(3), client.read(&mut byte))
            .await
            .expect("Client timed out waiting for 100 Continue (proxy deadlock detected!)")
            .unwrap();
        assert_ne!(n, 0, "Proxy closed connection before sending 100 Continue");
        interim_buf.push(byte[0]);
        assert!(
            interim_buf.len() < 1024,
            "Interim response header too large"
        );
    }
    let interim_text = String::from_utf8_lossy(&interim_buf);
    assert!(
        interim_text.starts_with("HTTP/1.1 100"),
        "Expected 100 Continue, got: {interim_text}"
    );

    // Client sends request body now that 100 Continue has arrived
    client.write_all(req_body).await.unwrap();

    // Client reads final 200 OK response
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .expect("Client timed out waiting for final 200 OK")
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.contains("HTTP/1.1 200 OK"),
        "client must receive final 200 OK: {text}"
    );
    assert!(
        text.contains("{\"result\":\"ok\"}"),
        "client must receive body: {text}"
    );

    let flow = hub.with_session(|session| session.get(1).cloned());
    let flow = flow.expect("flow");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.request_body, req_body);
    assert_eq!(flow.response_body, b"{\"result\":\"ok\"}");
    assert!(
        flow.elapsed_ms().is_some(),
        "flow must be marked finished with elapsed time"
    );
    let has_ct = flow
        .response_headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("application/json"));
    assert!(has_ct, "response headers must be recorded");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_expect_100_continue_early_rejection_413() {
    let dir = std::env::temp_dir().join(format!("zniff-100-continue-413-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut total_read = 0;
        while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf[total_read..]).await.unwrap();
            if n == 0 {
                break;
            }
            total_read += n;
        }
        let head = String::from_utf8_lossy(&buf[..total_read]);
        assert!(head.to_ascii_lowercase().contains("expect: 100-continue"));
        assert!(
            head.to_ascii_lowercase()
                .contains("content-length: 10485760")
        );

        // Origin rejects immediately with 413 Payload Too Large without sending 100 Continue
        let body = b"{\"error\":\"payload_too_large\"}";
        let resp = format!(
            "HTTP/1.1 413 Payload Too Large\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    // Client declares a 10MB payload with Expect: 100-continue
    let req = format!(
        "POST http://{origin_addr}/upload HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: 10485760\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    // Under RFC 9110 §10.1.1, client does NOT send body bytes when awaiting 100-continue.
    // If zniff blocks waiting for 10MB body, client read_to_end will time out (deadlock!).
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(3), client.read_to_end(&mut resp))
            .await
            .expect("Client timed out waiting for 413 rejection (deadlock detected: proxy blocked waiting for body!)")
            .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.contains("HTTP/1.1 413"),
        "Client must receive 413 response: {text}"
    );
    assert!(
        text.contains("payload_too_large"),
        "Client must receive 413 body: {text}"
    );

    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.status, Some(413));
    assert!(
        flow.request_body.is_empty(),
        "No request body should be recorded since client never sent one"
    );
    assert_eq!(flow.response_body, b"{\"error\":\"payload_too_large\"}");
    assert!(
        flow.elapsed_ms().is_some(),
        "Flow must be marked finished with elapsed time"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_expect_100_continue_with_early_hints_103_then_continue() {
    let dir = std::env::temp_dir().join(format!("zniff-103-then-100-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let req_body = b"multi-interim-payload";
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut total_read = 0;
        while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf[total_read..]).await.unwrap();
            if n == 0 {
                break;
            }
            total_read += n;
        }
        // 1. Origin sends 103 Early Hints
        sock.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </app.js>; rel=preload\r\n\r\n")
            .await
            .unwrap();
        // 2. Origin sends 100 Continue
        sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .unwrap();
        // 3. Origin reads body
        let mut body_buf = Vec::new();
        while body_buf.len() < req_body.len() {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            body_buf.extend_from_slice(&buf[..n]);
        }
        assert_eq!(&body_buf[..req_body.len()], req_body);
        // 4. Origin sends 200 OK
        let body = b"{\"done\":true}";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "POST http://{origin_addr}/multi HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        req_body.len()
    );
    client.write_all(req.as_bytes()).await.unwrap();

    // Client reads 103 Early Hints first
    let mut byte = [0u8; 1];
    let mut interim_103 = Vec::new();
    while !interim_103.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(3), client.read(&mut byte))
            .await
            .expect("Client timed out waiting for 103 Early Hints")
            .unwrap();
        assert_ne!(
            n, 0,
            "Proxy closed connection before sending 103 Early Hints"
        );
        interim_103.push(byte[0]);
    }
    let text_103 = String::from_utf8_lossy(&interim_103);
    assert!(
        text_103.starts_with("HTTP/1.1 103 Early Hints"),
        "{text_103}"
    );

    // Client reads 100 Continue second
    let mut interim_100 = Vec::new();
    while !interim_100.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(3), client.read(&mut byte))
            .await
            .expect("Client timed out waiting for 100 Continue")
            .unwrap();
        assert_ne!(n, 0, "Proxy closed connection before sending 100 Continue");
        interim_100.push(byte[0]);
    }
    let text_100 = String::from_utf8_lossy(&interim_100);
    assert!(text_100.starts_with("HTTP/1.1 100 Continue"), "{text_100}");

    // Now client writes body
    client.write_all(req_body).await.unwrap();

    // Client reads final 200 OK
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .expect("Client timed out waiting for 200 OK")
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("HTTP/1.1 200 OK"), "{text}");
    assert!(text.contains("{\"done\":true}"), "{text}");

    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.request_body, req_body);
    assert_eq!(flow.response_body, b"{\"done\":true}");
    assert!(flow.elapsed_ms().is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_adversarial_100_continue_client_waits_for_continue() {
    let dir = std::env::temp_dir().join(format!("zniff-100-continue-adv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut total_read = 0;
        while !buf[..total_read].windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf[total_read..]).await.unwrap();
            if n == 0 {
                break;
            }
            total_read += n;
        }
        sock.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .unwrap();
        let mut body = [0u8; 5];
        sock.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"hello");
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "POST http://{origin_addr}/adv HTTP/1.1\r\nHost: {origin_addr}\r\nExpect: 100-continue\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut interim_buf = [0u8; 1024];
    let res = timeout(Duration::from_millis(2000), client.read(&mut interim_buf)).await;
    assert!(
        res.is_ok(),
        "DEADLOCK: Client timed out waiting for 100 Continue!"
    );
    let n = res.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&interim_buf[..n]).contains("100 Continue"));
    client.write_all(b"hello").await.unwrap();
    let mut final_resp = Vec::new();
    client.read_to_end(&mut final_resp).await.unwrap();
    assert!(String::from_utf8_lossy(&final_resp).contains("200 OK"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_early_hints_103_records_final_response() {
    let dir = std::env::temp_dir().join(format!("zniff-103-hints-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = sock.read(&mut buf).await.unwrap();
        // Origin sends 103 Early Hints
        sock.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n")
            .await
            .unwrap();
        // Origin sends final 200 OK response
        let body = b"main page content";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/page HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(
        text.contains("103 Early Hints"),
        "client must receive 103 Early Hints"
    );
    assert!(text.contains("200 OK"), "client must receive 200 OK");
    assert!(
        text.contains("main page content"),
        "client must receive body"
    );

    let flow = hub.with_session(|session| session.get(1).cloned());
    let flow = flow.expect("flow");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.response_body, b"main page content");
    assert!(flow.elapsed_ms().is_some(), "flow must be marked finished");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn install_page_follows_accept_language() {
    assert!(super::install_page_for(None).contains("安装 zniff"));
    assert!(super::install_page_for(Some("")).contains("安装 zniff"));
    assert!(super::install_page_for(Some("zh-CN,en;q=0.8")).contains("安装 zniff"));
    assert!(super::install_page_for(Some("en-US,en;q=0.9")).contains("Install the zniff"));
    assert!(super::install_page_for(Some("fr")).contains("Install the zniff"));
}

#[test]
fn install_page_includes_android_network_security_config_guidance() {
    for (lang, expect_heading, expect_copy) in [
        (Some("zh-CN"), "Android 7+ 应用抓包配置", "复制 XML"),
        (Some("en-US"), "Android 7+ app capture setup", "Copy XML"),
    ] {
        let html = super::install_page_for(lang);
        assert!(
            html.contains(expect_heading),
            "missing Android heading for {lang:?}"
        );
        assert!(
            html.contains(expect_copy),
            "missing copy button for {lang:?}"
        );
        assert!(html.contains("res/xml/network_security_config.xml"));
        assert!(html.contains("&lt;network-security-config&gt;"));
        assert!(html.contains("&lt;debug-overrides&gt;"));
        assert!(html.contains("&lt;certificates src=&quot;user&quot; /&gt;"));
        assert!(
            html.contains("android:networkSecurityConfig=&quot;@xml/network_security_config&quot;")
        );
        assert!(html.contains("subject_hash_old"));
        assert!(html.contains("data-copy=\"nsc-xml\""));
    }
}

use super::*;

#[tokio::test]
async fn plain_http_is_recorded_and_forwarded() {
    let dir = std::env::temp_dir().join(format!("snoop-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = sock.read(&mut buf).await.unwrap();
        let body = b"hello-snoop";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("hello-snoop"), "{text}");
    let flow = hub.with_session(|session| session.get(1).cloned());
    let flow = flow.expect("flow");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.response_body, b"hello-snoop");
    assert_eq!(flow.protocol, Protocol::Http11);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn replace_rewrites_a_plain_response_body() {
    let dir = std::env::temp_dir().join(format!("snoop-replace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    hub.set_rules("replace 127.0.0.1 res hello -> hi there");
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = sock.read(&mut buf).await.unwrap();
        let body = b"say hello";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("say hi there"), "{text}");
    assert!(!text.contains("say hello"), "{text}");
    let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
    assert_eq!(flow.response_body, b"say hi there");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_follows_origin_alpn_and_records_the_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("snoop-mitm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let mut hub = Hub::new(ca.clone()).unwrap();

    let origin_ca_key = rcgen::KeyPair::generate().unwrap();
    let mut origin_ca_params = rcgen::CertificateParams::default();
    origin_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let mut origin_name = rcgen::DistinguishedName::new();
    origin_name.push(rcgen::DnType::CommonName, "origin-test");
    origin_ca_params.distinguished_name = origin_name;
    origin_ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let origin_ca = origin_ca_params.self_signed(&origin_ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&origin_ca_params, &origin_ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    leaf_params.is_ca = rcgen::IsCa::NoCa;
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = tls.read(&mut buf).await.unwrap();
        let body = b"mitm-ok";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tls.write_all(resp.as_bytes()).await.unwrap();
        tls.write_all(body).await.unwrap();
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        if let Err(err) = handle_conn(socket_from(proxy).await, hub_serve).await {
            log::debug!("测试连接: {err:#}");
        }
    });

    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(5), tcp.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(n, 0, "代理没有回 CONNECT 响应");
        header.push(byte[0]);
        assert!(header.len() < 1024, "CONNECT 响应过长");
    }
    let header_text = String::from_utf8_lossy(&header);
    assert!(header_text.starts_with("HTTP/1.1 200"), "{header_text}");

    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(client_config));
    let mut tls = timeout(
        Duration::from_secs(5),
        connector.connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS");
    assert_eq!(
        tls.get_ref().1.alpn_protocol(),
        Some(b"http/1.1".as_slice())
    );

    let req = format!(
        "GET /hi HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        origin_addr.ip()
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
        .await
        .expect("读响应超时")
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("mitm-ok"), "{text}");

    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.response_body, b"mitm-ok");
    assert_eq!(flow.protocol, Protocol::Http11);
    assert_eq!(flow.method, "GET");
    assert!(flow.url.contains("/hi"), "{}", flow.url);
    let _ = std::fs::remove_dir_all(&dir);
}

async fn socket_from(listener: TcpListener) -> TcpStream {
    listener.accept().await.unwrap().0
}

#[tokio::test]
async fn https_follows_h2_when_the_origin_selects_it() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("snoop-h2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server_config.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let tls = acceptor.accept(sock).await.unwrap();
        let service = hyper::service::service_fn(|_req: Request<Incoming>| async move {
            Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                Bytes::from_static(b"h2-ok"),
            )))
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        if let Err(err) = handle_conn(socket_from(proxy).await, hub_serve).await {
            log::debug!("测试连接: {err:#}");
        }
    });

    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    read_http_head(&mut tcp).await;

    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let tls = timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(client_config))
            .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS");
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));

    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
        .await
        .expect("客户端 HTTP/2");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let req = Request::builder()
        .uri("/hi")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("HTTP/2 响应超时")
        .expect("HTTP/2 响应");
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"h2-ok");

    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.protocol, Protocol::Http2);
    assert_eq!(flow.method, "GET");
    assert_eq!(flow.status, Some(200));
    assert_eq!(flow.response_body, b"h2-ok");
    let _ = std::fs::remove_dir_all(&dir);
}

fn test_origin_cert() -> (rcgen::Certificate, rcgen::Certificate, rcgen::KeyPair) {
    let origin_ca_key = rcgen::KeyPair::generate().unwrap();
    let mut origin_ca_params = rcgen::CertificateParams::default();
    origin_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let mut origin_name = rcgen::DistinguishedName::new();
    origin_name.push(rcgen::DnType::CommonName, "origin-test");
    origin_ca_params.distinguished_name = origin_name;
    origin_ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let origin_ca = origin_ca_params.self_signed(&origin_ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&origin_ca_params, &origin_ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let mut leaf_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    leaf_params.is_ca = rcgen::IsCa::NoCa;
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    (origin_ca, origin_leaf, leaf_key)
}

async fn read_http_head(tcp: &mut TcpStream) {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(5), tcp.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(n, 0, "代理没有回 CONNECT 响应");
        header.push(byte[0]);
        assert!(header.len() < 1024, "CONNECT 响应过长");
    }
    let header_text = String::from_utf8_lossy(&header);
    assert!(header_text.starts_with("HTTP/1.1 200"), "{header_text}");
}

#[tokio::test]
async fn websocket_handshake_is_recorded_and_frames_are_not() {
    let dir = std::env::temp_dir().join(format!("snoop-ws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(text.contains("Upgrade: websocket"), "{text}");
        let resp =
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        sock.write_all(resp.as_bytes()).await.unwrap();
        let mut frame = [0u8; 4];
        sock.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame, b"ping");
        sock.write_all(b"pong").await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(5), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(n, 0);
        header.push(byte[0]);
    }
    let header_text = String::from_utf8_lossy(&header);
    assert!(header_text.contains("101"), "{header_text}");
    client.write_all(b"ping").await.unwrap();
    let mut pong = [0u8; 4];
    timeout(Duration::from_secs(5), client.read_exact(&mut pong))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&pong, b"pong");
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.protocol, Protocol::WebSocket);
    assert_eq!(flow.status, Some(101));
    assert!(flow.response_body.is_empty(), "{:?}", flow.response_body);
    assert!(flow.request_body.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plaintext_keep_alive_reuses_the_origin_socket() {
    let dir = std::env::temp_dir().join(format!("zniff-pool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let accepts_bg = accepts.clone();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        accepts_bg.fetch_add(1, Ordering::SeqCst);
        for mark in [b"one", b"two"] {
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                let n = timeout(Duration::from_secs(5), sock.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap();
                assert_ne!(n, 0);
                buf.push(byte[0]);
            }
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", mark.len());
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(mark).await.unwrap();
        }
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = proxy.accept().await else {
                break;
            };
            let hub = hub_serve.clone();
            tokio::spawn(async move {
                let _ = handle_conn(socket, hub).await;
            });
        }
    });
    for mark in ["one", "two"] {
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!("GET http://{origin_addr}/x HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        client.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains(mark), "{text}");
    }
    assert_eq!(accepts.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn upstream_proxy_receives_absolute_form() {
    let dir = std::env::temp_dir().join(format!("zniff-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = upstream.accept().await.unwrap();
        let mut buf = vec![0u8; 2048];
        let n = timeout(Duration::from_secs(5), sock.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(text.starts_with("GET http://example.test/hi "), "{text}");
        let body = b"via-upstream";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    hub.set_upstream(&format!("127.0.0.1:{}", upstream_addr.port()));
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req =
        "GET http://example.test/hi HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n";
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&resp).contains("via-upstream"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bypass_tunnels_tls_without_a_leaf() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-bypass-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    hub.set_bypass("127.0.0.1");
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        let mut buf = vec![0u8; 1024];
        let _ = tls.read(&mut buf).await.unwrap();
        let body = b"tunnel-ok";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tls.write_all(resp.as_bytes()).await.unwrap();
        tls.write_all(body).await.unwrap();
        tls.shutdown().await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    read_http_head(&mut tcp).await;
    let mut roots = RootCertStore::empty();
    roots.add(origin_ca.der().clone()).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    let mut tls = timeout(
        Duration::from_secs(5),
        connector.connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("直通后应使用源站证书");
    tls.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    let read = timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("tunnel-ok"), "{read:?} {text}");
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.protocol, Protocol::Tunnel);
    let reason = flow.error.unwrap_or_default();
    assert!(reason.contains("直通"), "{reason}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn websocket_text_frame_is_recorded_beside_the_handshake() {
    let dir = std::env::temp_dir().join(format!("zniff-ws-frame-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let _ = sock.read(&mut buf).await.unwrap();
        sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
        sock.write_all(&[0x81, 0x02, b'h', b'i']).await.unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(5), client.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(n, 0);
        header.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&header).contains("101"));
    let mut frame = [0u8; 4];
    timeout(Duration::from_secs(5), client.read_exact(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&frame, &[0x81, 0x02, b'h', b'i']);
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.protocol, Protocol::WebSocket);
    assert_eq!(flow.messages.len(), 1);
    assert_eq!(flow.messages[0].kind, "文本");
    assert_eq!(flow.messages[0].text, "hi");
    assert!(flow.response_body.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wss_websocket_echoes_and_records_frames() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-wss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        let mut req_head = Vec::new();
        let mut byte = [0u8; 1];
        while !req_head.ends_with(b"\r\n\r\n") {
            let n = tls.read(&mut byte).await.unwrap();
            assert_ne!(n, 0);
            req_head.push(byte[0]);
        }
        tls.write_all(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
            )
            .await
            .unwrap();
        // Masked client frame: "ok" with mask [1, 2, 3, 4]
        let mut client_frame = [0u8; 8];
        tls.read_exact(&mut client_frame).await.unwrap();
        let mask = [
            client_frame[2],
            client_frame[3],
            client_frame[4],
            client_frame[5],
        ];
        let payload = [client_frame[6] ^ mask[0], client_frame[7] ^ mask[1]];
        assert_eq!(&payload, b"ok");
        // Unmasked server echo frame: "ok"
        tls.write_all(&[0x81, 0x02, payload[0], payload[1]])
            .await
            .unwrap();
        let _ = tls.shutdown().await;
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    read_http_head(&mut tcp).await;

    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(client_config))
            .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS");

    let req = format!(
        "GET /ws HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        let n = timeout(Duration::from_secs(5), tls.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(n, 0);
        header.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&header);
    assert!(head.contains("101"), "{head}");

    // Send masked text frame "ok" with mask [1, 2, 3, 4]
    let masked = [0x81, 0x82, 1, 2, 3, 4, b'o' ^ 1, b'k' ^ 2];
    tls.write_all(&masked).await.unwrap();

    let mut frame = [0u8; 4];
    timeout(Duration::from_secs(5), tls.read_exact(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&frame, &[0x81, 0x02, b'o', b'k']);
    drop(tls);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let done = hub.with_session(|session| {
            session
                .get(1)
                .is_some_and(|flow| flow.messages.len() == 2 && flow.finished_at.is_some())
        });
        if done || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.protocol, Protocol::WebSocket);
    assert_eq!(flow.messages.len(), 2);
    assert_eq!(flow.messages[0].text, "ok");
    assert_eq!(flow.messages[1].text, "ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn map_local_answers_without_dialing() {
    let dir = std::env::temp_dir().join(format!("zniff-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let file = dir.join("body.txt");
    std::fs::write(&file, b"from-file").unwrap();
    hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET http://127.0.0.1:9/hi HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("200"), "{text}");
    assert!(text.contains("from-file"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn map_local_status_and_cors_preflight() {
    let dir = std::env::temp_dir().join(format!("zniff-mock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let file = dir.join("body.txt");
    std::fs::write(&file, b"nope").unwrap();
    hub.set_rules(&format!(
            "map-local 127.0.0.1/hi {} status=429\ncors 127.0.0.1\nheader 127.0.0.1 res set X-Zniff yes\n",
            file.display()
        ));
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET http://127.0.0.1:9/hi HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("429"), "{text}");
    assert!(text.to_ascii_lowercase().contains("x-zniff: yes"), "{text}");
    assert!(
        text.to_ascii_lowercase()
            .contains("access-control-allow-origin: *"),
        "{text}"
    );

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"OPTIONS http://127.0.0.1:9/other HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("204"), "{text}");
    assert!(
        text.to_ascii_lowercase()
            .contains("access-control-allow-origin: *"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn response_break_can_replace_status_and_body() {
    let dir = std::env::temp_dir().join(format!("zniff-break-res-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 2048];
        let _ = sock.read(&mut buf).await.unwrap();
        let body = b"orig";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
    });
    hub.set_rules("break-res 127.0.0.1/hi");
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let hub_wait = hub.clone();
    let waiter = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait.paused_id() == Some(1) {
                assert_eq!(
                    hub_wait.paused().map(|(_, side)| side),
                    Some(BreakSide::Response)
                );
                hub_wait.resolve_break(BreakDecision {
                    drop: false,
                    headers: None,
                    body: Some(b"edited".to_vec()),
                    status: Some(201),
                });
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("响应断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/hi HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    waiter.await.unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("201"), "{text}");
    assert!(text.contains("edited"), "{text}");
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.status, Some(201));
    assert_eq!(flow.response_body, b"edited");
    assert!(flow.finished_at.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_response_break_can_replace_status_and_marks_finished() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-https-break-res-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        let mut buf = vec![0u8; 1024];
        let _ = tls.read(&mut buf).await.unwrap();
        tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\norig")
            .await
            .unwrap();
        let _ = tls.shutdown().await;
    });

    hub.set_rules("break-res 127.0.0.1/hi");
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let hub_wait = hub.clone();
    let waiter = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait.paused_id() == Some(1) {
                hub_wait.touch_break();
                assert_eq!(
                    hub_wait.paused().map(|(_, side)| side),
                    Some(BreakSide::Response)
                );
                hub_wait.resolve_break(BreakDecision {
                    drop: false,
                    headers: None,
                    body: None,
                    status: Some(201),
                });
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("HTTPS 响应断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    read_http_head(&mut tcp).await;

    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(client_config))
            .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS");

    let req = format!(
        "GET /hi HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        origin_addr.ip()
    );
    tls.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), tls.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    waiter.await.unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("201"), "{text}");
    assert!(text.contains("orig"), "{text}");
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.status, Some(201));
    assert_eq!(flow.response_body, b"orig");
    assert!(flow.finished_at.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn breakpoint_drop_closes_client_connection_and_marks_failed() {
    let dir = std::env::temp_dir().join(format!("zniff-break-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = origin.accept().await {
            let _ = read_header_block(&mut sock).await;
            let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
                    )
                    .await;
        }
    });
    hub.set_rules("break 127.0.0.1/hold\nbreak-res 127.0.0.1/hold-res");
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // 1. Request breakpoint drop
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET http://127.0.0.1:9/hold HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n")
        .await
        .unwrap();
    let hub_wait = hub.clone();
    let waiter = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait.paused_id() == Some(1) {
                hub_wait.resolve_break(BreakDecision {
                    drop: true,
                    headers: None,
                    body: None,
                    status: None,
                });
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap();
    waiter.await.unwrap();
    assert!(
        resp.is_empty(),
        "dropped request breakpoint must not write HTTP 204: {:?}",
        String::from_utf8_lossy(&resp)
    );

    // 2. Response breakpoint drop (origin returned 200 OK, drop clears status -> None)
    let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
    client2
            .write_all(
                format!(
                    "GET http://{origin_addr}/hold-res HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    let hub_wait2 = hub.clone();
    let waiter2 = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait2.paused_id() == Some(2) {
                hub_wait2.resolve_break(BreakDecision {
                    drop: true,
                    headers: None,
                    body: None,
                    status: None,
                });
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("响应断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut resp2 = Vec::new();
    let _ = timeout(Duration::from_secs(5), client2.read_to_end(&mut resp2))
        .await
        .unwrap();
    waiter2.await.unwrap();
    assert!(
        resp2.is_empty(),
        "dropped response breakpoint must not write HTTP 204: {:?}",
        String::from_utf8_lossy(&resp2)
    );

    hub.with_session(|session| {
        for fid in [1u64, 2u64] {
            let flow = session.get(fid).expect("flow");
            assert_eq!(flow.status, None);
            assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
            assert!(flow.finished_at.is_some());
        }
        assert_eq!(session.ids_matching("失败"), vec![1, 2]);
        let stats = session.filtered_stats("");
        assert_eq!(stats.s2xx, 0);
        assert_eq!(stats.failed, 2);
        let facets = session.facets();
        assert_eq!(facets.domains[0].error_count, 2);
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_routes_leave_proxied_roots_alone() {
    assert!(matches!(
        local_route("GET", "/zniff"),
        Some(LocalRoute::Install)
    ));
    assert!(matches!(local_route("GET", "/"), Some(LocalRoute::Install)));
    assert!(matches!(
        local_route("GET", "http://192.168.1.8:8877/zniff"),
        Some(LocalRoute::Install)
    ));
    assert!(local_route("GET", "http://example.com/").is_none());
    assert!(local_route("GET", "http://example.com/index").is_none());
    let id = "550e8400-e29b-41d4-a716-446655440000";
    assert!(matches!(
        local_route("GET", &format!("/s/{id}")),
        Some(LocalRoute::Share(_, crate::share::SharePath::Page))
    ));
    assert!(local_route("GET", &format!("http://example.com/s/{id}")).is_none());
    assert!(matches!(
        local_route("GET", "http://127.0.0.1/zniff-ca.crt"),
        Some(LocalRoute::Cert)
    ));
    assert!(matches!(
        local_route("GET", "/favicon.ico"),
        Some(LocalRoute::Favicon)
    ));
    assert!(local_route("GET", "http://example.com/favicon.ico").is_none());
    assert!(matches!(
        local_route_for("GET", "http://127.0.0.1:8877/favicon.ico", true),
        Some(LocalRoute::Favicon)
    ));
    assert!(matches!(
        local_route_for("GET", &format!("http://127.0.0.1:8877/s/{id}"), true),
        Some(LocalRoute::Share(_, crate::share::SharePath::Page))
    ));
    assert!(local_route("POST", "/zniff").is_none());
    assert!(local_route("CONNECT", "example.com:443").is_none());
    assert!(local_route("PUT", &format!("http://example.com/s/{id}/flows.json")).is_none());
    assert!(matches!(
        local_route("PUT", &format!("/s/{id}/flows.json")),
        Some(LocalRoute::Relay(
            _,
            crate::share::SharePath::Data { .. },
            RelayVerb::Put
        ))
    ));
    assert!(matches!(
        local_route("DELETE", &format!("/s/{id}")),
        Some(LocalRoute::Relay(
            _,
            crate::share::SharePath::Page,
            RelayVerb::Delete
        ))
    ));
    assert!(
        local_route_for(
            "PUT",
            &format!("http://example.com/s/{id}/flows.json"),
            false
        )
        .is_none()
    );
}

#[tokio::test]
async fn favicon_and_self_targeted_requests_do_not_loop_or_record() {
    let dir = std::env::temp_dir().join(format!("zniff-self-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req =
        format!("GET /favicon.ico HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n");
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("HTTP/1.1 204"), "{text}");

    let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
    let req2 = format!(
        "GET http://{proxy_addr}/unknown-local-path HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n"
    );
    client2.write_all(req2.as_bytes()).await.unwrap();
    let mut resp2 = Vec::new();
    timeout(Duration::from_secs(5), client2.read_to_end(&mut resp2))
        .await
        .unwrap()
        .unwrap();
    let text2 = String::from_utf8_lossy(&resp2);
    assert!(text2.starts_with("HTTP/1.1 404"), "{text2}");

    assert_eq!(hub.with_session(|session| session.flows().len()), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn relay_accept_puts_shares_in_the_session() {
    let dir = std::env::temp_dir().join(format!("zniff-relay-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let id = "550e8400-e29b-41d4-a716-446655440000";
    let other = "550e8400-e29b-41d4-a716-446655440001";
    let missing = "550e8400-e29b-41d4-a716-446655440002";
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let alpha = inbound_doc("alpha.test", 7, "ready", false, expires);
    let beta = inbound_doc("beta.test", 8, "beta", false, expires);

    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(hub.with_session(|session| session.flows().len()), 0);

    hub.set_relay_ingress(true, "a b");
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "a b", &alpha),
    )
    .await;
    assert_eq!(status, 401);
    hub.set_relay_token("secret");
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "nope", &alpha),
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(hub.with_session(|session| session.flows().len()), 0);

    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
    )
    .await;
    assert_eq!(status, 204);
    let (local_id, host, label) = hub.with_session(|session| {
        let flow = session.flows().into_iter().next().unwrap();
        (flow.id, flow.host.clone(), flow.client_ip.clone())
    });
    assert_eq!(host, "alpha.test");
    assert_eq!(label, "中继 550e8400");
    let flow = hub
        .with_session(|session| session.get(local_id).cloned())
        .unwrap();
    assert_eq!(flow.relay_source.as_deref(), Some(id));
    assert_eq!(flow.relay_remote_id, Some(7));
    assert_eq!(flow.response_body, b"ready");
    assert!(flow.comment.is_empty());
    assert_eq!(flow.messages.len(), 1);
    assert!(hub.with_session(|session| session.flows_matching("550e8400").len()) == 1);

    let (status, page) = relay_exchange(
        addr,
        &format!("GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(page.contains("alpha.test"), "{page}");
    let (status, flows) = relay_exchange(
        addr,
        &format!(
            "GET /s/{id}/flows.json?since=1 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(status, 200);
    assert!(flows.contains("\"incremental\":false"), "{flows}");
    assert!(!flows.contains("\"incremental\":true"), "{flows}");

    let packed = raw_exchange(
            addr,
            &format!(
                "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    let packed_text = String::from_utf8_lossy(&packed);
    assert!(
        packed_text.contains("Content-Encoding: gzip"),
        "{packed_text}"
    );
    assert!(
        packed.windows(2).any(|pair| pair == [0x1f, 0x8b]),
        "gzip magic missing"
    );

    let (status, _) = relay_exchange(
            addr,
            &format!(
                "PUT http://{addr}/s/{other}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{beta}",
                beta.len()
            ),
        )
        .await;
    assert_eq!(status, 204);
    let hosts = hub.with_session(|session| {
        session
            .flows()
            .into_iter()
            .map(|flow| flow.host)
            .collect::<Vec<_>>()
    });
    assert!(hosts.contains(&"alpha.test".to_string()), "{hosts:?}");
    assert!(hosts.contains(&"beta.test".to_string()), "{hosts:?}");

    assert!(hub.with_session(|session| session.set_pinned(local_id, true)));
    let gamma = inbound_doc("gamma.test", 7, "gamma", false, expires);
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "secret", &gamma),
    )
    .await;
    assert_eq!(status, 204);
    let updated = hub
        .with_session(|session| session.get(local_id).cloned())
        .unwrap();
    assert_eq!(updated.host, "gamma.test");
    assert!(updated.pinned);
    assert!(
        hub.with_session(|session| { session.flows().iter().any(|flow| flow.host == "beta.test") })
    );

    let (tx, rx) = std::sync::mpsc::channel();
    hub.replay_later(local_id, ReplayOverride::default(), move |result| {
        let _ = tx.send(result.err().map(|err| err.to_string()).unwrap_or_default());
    });
    let err = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(err.contains("中继"), "{err}");
    let (tx, rx) = std::sync::mpsc::channel();
    hub.replay_batch_later(
        local_id,
        2,
        1,
        0,
        ReplayOverride::default(),
        move |result| {
            let _ = tx.send(result.err().map(|err| err.to_string()).unwrap_or_default());
        },
    );
    let err = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(err.contains("中继"), "{err}");

    let spilled = inbound_doc("gamma.test", 7, "hello", true, expires);
    relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "secret", &spilled),
    )
    .await;
    assert!(hub.with_session(|session| session.get(local_id).unwrap().response_body.is_empty()));
    let flow_body =
        r#"{"id":7,"request_body":"ping","response_body":"hello","messages":["← text pong"]}"#;
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{missing}/flows/7.json"), "secret", flow_body),
    )
    .await;
    assert_eq!(status, 409);
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows/7.json"), "secret", flow_body),
    )
    .await;
    assert_eq!(status, 204);
    let filled = hub
        .with_session(|session| session.get(local_id).cloned())
        .unwrap();
    assert_eq!(filled.response_body, b"hello");
    assert_eq!(filled.id, local_id);
    let (status, body_page) = relay_exchange(
        addr,
        &format!("GET /s/{id}/flows/7.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(body_page.contains("hello"), "{body_page}");

    let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{other} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer nope\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 401);
    assert!(
        hub.with_session(|session| session.flows().iter().any(|flow| flow.host == "beta.test"))
    );
    let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{other} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 204);
    assert!(
        hub.with_session(|session| session.flows().iter().all(|flow| flow.host != "beta.test"))
    );
    assert!(hub.with_session(|session| session.get(local_id).is_some()));

    let local = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "local.test".into(),
        path: "/mine".into(),
        url: "http://local.test/mine".into(),
        request_headers: Vec::new(),
    });
    let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 204);
    assert!(hub.with_session(|session| session.get(local_id).is_none()));
    assert!(hub.with_session(|session| session.get(local).is_some()));
    let (status, _) = relay_exchange(
            addr,
            &format!(
                "DELETE /s/{missing} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 204);

    let (status, text) = relay_exchange(
            addr,
            &format!(
                "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 411, "{text}");
    let (status, text) = relay_exchange(
            addr,
            &format!(
                "PUT /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer secret\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
    assert_eq!(status, 413, "{text}");

    hub.set_relay_accept(false);
    let (status, _) = relay_exchange(
        addr,
        &relay_put(&format!("/s/{id}/flows.json"), "secret", &alpha),
    )
    .await;
    assert_eq!(status, 404);
    assert!(hub.with_session(|session| session.get(local).is_some()));
    assert_eq!(hub.with_session(|session| session.flows().len()), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn inbound_doc(host: &str, remote_id: u64, body: &str, spilled: bool, expires: u64) -> String {
    let flag = if spilled { "true" } else { "false" };
    let response = if spilled { "" } else { body };
    format!(
        r#"{{"expires":{expires},"filter":"host:{host}","omitted":0,"rev":3,"incremental":true,"ids":[{remote_id}],"flows":[{{"id":{remote_id},"method":"POST","status":201,"protocol":"HTTP/1.1","host":"{host}","path":"/inbox","url":"http://{host}/inbox","request_headers":[["Accept","text/plain"]],"response_headers":[["Content-Type","text/plain"]],"request_body":"ping","response_body":"{response}","request_body_spilled":false,"response_body_spilled":{flag},"request_bytes":4,"response_bytes":5,"ttfb_ms":8,"total_ms":20,"comment":"","rule_hit":"","messages":["→ text hi"]}}]}}"#
    )
}

fn relay_put(path: &str, token: &str, body: &str) -> String {
    format!(
        "PUT {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn relay_exchange(addr: SocketAddr, request: &str) -> (u16, String) {
    let bytes = raw_exchange(addr, request).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (status, text)
}

async fn raw_exchange(addr: SocketAddr, request: &str) -> Vec<u8> {
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    resp
}

#[test]
fn share_document_keeps_only_the_frozen_filter() {
    let dir = std::env::temp_dir().join(format!("zniff-share-filter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Hub::new(ca).unwrap();
    hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "a.test".into(),
        path: "/secret".into(),
        url: "http://a.test/secret".into(),
        request_headers: vec![("Cookie".into(), "session=1".into())],
    });
    hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "b.test".into(),
        path: "/other".into(),
        url: "http://b.test/other".into(),
        request_headers: vec![("Accept".into(), "text/plain".into())],
    });
    hub.begin_share(Duration::from_secs(60), "header:cookie");
    let id = hub.share_id().expect("share id");
    let json = hub.share_document(&id).expect("document");
    assert!(json.contains("http://a.test/secret"), "{json}");
    assert!(!json.contains("http://b.test/other"), "{json}");
    assert!(json.contains("header:cookie"), "{json}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gzip_is_offered_only_when_the_coding_is_accepted() {
    assert!(!accepts_gzip(None));
    assert!(!accepts_gzip(Some("")));
    assert!(!accepts_gzip(Some("identity")));
    assert!(!accepts_gzip(Some("*")));
    assert!(!accepts_gzip(Some("gzip;q=0")));
    assert!(!accepts_gzip(Some("deflate, gzip;q=0.0")));
    assert!(!accepts_gzip(Some("gzip;q=nope")));
    assert!(accepts_gzip(Some("gzip")));
    assert!(accepts_gzip(Some("GZIP")));
    assert!(accepts_gzip(Some("deflate, gzip")));
    assert!(accepts_gzip(Some("br, gzip;q=0.5")));
    let packed = gzip_bytes(b"{\"ok\":true}").expect("gzip");
    assert!(packed.starts_with(&[0x1f, 0x8b]));
    let mut decoder = flate2::read::GzDecoder::new(packed.as_slice());
    let mut plain = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut plain).unwrap();
    assert_eq!(plain, b"{\"ok\":true}");
}

#[tokio::test]
async fn share_flows_json_is_gzipped_only_when_the_client_accepts_it() {
    let dir = std::env::temp_dir().join(format!("zniff-share-gzip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let flow_id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "share.example".into(),
        path: "/items".into(),
        url: "http://share.example/items".into(),
        request_headers: vec![("Accept".into(), "application/json".into())],
    });
    hub.set_response(
        flow_id,
        200,
        vec![("Content-Type".into(), "application/json".into())],
        None,
    );
    hub.append_body(flow_id, Direction::Response, b"{\"n\":1}");
    hub.mark_finished(flow_id);
    hub.begin_share(Duration::from_secs(60), "");
    let id = hub.share_id().expect("share id");
    let before = hub.with_session(|session| session.flows().len());

    let proxy = listen().await.unwrap();
    let addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let plain_req =
        format!("GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (plain_head, plain_body) = read_closed_http(addr, &plain_req).await;
    assert!(plain_head.starts_with("HTTP/1.1 200"), "{plain_head}");
    assert!(
        header_field(&plain_head, "content-encoding").is_none(),
        "{plain_head}"
    );
    assert_eq!(
        header_field(&plain_head, "content-length"),
        Some(plain_body.len().to_string()).as_deref()
    );
    assert_eq!(header_field(&plain_head, "cache-control"), Some("no-store"));
    assert!(
        plain_head
            .to_ascii_lowercase()
            .contains("connection: close")
    );
    assert!(
        plain_body
            .windows(b"share.example".len())
            .any(|w| w == b"share.example")
    );

    let gzip_req = format!(
        "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: deflate, gzip\r\nConnection: close\r\n\r\n"
    );
    let (gzip_head, gzip_body) = read_closed_http(addr, &gzip_req).await;
    assert!(gzip_head.starts_with("HTTP/1.1 200"), "{gzip_head}");
    assert_eq!(header_field(&gzip_head, "content-encoding"), Some("gzip"));
    assert_eq!(
        header_field(&gzip_head, "content-length"),
        Some(gzip_body.len().to_string()).as_deref(),
        "{gzip_head}"
    );
    assert_eq!(
        header_field(&gzip_head, "content-type"),
        Some("application/json; charset=utf-8")
    );
    assert!(gzip_body.starts_with(&[0x1f, 0x8b]), "{gzip_body:?}");
    assert_ne!(gzip_body, plain_body);
    let mut decoder = flate2::read::GzDecoder::new(gzip_body.as_slice());
    let mut inflated = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut inflated).unwrap();
    assert_eq!(inflated, plain_body);

    let refused = format!(
        "GET /s/{id}/flows.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip;q=0, identity\r\nConnection: close\r\n\r\n"
    );
    let (refused_head, refused_body) = read_closed_http(addr, &refused).await;
    assert!(
        header_field(&refused_head, "content-encoding").is_none(),
        "{refused_head}"
    );
    assert_eq!(refused_body, plain_body);

    let page_req = format!(
        "GET /s/{id} HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
    );
    let (page_head, page_body) = read_closed_http(addr, &page_req).await;
    assert!(page_head.starts_with("HTTP/1.1 200"), "{page_head}");
    assert!(
        header_field(&page_head, "content-encoding").is_none(),
        "{page_head}"
    );
    assert!(page_body.starts_with(b"<!DOCTYPE html") || page_body.starts_with(b"<html"));

    let one_req = format!(
        "GET /s/{id}/flows/{flow_id}.json HTTP/1.1\r\nHost: {addr}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
    );
    let (one_head, one_body) = read_closed_http(addr, &one_req).await;
    assert!(one_head.starts_with("HTTP/1.1 200"), "{one_head}");
    assert!(
        header_field(&one_head, "content-encoding").is_none(),
        "{one_head}"
    );
    let one_text = String::from_utf8_lossy(&one_body);
    assert!(one_text.contains("response_body"), "{one_text}");
    assert!(one_text.contains('n'), "{one_text}");

    assert_eq!(hub.with_session(|session| session.flows().len()), before);
    let _ = std::fs::remove_dir_all(&dir);
}

async fn read_closed_http(addr: std::net::SocketAddr, request: &str) -> (String, Vec<u8>) {
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut raw))
        .await
        .unwrap()
        .unwrap();
    let split = raw
        .windows(4)
        .position(|mark| mark == b"\r\n\r\n")
        .expect("http header");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body = raw[split + 4..].to_vec();
    (head, body)
}

fn header_field<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

#[test]
fn share_document_supports_incremental_since_and_lazy_spilled_body() {
    let dir = std::env::temp_dir().join(format!("zniff-share-inc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Hub::new(ca).unwrap();
    let f1 = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "a.test".into(),
        path: "/big".into(),
        url: "http://a.test/big".into(),
        request_headers: Vec::new(),
    });
    hub.set_response(
        f1,
        200,
        vec![("Content-Type".into(), "text/plain".into())],
        None,
    );
    let big_payload = "x".repeat(20 * 1024);
    hub.append_body(f1, Direction::Response, big_payload.as_bytes());
    hub.mark_finished(f1);

    let f2 = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "b.test".into(),
        path: "/hidden".into(),
        url: "http://b.test/hidden".into(),
        request_headers: Vec::new(),
    });
    hub.set_response(f2, 200, Vec::new(), None);
    hub.append_body(f2, Direction::Response, b"secret-b");
    hub.mark_finished(f2);

    hub.begin_share(Duration::from_secs(60), "host:a.test");
    let share_id = hub.share_id().expect("share id");

    // Initial snapshot: spilled body (>16KB) is NOT embedded in flows.json
    let snap_str = hub
        .share_document_since(&share_id, None)
        .expect("initial snapshot");
    let snap: serde_json::Value = serde_json::from_str(&snap_str).unwrap();
    let rev1 = snap["rev"].as_u64().unwrap();
    assert!(rev1 > 0);
    assert_eq!(snap["incremental"], false);
    assert_eq!(snap["ids"], serde_json::json!([f1]));
    assert_eq!(snap["flows"].as_array().unwrap().len(), 1);
    assert_eq!(snap["flows"][0]["response_body_spilled"], true);
    assert_eq!(snap["flows"][0]["response_body"], "");

    // Polling with since=rev1 when nothing changed returns empty flows
    let unchanged_str = hub
        .share_document_since(&share_id, Some(rev1))
        .expect("unchanged poll");
    let unchanged: serde_json::Value = serde_json::from_str(&unchanged_str).unwrap();
    assert_eq!(unchanged["incremental"], true);
    assert_eq!(unchanged["rev"], rev1);
    assert_eq!(unchanged["ids"], serde_json::json!([f1]));
    assert!(unchanged["flows"].as_array().unwrap().is_empty());

    // Clicking flow f1 fetches its spilled body on demand; f2 (filtered out) is rejected
    let body_str = hub
        .share_flow_body(&share_id, f1)
        .expect("lazy body for matching flow");
    let body_doc: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    assert_eq!(body_doc["id"], f1);
    assert!(
        body_doc["response_body"]
            .as_str()
            .unwrap()
            .starts_with("xxxx")
    );
    assert!(hub.share_flow_body(&share_id, f2).is_none());

    // Adding a new matching flow f3: incremental poll since=rev1 returns ONLY f3
    let f3 = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "POST".into(),
        host: "a.test".into(),
        path: "/small".into(),
        url: "http://a.test/small".into(),
        request_headers: Vec::new(),
    });
    let delta_str = hub
        .share_document_since(&share_id, Some(rev1))
        .expect("delta poll");
    let delta: serde_json::Value = serde_json::from_str(&delta_str).unwrap();
    assert_eq!(delta["incremental"], true);
    assert!(delta["rev"].as_u64().unwrap() > rev1);
    assert_eq!(delta["ids"], serde_json::json!([f1, f3]));
    assert_eq!(delta["flows"].as_array().unwrap().len(), 1);
    assert_eq!(delta["flows"][0]["id"], f3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn install_page_links_to_the_certificate() {
    let dir = std::env::temp_dir().join(format!("zniff-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET /zniff HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("200"), "{text}");
    assert!(text.contains("text/html"), "{text}");
    assert!(text.contains("href=\"/zniff-ca.crt\""), "{text}");
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.path, "/zniff");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ca_cert_route_returns_the_pem() {
    let dir = std::env::temp_dir().join(format!("zniff-ca-route-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let pem = std::fs::read(ca.cert_path()).unwrap();
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
        .write_all(b"GET http://127.0.0.1/zniff-ca.crt HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut resp = Vec::new();
    timeout(Duration::from_secs(5), client.read_to_end(&mut resp))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("200"), "{text}");
    assert!(text.contains("application/x-x509-ca-cert"), "{text}");
    assert!(
        resp.windows(pem.len())
            .any(|window| window == pem.as_slice())
    );
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.path, "/zniff-ca.crt");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn splits_connect_authority() {
    assert_eq!(
        split_host_port("example.com:443", 80).unwrap(),
        ("example.com".into(), 443)
    );
    assert_eq!(
        split_host_port("[::1]:8443", 443).unwrap(),
        ("::1".into(), 8443)
    );
}

#[test]
fn strip_ws_extension_drops_only_that_header() {
    let raw = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nConnection: Upgrade\r\n\r\n";
    let text = String::from_utf8(strip_ws_extension(raw)).unwrap();
    assert!(
        !text
            .to_ascii_lowercase()
            .contains("sec-websocket-extensions")
    );
    assert!(text.contains("Upgrade: websocket"));
    assert!(text.contains("Connection: Upgrade"));
    assert!(text.ends_with("\r\n\r\n"));
}

#[test]
fn rule_switches_skip_matching_without_rewriting_the_text() {
    let dir = std::env::temp_dir().join(format!("zniff-mute-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Hub::new(ca).unwrap();
    hub.set_rules("delay example.com 1s\nbreak example.com");
    assert!(matches!(
        hub.plan("example.com", "/").action,
        Some(Action::BreakRequest)
    ));
    hub.set_breaks_enabled(false);
    let kept = hub.plan("example.com", "/");
    assert!(kept.action.is_none());
    assert!(!kept.delay.is_zero());
    hub.set_rules_enabled(false);
    assert_eq!(hub.plan("example.com", "/"), rules::Effect::default());
    assert_eq!(
        hub.rules
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len(),
        2
    );
    let id = hub.with_session(|session| {
        session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "example.com".into(),
            path: "/".into(),
            url: "http://example.com/".into(),
            request_headers: Vec::new(),
        })
    });
    hub.note_rules(id, "example.com", "/");
    assert!(hub.with_session(|session| session.get(id).unwrap().rule_hit.is_empty()));
    hub.set_rules_enabled(true);
    hub.note_rules(id, "example.com", "/");
    assert_eq!(
        hub.with_session(|session| session.get(id).unwrap().rule_hit.clone()),
        "delay"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_har_restores_timing_after_mark_finished() {
    let dir = std::env::temp_dir().join(format!("zniff-har-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Hub::new(ca).unwrap();
    let har = serde_json::json!({
        "log": {
            "entries": [{
                "startedDateTime": "2023-11-14T22:13:20.000Z",
                "time": 40,
                "request": {
                    "method": "POST",
                    "url": "http://example.com/v1",
                    "httpVersion": "HTTP/1.1",
                    "headers": [],
                    "bodySize": 18,
                    "postData": {"mimeType": "text/plain", "text": "hi"}
                },
                "response": {
                    "status": 200,
                    "httpVersion": "HTTP/1.1",
                    "headers": [],
                    "bodySize": 4,
                    "content": {"mimeType": "text/plain", "text": "ok"}
                },
                "timings": {"wait": 15, "receive": 25}
            }]
        }
    });
    assert_eq!(hub.import_har(&har), 1);
    let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
    assert_eq!(flow.request_body, b"hi");
    assert_eq!(flow.response_body, b"ok");
    assert_eq!(flow.request_bytes, 18);
    assert_eq!(flow.response_bytes, 4);
    let wait = flow
        .response_at
        .unwrap()
        .duration_since(flow.started)
        .unwrap();
    let total = flow
        .finished_at
        .unwrap()
        .duration_since(flow.started)
        .unwrap();
    assert_eq!(wait, Duration::from_millis(15));
    assert_eq!(total, Duration::from_millis(40));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn websocket_extension_is_not_forwarded() {
    let dir = std::env::temp_dir().join(format!("zniff-ws-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let req = read_h1_message(&mut sock).await;
        *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nConnection: Upgrade\r\n\r\n")
                .await
                .unwrap();
        sock.write_all(&[0x81, 0x05, b'p', b'l', b'a', b'i', b'n'])
            .await
            .unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/chat HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Extensions: permessage-deflate\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let header = read_h1_message(&mut client).await;
    let header_text = String::from_utf8_lossy(&header);
    assert!(header_text.contains("101"), "{header_text}");
    assert!(
        !header_text
            .to_ascii_lowercase()
            .contains("sec-websocket-extensions"),
        "{header_text}"
    );
    let mut frame = [0u8; 7];
    timeout(Duration::from_secs(5), client.read_exact(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&frame, &[0x81, 0x05, b'p', b'l', b'a', b'i', b'n']);
    let origin_text = seen.lock().unwrap().clone();
    assert!(
        !origin_text
            .to_ascii_lowercase()
            .contains("sec-websocket-extensions"),
        "{origin_text}"
    );
    assert!(
        origin_text
            .to_ascii_lowercase()
            .contains("upgrade: websocket"),
        "{origin_text}"
    );
    let flow = hub
        .with_session(|session| session.get(1).cloned())
        .expect("flow");
    assert_eq!(flow.messages.len(), 1);
    assert_eq!(flow.messages[0].text, "plain");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn replay_override_recalculates_content_length_and_skips_rules() {
    let dir = std::env::temp_dir().join(format!("zniff-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let file = dir.join("mapped.txt");
    std::fs::write(&file, b"from-rule").unwrap();
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    hub.set_rules(&format!("map-local 127.0.0.1/new {}", file.display()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let req = read_h1_message(&mut sock).await;
        *seen_task.lock().unwrap() = req;
        sock.write_all(
            b"HTTP/1.1 201 Created\r\nContent-Length: 4\r\nConnection: close\r\n\r\nokay",
        )
        .await
        .unwrap();
    });
    let id = hub.with_session(|session| {
        session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "127.0.0.1".into(),
            path: "/old".into(),
            url: format!("http://{origin_addr}/old"),
            request_headers: vec![
                ("Host".into(), origin_addr.to_string()),
                ("Content-Length".into(), "1".into()),
                ("X-Old".into(), "keep".into()),
            ],
        })
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    hub.replay_later(
        id,
        ReplayOverride {
            method: Some("POST".into()),
            url: Some(format!("http://{origin_addr}/new?x=1")),
            headers: Some(vec![
                ("Content-Length".into(), "1".into()),
                ("X-Edit".into(), "yes".into()),
                ("Host".into(), "wrong.test".into()),
            ]),
            body: Some(b"abcdef".to_vec()),
        },
        move |result| {
            let _ = tx.send(result);
        },
    );
    let new_id = timeout(Duration::from_secs(5), rx)
        .await
        .expect("重放超时")
        .expect("重放任务中断")
        .expect("重放失败");
    let text = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
    assert!(text.starts_with("POST /new?x=1 HTTP/1.1"), "{text}");
    assert!(text.contains("Content-Length: 6\r\n"), "{text}");
    assert!(!text.contains("Content-Length: 1"), "{text}");
    assert!(text.contains("X-Edit: yes"), "{text}");
    assert!(!text.contains("X-Old"), "{text}");
    assert!(
        !text.to_ascii_lowercase().contains("host: wrong.test"),
        "{text}"
    );
    assert!(text.ends_with("abcdef"), "{text}");
    let flow = hub
        .with_session(|session| session.get(new_id).cloned())
        .unwrap();
    assert_eq!(flow.method, "POST");
    assert!(flow.url.contains("/new?x=1"), "{}", flow.url);
    assert_eq!(flow.status, Some(201));
    assert_eq!(flow.request_body, b"abcdef");
    assert_eq!(flow.response_body, b"okay");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn h2_sender_cache_reuses_the_origin_connection() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());
    let first = h2_get(&ca, proxy_addr, origin_addr, "/a").await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(&first.1[..], b"h2-/a");
    let second = h2_get(&ca, proxy_addr, origin_addr, "/b").await;
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(&second.1[..], b"h2-/b");
    tokio::task::yield_now().await;
    assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn h2_sender_cache_misses_after_idle() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-idle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());
    let first = h2_get(&ca, proxy_addr, origin_addr, "/a").await;
    assert_eq!(first.0, StatusCode::OK);
    hub.expire_h2_cache();
    let second = h2_get(&ca, proxy_addr, origin_addr, "/b").await;
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(&second.1[..], b"h2-/b");
    tokio::task::yield_now().await;
    assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn h2_lane_redials_on_the_same_client_connection() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-redial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());
    let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
    let first = h2_send(&mut sender, "/a").await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(&first.1[..], b"h2-/a");
    hub.refresh_h2
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let second = h2_send(&mut sender, "/b").await;
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(&second.1[..], b"h2-/b");
    tokio::task::yield_now().await;
    assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_map_local_answers_from_the_file() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-https-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let file = dir.join("body.txt");
    std::fs::write(&file, b"from-file").unwrap();
    hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
    let hub = Arc::new(hub);
    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let (origin_addr, seen_rx) = spawn_tls_sink(server);
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
    tls.write_all(b"GET /hi HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let resp = read_h1_message(&mut tls).await;
    drop(tls);
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("200"), "{text}");
    assert!(text.contains("from-file"), "{text}");
    let seen = timeout(Duration::from_secs(3), seen_rx)
        .await
        .expect("timed out waiting for sink close")
        .unwrap();
    assert!(seen.is_empty(), "map local 仍把请求送到了源站");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_map_remote_fetches_the_other_origin() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-https-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    hub.set_rules(&format!(
        "map-remote 127.0.0.1/hi http://{remote_addr}/landed"
    ));
    let hub = Arc::new(hub);
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = remote.accept().await.unwrap();
        let req = read_h1_message(&mut sock).await;
        *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nremote-ok",
        )
        .await
        .unwrap();
    });
    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let (origin_addr, _) = spawn_tls_sink(server);
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
    tls.write_all(b"GET /hi HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let resp = read_h1_message(&mut tls).await;
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("remote-ok"), "{text}");
    let remote_text = seen.lock().unwrap().clone();
    assert!(remote_text.contains("/landed"), "{remote_text}");
    let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
    assert!(flow.url.contains("/landed"), "{}", flow.url);
    assert_eq!(flow.response_body, b"remote-ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_map_remote_preserves_subpath_query_host_and_origin_form() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir =
        std::env::temp_dir().join(format!("zniff-https-remote-subpath-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());

    let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    hub.set_rules(&format!("map-remote 127.0.0.1/api http://{remote_addr}/v2"));
    let hub = Arc::new(hub);
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = remote.accept().await.unwrap();
        let req = read_h1_message(&mut sock).await;
        *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nremote-ok",
        )
        .await
        .unwrap();
    });

    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let (origin_addr, _) = spawn_tls_sink(server);
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());

    let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
    tls.write_all(
        b"GET /api/users?id=1 HTTP/1.1\r\nHost: client-custom.test\r\nConnection: close\r\n\r\n",
    )
    .await
    .unwrap();
    let resp = read_h1_message(&mut tls).await;
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("remote-ok"), "{text}");

    let remote_text = seen.lock().unwrap().clone();
    // 1. Verify origin-form request line (NOT absolute-form "GET http://...")
    assert!(
        remote_text.starts_with("GET /v2/users?id=1 HTTP/1.1\r\n"),
        "Expected origin-form request line, got: {remote_text}"
    );
    // 2. Verify Host header was rewritten to destination authority
    assert!(
        remote_text
            .to_ascii_lowercase()
            .contains(&format!("host: {remote_addr}\r\n")),
        "Expected rewritten Host header {remote_addr}, got: {remote_text}"
    );
    // 3. Verify original Host was NOT forwarded
    assert!(
        !remote_text.contains("client-custom.test"),
        "Original Host leaked: {remote_text}"
    );

    let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
    assert_eq!(flow.url, format!("http://{remote_addr}/v2/users?id=1"));
    assert_eq!(flow.path, "/v2/users?id=1");
    assert_eq!(flow.host, "127.0.0.1");
    assert_eq!(flow.response_body, b"remote-ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_map_remote_preserves_subpath_query_and_rewrites_host() {
    let dir = std::env::temp_dir().join(format!("zniff-plain-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Hub::new(ca.clone()).unwrap();

    let remote = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_addr = remote.local_addr().unwrap();
    hub.set_rules(&format!("map-remote fake.test/api http://{remote_addr}/v2"));
    let hub = Arc::new(hub);
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = remote.accept().await.unwrap();
        let req = read_h1_message(&mut sock).await;
        *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nplain-remote-ok",
        )
        .await
        .unwrap();
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
            .write_all(
                b"GET http://fake.test/api/users?id=1 HTTP/1.1\r\nHost: fake.test\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    let resp = read_h1_message(&mut client).await;
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("plain-remote-ok"), "{text}");

    let remote_text = seen.lock().unwrap().clone();
    assert!(
        remote_text.starts_with("GET /v2/users?id=1 HTTP/1.1\r\n"),
        "Expected origin-form request line, got: {remote_text}"
    );
    assert!(
        remote_text
            .to_ascii_lowercase()
            .contains(&format!("host: {remote_addr}\r\n")),
        "Expected rewritten Host header, got: {remote_text}"
    );
    assert!(
        !remote_text.contains("fake.test"),
        "Original Host leaked: {remote_text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_break_continue_rewrites_the_request() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-https-break-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules("break 127.0.0.1/hold");
    let hub = Arc::new(hub);
    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        let req = read_h1_message(&mut tls).await;
        *seen_task.lock().unwrap() = String::from_utf8_lossy(&req).into_owned();
        tls.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\ncontinued",
        )
        .await
        .unwrap();
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let hub_wait = hub.clone();
    let waiter = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait.paused_id() == Some(1) {
                assert!(hub_wait.resolve_break(BreakDecision {
                    drop: false,
                    headers: Some(vec![
                        ("Host".into(), "127.0.0.1".into()),
                        ("X-Edit".into(), "yes".into()),
                        ("Content-Length".into(), "99".into()),
                    ]),
                    body: Some(b"abcdef".to_vec()),
                    status: None,
                }));
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
    tls.write_all(b"POST /hold HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 8\r\nConnection: close\r\n\r\noriginal")
            .await
            .unwrap();
    let resp = read_h1_message(&mut tls).await;
    waiter.await.unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.contains("continued"), "{text}");
    let origin_text = seen.lock().unwrap().clone();
    assert!(origin_text.contains("x-edit: yes"), "{origin_text}");
    assert!(origin_text.contains("Content-Length: 6"), "{origin_text}");
    assert!(origin_text.ends_with("abcdef"), "{origin_text}");
    assert!(!origin_text.contains("original"), "{origin_text}");
    let flow = hub.with_session(|session| session.get(1).cloned()).unwrap();
    assert_eq!(flow.request_body, b"abcdef");
    assert_eq!(flow.response_body, b"continued");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_break_drop_does_not_reach_the_origin() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-break-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules("break 127.0.0.1/hold");
    let hub = Arc::new(hub);
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_task = hits.clone();
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let Ok(tls) = acceptor.accept(sock).await else {
            return;
        };
        let hits_task = hits_task.clone();
        let service = hyper::service::service_fn(move |_req: Request<Incoming>| {
            hits_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                    Bytes::from_static(b"origin"),
                )))
            }
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let hub_wait = hub.clone();
    let waiter = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if hub_wait.paused_id().is_some() {
                assert!(hub_wait.resolve_break(BreakDecision {
                    drop: true,
                    headers: None,
                    body: None,
                    status: None,
                }));
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!("断点没有暂停");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/hold")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let res = sender.send_request(req).await;
    waiter.await.unwrap();
    assert!(res.is_err(), "expected H2 stream reset on breakpoint drop");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    let flow = hub
        .with_session(|session| session.flows().into_iter().next())
        .unwrap();
    assert_eq!(flow.status, None);
    assert_eq!(flow.error.as_deref(), Some("断点丢弃"));
    assert!(flow.finished_at.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_map_local_answers_from_the_file() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let file = dir.join("body.txt");
    std::fs::write(&file, b"from-h2-file").unwrap();
    hub.set_rules(&format!("map-local 127.0.0.1/hi {}", file.display()));
    let hub = Arc::new(hub);
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_task = hits.clone();
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let Ok(tls) = acceptor.accept(sock).await else {
            return;
        };
        let hits_task = hits_task.clone();
        let service = hyper::service::service_fn(move |_req: Request<Incoming>| {
            hits_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                    Bytes::from_static(b"origin"),
                )))
            }
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/hi").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"from-h2-file");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

fn serve_one(proxy: TcpListener, hub: Arc<Hub>) {
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub).await;
    });
}

fn serve_proxy(proxy: TcpListener, hub: Arc<Hub>) {
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = proxy.accept().await else {
                break;
            };
            let hub = hub.clone();
            tokio::spawn(async move {
                let _ = handle_conn(socket, hub).await;
            });
        }
    });
}

fn origin_tls_config(
    origin_ca: &rcgen::Certificate,
    origin_leaf: &rcgen::Certificate,
    leaf_key: &rcgen::KeyPair,
    alpn: Vec<Vec<u8>>,
) -> rustls::ServerConfig {
    let mut server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![origin_leaf.der().clone(), origin_ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                leaf_key.serialize_der(),
            )),
        )
        .unwrap();
    server.alpn_protocols = alpn;
    server
}

fn spawn_tls_sink(
    server: rustls::ServerConfig,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Receiver<Vec<u8>>,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    origin.set_nonblocking(true).unwrap();
    let addr = origin.local_addr().unwrap();
    let origin = TcpListener::from_std(origin).unwrap();
    tokio::spawn(async move {
        let Ok((sock, _)) = origin.accept().await else {
            return;
        };
        let Ok(mut tls) = acceptor.accept(sock).await else {
            return;
        };
        let mut seen = Vec::new();
        let _ = timeout(Duration::from_secs(3), tls.read_to_end(&mut seen)).await;
        let _ = seen_tx.send(seen);
    });
    (addr, seen_rx)
}

async fn connect_mitm(
    proxy_addr: std::net::SocketAddr,
    origin_addr: std::net::SocketAddr,
    ca: &Ca,
    alpn: Vec<Vec<u8>>,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    tcp.write_all(connect.as_bytes()).await.unwrap();
    read_http_head(&mut tcp).await;
    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = alpn;
    timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(client_config))
            .connect(ServerName::IpAddress(origin_addr.ip().into()), tcp),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS")
}

async fn spawn_h2_origin(
    server: rustls::ServerConfig,
) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepts_task = accepts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = origin.accept().await else {
                break;
            };
            accepts_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(sock).await else {
                    return;
                };
                let service = hyper::service::service_fn(|req: Request<Incoming>| async move {
                    let body = format!("h2-{}", req.uri().path());
                    Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                        Bytes::from(body),
                    )))
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });
    (origin_addr, accepts)
}

async fn h2_client(
    ca: &Ca,
    proxy_addr: std::net::SocketAddr,
    origin_addr: std::net::SocketAddr,
) -> hyper::client::conn::http2::SendRequest<http_body_util::Full<Bytes>> {
    let tls = connect_mitm(proxy_addr, origin_addr, ca, vec![b"h2".to_vec()]).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
    let (sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
        .await
        .expect("客户端 HTTP/2");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
}

async fn h2_send(
    sender: &mut hyper::client::conn::http2::SendRequest<http_body_util::Full<Bytes>>,
    path: &str,
) -> (StatusCode, Bytes) {
    let req = Request::builder()
        .uri(path)
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("HTTP/2 响应超时")
        .expect("HTTP/2 响应");
    let status = response.status();
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (status, body)
}

async fn h2_get(
    ca: &Ca,
    proxy_addr: std::net::SocketAddr,
    origin_addr: std::net::SocketAddr,
    path: &str,
) -> (StatusCode, Bytes) {
    let tls = connect_mitm(proxy_addr, origin_addr, ca, vec![b"h2".to_vec()]).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, http_body_util::Full<Bytes>>(TokioIo::new(tls))
        .await
        .expect("客户端 HTTP/2");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let req = Request::builder()
        .uri(path)
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let response = timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("HTTP/2 响应超时")
        .expect("HTTP/2 响应");
    let status = response.status();
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (status, body)
}

async fn read_h1_message(reader: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = timeout(Duration::from_secs(5), reader.read(&mut byte))
            .await
            .expect("读超时")
            .expect("读失败");
        assert_ne!(n, 0, "连接提前关闭");
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        assert!(buf.len() < 16 * 1024, "头过长");
    }
    let head = String::from_utf8_lossy(&buf);
    let len = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let start = buf.len();
    buf.resize(start + len, 0);
    if len > 0 {
        timeout(Duration::from_secs(5), reader.read_exact(&mut buf[start..]))
            .await
            .expect("读 body 超时")
            .unwrap();
    }
    buf
}

use bytes::Bytes;
use http_body::{Body, Frame};
use std::pin::Pin;
use std::task::Poll;

struct ChunkThenEnd {
    pending: std::collections::VecDeque<Bytes>,
}

impl Body for ChunkThenEnd {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(chunk) = self.pending.pop_front() {
            return Poll::Ready(Some(Ok(Frame::data(chunk))));
        }
        // The caller stops once `is_end_stream` is true and never polls `None`.
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        self.pending.is_empty()
    }
}

struct AlreadyEnded;

impl Body for AlreadyEnded {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        true
    }
}

fn open_recorded(hub: &Hub) -> u64 {
    hub.with_session(|session| {
        session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "GET".into(),
            host: "example.com".into(),
            path: "/".into(),
            url: "https://example.com/".into(),
            request_headers: Vec::new(),
        })
    })
}

fn poll_once<B>(body: &mut RecordingBody<B>)
where
    B: Body<Data = Bytes> + Unpin,
{
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let _ = Pin::new(body).poll_frame(&mut cx);
}

#[test]
fn recording_body_finishes_on_end_of_stream_without_a_none_poll() {
    let dir = std::env::temp_dir().join(format!("zniff-body-end-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let id = open_recorded(&hub);
    hub.with_session(|session| session.set_response(id, 200, Vec::new(), None));
    {
        let mut body = RecordingBody::new(
            ChunkThenEnd {
                pending: std::collections::VecDeque::from([Bytes::from_static(b"hi")]),
            },
            hub.clone(),
            id,
            Direction::Response,
            None,
        );
        poll_once(&mut body);
        let flow = hub
            .with_session(|session| session.get(id).cloned())
            .unwrap();
        assert_eq!(flow.response_body, b"hi");
        assert!(
            flow.finished_at.is_some(),
            "last frame must finish the flow"
        );
    }

    let empty = open_recorded(&hub);
    hub.with_session(|session| session.set_response(empty, 304, Vec::new(), None));
    {
        let body = RecordingBody::new(AlreadyEnded, hub.clone(), empty, Direction::Response, None);
        drop(body);
    }
    let flow = hub
        .with_session(|session| session.get(empty).cloned())
        .unwrap();
    assert!(flow.response_body.is_empty());
    assert!(
        flow.finished_at.is_some(),
        "dropping an already-ended body must finish the flow"
    );

    let request = open_recorded(&hub);
    {
        let body = RecordingBody::new(AlreadyEnded, hub.clone(), request, Direction::Request, None);
        drop(body);
    }
    let flow = hub
        .with_session(|session| session.get(request).cloned())
        .unwrap();
    assert!(
        flow.finished_at.is_none(),
        "the request side copy must not mark the flow finished"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn status_only_map_local_query_rewrite_and_composed_request_work() {
    let dir = std::env::temp_dir().join(format!("zniff-p1-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    hub.set_rules(
        "map-local 127.0.0.1/status-only status=204\n\
             query 127.0.0.1/items set env staging\n\
             query 127.0.0.1/items del drop\n",
    );

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    // 1. Status-only map-local returns 204 with empty body without dialing origin.
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client
            .write_all(b"GET http://127.0.0.1:9/status-only HTTP/1.1\r\nHost: 127.0.0.1:9\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    let mut raw = Vec::new();
    client.read_to_end(&mut raw).await.unwrap();
    let head = String::from_utf8_lossy(&raw);
    assert!(head.starts_with("HTTP/1.1 204 "), "{head}");

    // 2. Standalone composed request + query rule rewrite against a real local origin.
    let origin = listen().await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = origin.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let req_text = String::from_utf8_lossy(&buf[..n]);
        assert!(
            req_text.starts_with("GET /items?keep=1&env=staging HTTP/1.1\r\n"),
            "{req_text}"
        );
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });

    let origin2 = listen().await.unwrap();
    let origin2_addr = origin2.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = origin2.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 201 Created\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone",
            )
            .await
            .unwrap();
    });

    let composed_id = replay_once(
        hub.clone(),
        None,
        ReplayOverride {
            method: Some("POST".into()),
            url: Some(format!("http://{origin2_addr}/scratch")),
            headers: Some(vec![("X-From".into(), "scratchpad".into())]),
            body: Some(b"hello".to_vec()),
        },
    )
    .await
    .unwrap();
    let composed_flow = hub
        .with_session(|session| session.get(composed_id).cloned())
        .unwrap();
    assert_eq!(composed_flow.status, Some(201));
    assert_eq!(composed_flow.response_body, b"done");

    let proxy3 = listen().await.unwrap();
    let proxy3_addr = proxy3.local_addr().unwrap();
    let hub_serve3 = hub.clone();
    tokio::spawn(async move {
        let (socket, _) = proxy3.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve3).await;
    });
    let mut client2 = TcpStream::connect(proxy3_addr).await.unwrap();
    let req = format!(
        "GET http://{origin_addr}/items?keep=1&drop=2 HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
    );
    client2.write_all(req.as_bytes()).await.unwrap();
    let mut raw2 = Vec::new();
    client2.read_to_end(&mut raw2).await.unwrap();
    assert!(String::from_utf8_lossy(&raw2).ends_with("ok"));

    let _ = std::fs::remove_dir_all(&dir);
}

async fn wait_paused_len(hub: &Hub, want: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while hub.paused_queue_len() < want && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(
        hub.paused_queue_len(),
        want,
        "timed out waiting for paused_queue_len == {want}"
    );
}

#[tokio::test]
async fn breakpoint_fifo_queue_and_local_app_cache_work() {
    let dir = std::env::temp_dir().join(format!("zniff-break-q-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let h1 = hub.clone();
    let w1 = tokio::spawn(async move { h1.wait_break(10, BreakSide::Request).await });
    wait_paused_len(&hub, 1).await;
    let h2 = hub.clone();
    let w2 = tokio::spawn(async move { h2.wait_break(20, BreakSide::Response).await });
    wait_paused_len(&hub, 2).await;
    let h3 = hub.clone();
    let w3 = tokio::spawn(async move { h3.wait_break(30, BreakSide::Request).await });
    wait_paused_len(&hub, 3).await;
    assert_eq!(hub.paused(), Some((10, BreakSide::Request)));

    assert!(hub.resolve_break(BreakDecision {
        drop: true,
        headers: None,
        body: None,
        status: None,
    }));
    let d1 = w1.await.unwrap();
    assert!(d1.drop);

    assert_eq!(hub.paused_queue_len(), 2);
    assert_eq!(hub.paused(), Some((20, BreakSide::Response)));

    assert_eq!(hub.resolve_all_breaks(), 2);
    assert_eq!(hub.paused_queue_len(), 0);
    let d2 = w2.await.unwrap();
    let d3 = w3.await.unwrap();
    assert!(!d2.drop);
    assert!(!d3.drop);

    store_cached_local_app(65431, "curl".into());
    assert_eq!(cached_local_app(65431).as_deref(), Some("curl"));
    assert_eq!(resolve_local_process_name(0), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn promote_break_selects_that_slot_without_releasing_the_others() {
    let dir = std::env::temp_dir().join(format!("zniff-break-promote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let h1 = hub.clone();
    let w1 = tokio::spawn(async move { h1.wait_break(10, BreakSide::Request).await });
    wait_paused_len(&hub, 1).await;
    let h2 = hub.clone();
    let w2 = tokio::spawn(async move { h2.wait_break(20, BreakSide::Response).await });
    wait_paused_len(&hub, 2).await;
    let h3 = hub.clone();
    let mut w3 = tokio::spawn(async move { h3.wait_break(30, BreakSide::Request).await });
    wait_paused_len(&hub, 3).await;
    assert_eq!(
        hub.paused_queue(),
        vec![
            (10, BreakSide::Request),
            (20, BreakSide::Response),
            (30, BreakSide::Request),
        ]
    );
    assert!(hub.promote_break(30));
    assert_eq!(hub.paused(), Some((30, BreakSide::Request)));
    assert_eq!(
        hub.paused_queue(),
        vec![
            (30, BreakSide::Request),
            (10, BreakSide::Request),
            (20, BreakSide::Response),
        ]
    );
    assert!(!hub.promote_break(99));
    assert_eq!(hub.paused_queue_len(), 3);
    assert!(hub.promote_break(30));
    assert_eq!(hub.paused_queue()[0], (30, BreakSide::Request));

    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        !w1.is_finished(),
        "promoting another slot released the first"
    );
    assert!(!w2.is_finished());

    assert!(hub.resolve_break(BreakDecision {
        drop: true,
        headers: None,
        body: None,
        status: None,
    }));
    let dropped = timeout(Duration::from_secs(2), &mut w3)
        .await
        .expect("promoted break did not finish")
        .unwrap();
    assert!(dropped.drop);
    assert_eq!(hub.paused(), Some((10, BreakSide::Request)));
    assert_eq!(hub.paused_queue_len(), 2);

    assert_eq!(hub.resolve_all_breaks(), 2);
    let kept_first = w1.await.unwrap();
    let kept_second = w2.await.unwrap();
    assert!(!kept_first.drop);
    assert!(!kept_second.drop);
    assert!(hub.paused_queue().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn replay_batch_repeats_sequentially_and_records_each_flow() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-repeat-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin = listen().await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        for seq in 1..=3u32 {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let body = format!("hit-{seq}");
            let res = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(res.as_bytes()).await.unwrap();
        }
    });

    let source_id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: origin_addr.to_string(),
        path: "/repeat-target".into(),
        url: format!("http://{origin_addr}/repeat-target"),
        request_headers: vec![("Host".into(), origin_addr.to_string())],
    });
    hub.set_response(source_id, 200, Vec::new(), None);
    hub.mark_finished(source_id);
    let source_flow = hub
        .with_session(|session| session.hydrated(source_id))
        .unwrap();

    let ids = replay_batch(
        hub.clone(),
        source_flow.clone(),
        3,
        1,
        5,
        ReplayOverride::default(),
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 3);
    for (idx, id) in ids.iter().enumerate() {
        let flow = hub
            .with_session(|session| session.get(*id).cloned())
            .unwrap();
        assert_eq!(flow.status, Some(200));
        assert_eq!(
            String::from_utf8_lossy(&flow.response_body),
            format!("hit-{}", idx + 1)
        );
        assert!(
            flow.comment
                .contains(&format!("repeat:{source_id} ({}/3)", idx + 1))
        );
    }
    let repeat_filtered =
        hub.with_session(|session| session.flows_matching(&format!("repeat:{source_id}")));
    assert_eq!(repeat_filtered.len(), 4);

    // A transient origin failure on an intermediate iteration records ERR on that flow
    // and continues replaying the remaining iterations.
    let flaky_origin = listen().await.unwrap();
    let flaky_addr = flaky_origin.local_addr().unwrap();
    tokio::spawn(async move {
        for seq in 1..=3u32 {
            let (mut socket, _) = flaky_origin.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            if seq == 2 {
                drop(socket);
                continue;
            }
            let body = format!("flaky-{seq}");
            let res = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(res.as_bytes()).await.unwrap();
        }
    });
    let mut flaky_source = source_flow.clone();
    flaky_source.host = flaky_addr.to_string();
    flaky_source.url = format!("http://{flaky_addr}/repeat-target");
    flaky_source.request_headers = vec![("Host".into(), flaky_addr.to_string())];
    let flaky_ids = replay_batch(
        hub.clone(),
        flaky_source,
        3,
        1,
        0,
        ReplayOverride::default(),
    )
    .await
    .unwrap();
    assert_eq!(flaky_ids.len(), 3);
    let first = hub
        .with_session(|session| session.get(flaky_ids[0]).cloned())
        .unwrap();
    let second = hub
        .with_session(|session| session.get(flaky_ids[1]).cloned())
        .unwrap();
    let third = hub
        .with_session(|session| session.get(flaky_ids[2]).cloned())
        .unwrap();
    assert_eq!(first.status, Some(200));
    assert_eq!(second.status, None);
    assert!(second.error.is_some());
    assert_eq!(third.status, Some(200));
    assert_eq!(String::from_utf8_lossy(&third.response_body), "flaky-3");

    // Concurrent batch replay launches multiple in-flight requests and tags each flow.
    let conc_origin = listen().await.unwrap();
    let conc_addr = conc_origin.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..4 {
            let (mut socket, _) = conc_origin.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                let body = "conc-ok";
                let res = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(res.as_bytes()).await.unwrap();
            });
        }
    });
    let mut conc_source = source_flow;
    conc_source.host = conc_addr.to_string();
    conc_source.url = format!("http://{conc_addr}/repeat-conc");
    conc_source.request_headers = vec![("Host".into(), conc_addr.to_string())];
    conc_source.comment = "load-test".into();
    let conc_ids = replay_batch(hub.clone(), conc_source, 4, 3, 0, ReplayOverride::default())
        .await
        .unwrap();
    assert_eq!(conc_ids.len(), 4);
    for (idx, id) in conc_ids.iter().enumerate() {
        let flow = hub
            .with_session(|session| session.get(*id).cloned())
            .unwrap();
        assert_eq!(flow.status, Some(200));
        assert_eq!(
            flow.comment,
            format!("load-test · repeat:{source_id} ({}/4)", idx + 1)
        );
    }
    assert_eq!(throttle_sleep_duration(4096, Some(4096)).as_millis(), 1000);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_upstream_mtls_authenticates_with_client_cert_rule() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-mtls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();

    // Issue a client certificate signed by `origin_ca`
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let client_ca_key = rcgen::KeyPair::generate().unwrap();
    let client_ca = ca_params.self_signed(&client_ca_key).unwrap();
    let client_issuer = rcgen::Issuer::new(ca_params, client_ca_key);
    let client_key = rcgen::KeyPair::generate().unwrap();
    let mut client_params = rcgen::CertificateParams::default();
    client_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params
        .signed_by(&client_key, &client_issuer)
        .unwrap();

    let client_cert_path = dir.join("client.crt");
    let client_key_path = dir.join("client.key");
    std::fs::write(&client_cert_path, client_cert.pem()).unwrap();
    std::fs::write(&client_key_path, client_key.serialize_pem()).unwrap();

    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules(&format!(
        "client-cert 127.0.0.1 {} {}",
        client_cert_path.display(),
        client_key_path.display()
    ));
    let hub = Arc::new(hub);

    // Configure an origin server that requires client certificate authentication
    let mut client_roots = RootCertStore::empty();
    client_roots.add(client_ca.der().clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .unwrap();
    let cert_der = rustls::pki_types::CertificateDer::from(origin_leaf.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
    );
    let mut server_cfg = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    server_cfg.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let tls = acceptor.accept(sock).await.unwrap();
        let peer_certs = tls.get_ref().1.peer_certificates();
        assert!(peer_certs.is_some_and(|c| !c.is_empty()));
        let service = hyper::service::service_fn(move |_req: Request<Incoming>| async move {
            Ok::<_, std::convert::Infallible>(Response::new(http_body_util::Full::new(
                Bytes::from_static(b"mtls-verified"),
            )))
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());
    let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/mtls-check").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"mtls-verified");
    let flow = hub
        .with_session(|session| session.flows().into_iter().next())
        .unwrap();
    assert_eq!(flow.rule_hit, "client-cert");
    assert_eq!(
        flow.conn_info.client_cert,
        client_cert_path.display().to_string()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_mtls_broken_or_mismatched_client_cert_fails_closed_with_502_and_paths() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-mtls-failclosed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());

    let cert_a_key = rcgen::KeyPair::generate().unwrap();
    let cert_a = rcgen::CertificateParams::default()
        .self_signed(&cert_a_key)
        .unwrap();
    let wrong_key = rcgen::KeyPair::generate().unwrap();

    let cert_path = dir.join("client-a.crt");
    let wrong_key_path = dir.join("wrong-b.key");
    std::fs::write(&cert_path, cert_a.pem()).unwrap();
    std::fs::write(&wrong_key_path, wrong_key.serialize_pem()).unwrap();

    let hub = Hub::new(ca.clone()).unwrap();
    hub.set_rules(&format!(
        "client-cert 127.0.0.1 {} {}",
        cert_path.display(),
        wrong_key_path.display()
    ));
    let hub = Arc::new(hub);

    // Origin listener that must NEVER receive a fail-open tunnel connection
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let origin_accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepts_bg = origin_accepts.clone();
    tokio::spawn(async move {
        if origin.accept().await.is_ok() {
            accepts_bg.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // 1. KeyMismatch via `client-cert` rule returns 502 Bad Gateway over H2 without dialing origin
    let (status, body) = h2_get(&ca, proxy_addr, origin_addr, "/secure-api").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let body_text = String::from_utf8_lossy(&body);
    assert!(body_text.contains("客户端证书加载失败:"), "{body_text}");
    assert!(
        body_text.contains(&cert_path.display().to_string()),
        "{body_text}"
    );
    assert!(
        body_text.contains(&wrong_key_path.display().to_string()),
        "{body_text}"
    );
    assert!(!body_text.contains("已改为隧道"), "{body_text}");
    assert_eq!(origin_accepts.load(std::sync::atomic::Ordering::SeqCst), 0);

    let flows = hub.with_session(|session| session.flows());
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].status, Some(502));
    assert_eq!(flows[0].rule_hit, "client-cert");
    let err_msg = flows[0].error.as_deref().unwrap_or_default();
    assert!(err_msg.starts_with("客户端证书加载失败:"), "{err_msg}");
    assert!(err_msg.contains("cert="), "{err_msg}");
    assert!(err_msg.contains("key="), "{err_msg}");
    assert!(!err_msg.contains("已改为隧道"), "{err_msg}");

    // 2. Encrypted PEM in `<data_dir>/client-certs/127.0.0.1.pem` also fails closed with 502
    hub.set_rules("");
    let cert_dir = dir.join("client-certs");
    std::fs::create_dir_all(&cert_dir).unwrap();
    let enc_pem = cert_dir.join("127.0.0.1.pem");
    std::fs::write(
        &enc_pem,
        format!(
            "{}-----BEGIN ENCRYPTED PRIVATE KEY-----\nZm9v\n-----END ENCRYPTED PRIVATE KEY-----\n",
            cert_a.pem()
        ),
    )
    .unwrap();

    let (status2, body2) = h2_get(&ca, proxy_addr, origin_addr, "/dir-cert").await;
    assert_eq!(status2, StatusCode::BAD_GATEWAY);
    let body2_text = String::from_utf8_lossy(&body2);
    assert!(body2_text.contains("ENCRYPTED PRIVATE KEY"), "{body2_text}");
    assert_eq!(origin_accepts.load(std::sync::atomic::Ordering::SeqCst), 0);

    let flows = hub.with_session(|session| session.flows());
    assert_eq!(flows.len(), 2);
    assert_eq!(flows[1].status, Some(502));
    assert_eq!(flows[1].rule_hit, "client-cert");

    // 3. Valid directory-discovered client cert records `conn_info.client_cert` and `rule_hit`
    // even when the origin returns HTTP 400.
    std::fs::write(
        &enc_pem,
        format!("{}{}", cert_a.pem(), cert_a_key.serialize_pem()),
    )
    .unwrap();
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub2 = Hub::new(ca.clone()).unwrap();
    hub2.add_trust_anchor(origin_ca.der().clone());
    let hub2 = Arc::new(hub2);

    let cert_der = rustls::pki_types::CertificateDer::from(origin_leaf.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
    );
    let mut server_cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap();
    server_cfg.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));
    let origin400 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin400_addr = origin400.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin400.accept().await.unwrap();
        let tls = acceptor.accept(sock).await.unwrap();
        let service = hyper::service::service_fn(move |_req: Request<Incoming>| async move {
            let mut res = Response::new(http_body_util::Full::new(Bytes::from_static(
                b"No required SSL certificate was sent",
            )));
            *res.status_mut() = StatusCode::BAD_REQUEST;
            Ok::<_, std::convert::Infallible>(res)
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });

    let proxy2 = listen().await.unwrap();
    let proxy2_addr = proxy2.local_addr().unwrap();
    serve_one(proxy2, hub2.clone());
    let (status3, _) = h2_get(&ca, proxy2_addr, origin400_addr, "/badssl").await;
    assert_eq!(status3, StatusCode::BAD_REQUEST);
    let flow3 = hub2
        .with_session(|session| session.flows().into_iter().next())
        .unwrap();
    assert_eq!(flow3.rule_hit, "client-cert");
    assert_eq!(flow3.conn_info.client_cert, enc_pem.display().to_string());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_map_local_directory_serves_assets_and_falls_back_to_upstream() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-mapdir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dist = dir.join("dist");
    std::fs::create_dir_all(dist.join("js")).unwrap();
    std::fs::write(dist.join("index.html"), b"<html>index</html>").unwrap();
    std::fs::write(dist.join("js/app.js"), b"export const v = 1;").unwrap();

    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules(&format!(
        "map-local 127.0.0.1/assets \"{}\" fallback",
        dist.display()
    ));
    let hub = Arc::new(hub);

    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;

    // 1. Existing local asset is served from disk without hitting upstream handler
    let (status, body) = h2_send(&mut sender, "/assets/js/app.js?v=42").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"export const v = 1;");

    // 2. Directory root defaults to index.html
    let (status, body) = h2_send(&mut sender, "/assets/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"<html>index</html>");

    // 3. Missing file falls back to upstream when `fallback` is enabled
    let (status, body) = h2_send(&mut sender, "/assets/js/remote-only.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"h2-/assets/js/remote-only.js");
    assert_eq!(accepts.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 4. Directory traversal is blocked with 502 even when `fallback` is enabled
    let (status, _body) = h2_send(&mut sender, "/assets/%2e%2e/ca.crt").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let flows = hub.with_session(|session| session.flows());
    assert_eq!(flows.len(), 4);
    assert!(
        flows[0].response_headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("content-type") && v.starts_with("application/javascript")
        }),
        "expected application/javascript Content-Type on .js asset"
    );
    assert!(flows[2].comment.contains("map-local(fallback)"));
    assert!(
        flows[3]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("拒绝越界路径"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_records_conn_info_and_marks_subsequent_streams_reused() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-conninfo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, _accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;
    let (status1, _) = h2_send(&mut sender, "/first").await;
    assert_eq!(status1, StatusCode::OK);
    let (status2, _) = h2_send(&mut sender, "/second").await;
    assert_eq!(status2, StatusCode::OK);

    let flows = hub.with_session(|session| session.flows());
    assert_eq!(flows.len(), 2);
    assert!(!flows[0].conn_info.reused);
    assert!(flows[0].conn_info.tcp_ms.is_some());
    assert!(flows[0].conn_info.tls_ms.is_some());
    assert_eq!(flows[0].conn_info.server_addr, origin_addr.to_string());
    assert!(flows[0].conn_info.tls_version.starts_with("TLS 1."));

    assert!(flows[1].conn_info.reused);
    assert!(flows[1].conn_info.tcp_ms.is_none());
    assert!(flows[1].conn_info.tls_ms.is_none());
    assert_eq!(flows[1].conn_info.server_addr, origin_addr.to_string());
    assert_eq!(
        flows[1].conn_info.tls_version,
        flows[0].conn_info.tls_version
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn local_mcp_endpoint_serves_jsonrpc_without_recording_flow() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-mcp-http-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        proxy_addr.port(),
        payload.len()
    );
    let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.write_all(payload).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    let (_, body) = text.split_once("\r\n\r\n").unwrap();
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["id"], 1);
    assert!(json["result"]["tools"].is_array());
    // MCP calls must not pollute the captured flow session
    assert_eq!(hub.with_session(|session| session.len()), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_applies_no_cache_and_block_rules() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-nocache-block-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules(
        "no-cache 127.0.0.1/api\n\
             block 127.0.0.1/ads\n\
             block 127.0.0.1/ban status=451\n\
             block 127.0.0.1/drop drop\n",
    );
    let hub = Arc::new(hub);

    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (origin_addr, _accepts) = spawn_h2_origin(server).await;
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut sender = h2_client(&ca, proxy_addr, origin_addr).await;

    // 1. no-cache sets Cache-Control on request and response
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/items")
        .header("If-None-Match", "\"etag-1\"")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let res = sender.send_request(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-cache, no-store, must-revalidate")
    );
    let _ = res.into_body().collect().await.unwrap();

    // 2. block default 403
    let (status_403, body_403) = h2_send(&mut sender, "/ads/pixel").await;
    assert_eq!(status_403, StatusCode::FORBIDDEN);
    assert!(body_403.is_empty());

    // 3. block custom status 451
    let (status_451, body_451) = h2_send(&mut sender, "/ban/check").await;
    assert_eq!(status_451.as_u16(), 451);
    assert!(body_451.is_empty());

    // 4. block drop resets stream / closes connection and records error
    let drop_req = Request::builder()
        .method(Method::GET)
        .uri("/drop")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let drop_res = sender.send_request(drop_req).await;
    assert!(drop_res.is_err());

    let flows = hub.with_session(|session| session.flows());
    assert_eq!(flows.len(), 4);
    assert!(
        !flows[0]
            .request_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("if-none-match"))
    );
    assert!(flows[0].request_headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("cache-control") && v == "no-cache, no-store, must-revalidate"
    }));
    assert_eq!(flows[1].status, Some(403));
    assert_eq!(flows[2].status, Some(451));
    assert_eq!(flows[3].error.as_deref(), Some("规则阻断 (连接已断开)"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h1_redials_on_the_same_client_connection_when_origin_closes_idle() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h1-redial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let accepts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepts_bg = accepts.clone();
    let (first_closed_tx, first_closed_rx) = tokio::sync::oneshot::channel::<()>();
    let first_closed_tx = Arc::new(Mutex::new(Some(first_closed_tx)));
    let first_closed_bg = first_closed_tx.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = origin.accept().await else {
                break;
            };
            let n = accepts_bg.fetch_add(1, Ordering::SeqCst) + 1;
            let acceptor = acceptor.clone();
            let closed_tx = first_closed_bg.clone();
            tokio::spawn(async move {
                let mut tls = acceptor.accept(sock).await.unwrap();
                let mut buf = vec![0u8; 4096];
                let _ = tls.read(&mut buf).await.unwrap();
                let body = format!("h1-origin-{n}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{body}",
                    body.len()
                );
                tls.write_all(resp.as_bytes()).await.unwrap();
                tls.shutdown().await.ok();
                drop(tls);
                if n == 1
                    && let Some(tx) = closed_tx.lock().unwrap().take()
                {
                    let _ = tx.send(());
                }
            });
        }
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut client_tcp = TcpStream::connect(proxy_addr).await.unwrap();
    let connect = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
    client_tcp.write_all(connect.as_bytes()).await.unwrap();
    let mut head = [0u8; 128];
    let n = client_tcp.read(&mut head).await.unwrap();
    assert!(String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 200"));

    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let mut client_cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(client_cfg));
    let tls = connector
        .connect(server_name("127.0.0.1").unwrap(), client_tcp)
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    // First request on the client TLS connection
    let req1 = Request::builder()
        .method(Method::GET)
        .uri("/first")
        .header("Host", origin_addr.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap();
    let res1 = sender.send_request(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = res1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body1[..], b"h1-origin-1");

    // Wait deterministically for the first origin connection handler to close cleanly
    timeout(Duration::from_secs(2), first_closed_rx)
        .await
        .expect("first origin close timed out")
        .unwrap();

    // Second request reuses the SAME client-to-proxy TLS connection; proxy redials origin
    sender.ready().await.unwrap();
    let req2 = Request::builder()
        .method(Method::GET)
        .uri("/second")
        .header("Host", origin_addr.to_string())
        .body(Full::new(Bytes::new()))
        .unwrap();
    let res2 = sender.send_request(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = res2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body2[..], b"h1-origin-2");
    assert_eq!(accepts.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn map_remote_http_and_https_preserve_post_body_and_apply_req_res_rules() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-map-remote-post-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());

    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let tls_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_addr = tls_origin.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let seen_bg = seen.clone();

    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = tls_origin.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let seen_bg = seen_bg.clone();
            tokio::spawn(async move {
                let mut tls = acceptor.accept(sock).await.unwrap();
                let (head_bytes, mut rest) = read_header_block(&mut tls).await.unwrap();
                let head = parse_head(&head_bytes).unwrap();
                let len = content_length(&head.headers).unwrap_or(0);
                while rest.len() < len {
                    let mut chunk = vec![0u8; len - rest.len()];
                    let n = tls.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    rest.extend_from_slice(&chunk[..n]);
                }
                seen_bg
                    .lock()
                    .unwrap()
                    .push((head.start.clone(), rest[..len].to_vec()));

                // Send 103 Early Hints before final 200 OK JSON
                tls.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </app.js>; rel=preload\r\n\r\n")
                    .await
                    .unwrap();
                let resp_body = br#"{"ok":false,"src":"origin"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    resp_body.len()
                );
                tls.write_all(resp.as_bytes()).await.unwrap();
                tls.write_all(resp_body).await.unwrap();
                let _ = tls.shutdown().await;
            });
        }
    });

    hub.set_rules(&format!(
        "map-remote api.example.test/v1 https://{tls_addr}/mapped\n\
             query api.example.test set env prod\n\
             json api.example.test req set /msg \"from-rule\"\n\
             header api.example.test res set X-Mapped-Res applied\n\
             cors api.example.test\n\
             json api.example.test res set /ok true\n\
             map-remote 127.0.0.1/v2 https://{tls_addr}/mapped2\n\
             query 127.0.0.1/v2 set env h2\n\
             json 127.0.0.1/v2 req set /msg \"from-h2\"\n\
             header 127.0.0.1/v2 res set X-Mapped-Res h2-ok\n\
             json 127.0.0.1/v2 res set /ok true\n"
    ));
    let hub = Arc::new(hub);

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // 1. Plain HTTP -> HTTPS map-remote with POST JSON body
    let payload = br#"{"msg":"orig","keep":1}"#;
    let req = format!(
        "POST http://api.example.test/v1/submit HTTP/1.1\r\nHost: api.example.test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    client.write_all(req.as_bytes()).await.unwrap();
    client.write_all(payload).await.unwrap();
    let mut raw_resp = Vec::new();
    client.read_to_end(&mut raw_resp).await.unwrap();
    let text = String::from_utf8_lossy(&raw_resp);
    assert!(text.contains("HTTP/1.1 200 OK"), "{text}");
    assert!(
        text.to_ascii_lowercase().contains("x-mapped-res: applied"),
        "{text}"
    );
    assert!(
        text.to_ascii_lowercase()
            .contains("access-control-allow-origin: *"),
        "{text}"
    );
    assert!(text.ends_with(r#"{"ok":true,"src":"origin"}"#), "{text}");

    // 2. HTTPS -> HTTPS cross-port map-remote (`fetch_remote`) with POST JSON body
    let dummy_server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (dummy_addr, _) = spawn_h2_origin(dummy_server).await;
    let mut h2_sender = h2_client(&ca, proxy_addr, dummy_addr).await;
    let h2_req = Request::builder()
        .method(Method::POST)
        .uri("/v2/items")
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(br#"{"msg":"h2-orig"}"#)))
        .unwrap();
    let h2_res = h2_sender.send_request(h2_req).await.unwrap();
    assert_eq!(h2_res.status(), StatusCode::OK);
    assert_eq!(
        h2_res
            .headers()
            .get("x-mapped-res")
            .and_then(|v| v.to_str().ok()),
        Some("h2-ok")
    );
    let h2_body = h2_res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&h2_body[..], br#"{"ok":true,"src":"origin"}"#);

    let captured = seen.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].0, "POST /mapped/submit?env=prod HTTP/1.1");
    assert_eq!(captured[0].1, br#"{"msg":"from-rule","keep":1}"#);
    assert_eq!(captured[1].0, "POST /mapped2/items?env=h2 HTTP/1.1");
    assert_eq!(captured[1].1, br#"{"msg":"from-h2"}"#);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn head_and_204_without_content_length_do_not_hang_keep_alive_and_replay_skips_103() {
    let dir = std::env::temp_dir().join(format!("zniff-no-body-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let accepts = Arc::new(AtomicU64::new(0));
    let accepts_bg = accepts.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = origin.accept().await {
            let conn_num = accepts_bg.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::spawn(async move {
                loop {
                    let Ok((head_bytes, _)) = read_header_block(&mut sock).await else {
                        break;
                    };
                    let Ok(head) = parse_head(&head_bytes) else {
                        break;
                    };
                    if head.start.starts_with("HEAD /doc ") {
                        // Non-zero Content-Length on HEAD, zero body bytes, keep socket open!
                        sock.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\nKeep-Alive: timeout=5\r\n\r\n",
                            )
                            .await
                            .unwrap();
                    } else if head.start.starts_with("GET /empty ") {
                        // 204 with NO Content-Length header, keep socket open!
                        sock.write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                            .await
                            .unwrap();
                    } else if head.start.starts_with("GET /hint ") {
                        sock.write_all(
                                b"HTTP/1.1 103 Early Hints\r\nLink: </s.css>; rel=preload\r\n\r\n\
                                  HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhinted",
                            )
                            .await
                            .unwrap();
                        break;
                    } else {
                        let body = format!("conn-{conn_num}");
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        );
                        sock.write_all(reply.as_bytes()).await.unwrap();
                    }
                }
            });
        }
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // 1. HEAD with Content-Length: 9999 returns immediately without waiting for 9999 bytes
    let mut s1 = TcpStream::connect(proxy_addr).await.unwrap();
    s1.write_all(
        format!("HEAD http://{origin_addr}/doc HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let (h1, r1) = timeout(Duration::from_secs(2), read_header_block(&mut s1))
        .await
        .expect("HEAD must not hang")
        .unwrap();
    assert!(String::from_utf8_lossy(&h1).starts_with("HTTP/1.1 200 OK"));
    assert!(r1.is_empty());

    // 2. 204 without Content-Length returns immediately and parks origin socket for reuse
    let mut s2 = TcpStream::connect(proxy_addr).await.unwrap();
    s2.write_all(
        format!("GET http://{origin_addr}/empty HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let (h2, r2) = timeout(Duration::from_secs(2), read_header_block(&mut s2))
        .await
        .expect("204 without Content-Length must not hang")
        .unwrap();
    assert!(String::from_utf8_lossy(&h2).starts_with("HTTP/1.1 204"));
    assert!(r2.is_empty());

    // 3. Follow-up GET reuses the same parked origin TCP socket (conn-1)
    let pool_deadline = Instant::now() + Duration::from_secs(2);
    while hub.pool.lock().await.is_empty() && Instant::now() < pool_deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !hub.pool.lock().await.is_empty(),
        "204 response should have parked origin socket in HTTP/1.1 pool"
    );
    let mut s3 = TcpStream::connect(proxy_addr).await.unwrap();
    s3.write_all(
            format!("GET http://{origin_addr}/after HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let (_, mut r3) = timeout(Duration::from_secs(2), read_header_block(&mut s3))
        .await
        .unwrap()
        .unwrap();
    while r3.len() < 6 {
        let mut tmp = [0u8; 16];
        let n = s3.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        r3.extend_from_slice(&tmp[..n]);
    }
    assert_eq!(&r3[..], b"conn-1");
    assert_eq!(accepts.load(Ordering::SeqCst), 1);

    // 4. replay_once skips 103 Early Hints and records 200 + final body
    let replay_id = timeout(
        Duration::from_secs(2),
        replay_once(
            hub.clone(),
            None,
            ReplayOverride {
                method: Some("GET".into()),
                url: Some(format!("http://{origin_addr}/hint")),
                headers: Some(Vec::new()),
                body: Some(Vec::new()),
            },
        ),
    )
    .await
    .expect("replay must not hang")
    .unwrap();
    let replayed = hub.with_session(|s| s.get(replay_id).cloned()).unwrap();
    assert_eq!(replayed.status, Some(200));
    assert_eq!(replayed.response_body, b"hinted");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn switching_upstream_invalidates_pools_and_https_h1_preserves_header_case() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-pool-inval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    // Part A: H2 origin + dynamic upstream CONNECT proxy
    let h2_server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let (h2_origin_addr, _origin_accepts) = spawn_h2_origin(h2_server).await;

    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_hits = Arc::new(AtomicU64::new(0));
    let upstream_hits_bg = upstream_hits.clone();
    tokio::spawn(async move {
        while let Ok((mut client_sock, _)) = upstream.accept().await {
            upstream_hits_bg.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let (head_bytes, _) = read_header_block(&mut client_sock).await.unwrap();
                let head = parse_head(&head_bytes).unwrap();
                let target = head.start.split_whitespace().nth(1).unwrap();
                let mut out = TcpStream::connect(target).await.unwrap();
                client_sock
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client_sock, &mut out).await;
            });
        }
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut sender = h2_client(&ca, proxy_addr, h2_origin_addr).await;
    let (st1, _) = h2_send(&mut sender, "/direct").await;
    assert_eq!(st1, StatusCode::OK);
    assert_eq!(upstream_hits.load(Ordering::SeqCst), 0);

    // Dynamically switch upstream proxy while `sender` (`H2Lane`) and `h2_pool` are live!
    hub.set_upstream(&upstream_addr.to_string());
    let (st2, _) = h2_send(&mut sender, "/via-upstream").await;
    assert_eq!(st2, StatusCode::OK);
    assert_eq!(
        upstream_hits.load(Ordering::SeqCst),
        1,
        "live H2 lane must redial through the new upstream proxy immediately"
    );
    hub.set_upstream("");

    // Part B: HTTPS HTTP/1.1 preserves non-Title-Case response headers (e.g. ETag, x-custom-ID)
    let h1_server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let h1_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(h1_server));
    let h1_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let h1_origin_addr = h1_origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = h1_origin.accept().await.unwrap();
        let mut tls = h1_acceptor.accept(sock).await.unwrap();
        let _ = read_header_block(&mut tls).await.unwrap();
        tls.write_all(
                b"HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nx-custom-ID: 42\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
            )
            .await
            .unwrap();
        let _ = tls.shutdown().await;
    });

    let mut tls_client =
        connect_mitm(proxy_addr, h1_origin_addr, &ca, vec![b"http/1.1".to_vec()]).await;
    tls_client
        .write_all(b"GET /case HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let (raw_head, _) = read_header_block(&mut tls_client).await.unwrap();
    let head_str = String::from_utf8_lossy(&raw_head);
    assert!(head_str.contains("ETag: \"v1\"\r\n"), "{head_str}");
    assert!(head_str.contains("x-custom-ID: 42\r\n"), "{head_str}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn head_local_routes_preserve_method_and_omit_wire_body() {
    let dir = std::env::temp_dir().join(format!("zniff-head-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    let (share_id, _) = hub.begin_share(Duration::from_secs(60), "");

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    for path in [
        "/zniff".to_string(),
        "/zniff-ca.crt".to_string(),
        format!("/s/{share_id}"),
    ] {
        let mut sock = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            proxy_addr.port()
        );
        sock.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        timeout(Duration::from_secs(2), sock.read_to_end(&mut raw))
            .await
            .unwrap()
            .unwrap();
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("header terminator")
            + 4;
        let head = parse_head(&raw[..split]).unwrap();
        assert!(
            head.start.starts_with("HTTP/1.1 200 OK"),
            "unexpected status for {path}: {}",
            head.start
        );
        let declared_len = content_length(&head.headers).expect("Content-Length present");
        assert!(
            declared_len > 0,
            "Content-Length should reflect resource size for {path}"
        );
        assert!(
            raw[split..].is_empty(),
            "HEAD {path} must not write response body bytes on the wire (got {} bytes)",
            raw[split..].len()
        );
    }

    let flows = hub.with_session(|s| s.flows());
    assert_eq!(flows.len(), 2);
    assert_eq!(flows[0].method, "HEAD");
    assert_eq!(flows[0].path, "/zniff");
    assert!(flows[0].response_body.is_empty());
    assert_eq!(flows[1].method, "HEAD");
    assert_eq!(flows[1].path, "/zniff-ca.crt");
    assert!(flows[1].response_body.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn replay_batch_and_compose_support_pure_h2_origin_without_pooling() {
    use std::io::Write as _;

    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);

    // Pure h2-only TLS origin returning gzipped JSON body
    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let hits = Arc::new(AtomicU64::new(0));
    let hits_bg = hits.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = origin.accept().await {
            let acceptor = acceptor.clone();
            let hits_bg = hits_bg.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(sock).await else {
                    return;
                };
                assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
                let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let hits_bg = hits_bg.clone();
                    async move {
                        let n = hits_bg.fetch_add(1, Ordering::SeqCst) + 1;
                        let method = req.method().to_string();
                        let pq = req
                            .uri()
                            .path_and_query()
                            .map(|p| p.as_str().to_string())
                            .unwrap_or_default();
                        let req_bytes = req.into_body().collect().await.unwrap().to_bytes();
                        let plain = serde_json::json!({
                            "hit": n,
                            "method": method,
                            "path": pq,
                            "req": String::from_utf8_lossy(&req_bytes),
                        })
                        .to_string();
                        let mut gz =
                            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                        gz.write_all(plain.as_bytes()).unwrap();
                        let compressed = gz.finish().unwrap();
                        let resp = Response::builder()
                            .status(StatusCode::CREATED)
                            .header("content-type", "application/json")
                            .header("content-encoding", "gzip")
                            .header("x-origin-proto", "h2")
                            .body(Full::new(Bytes::from(compressed)))
                            .unwrap();
                        Ok::<_, std::convert::Infallible>(resp)
                    }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });

    // 1. Standalone composed HTTPS POST (`source.is_none()`) against pure-h2 origin
    let (tx_comp, rx_comp) = tokio::sync::oneshot::channel();
    hub.send_composed_later(
        ReplayOverride {
            method: Some("POST".into()),
            url: Some(format!(
                "https://127.0.0.1:{}/v2/compose",
                origin_addr.port()
            )),
            headers: Some(vec![("Content-Type".into(), "application/json".into())]),
            body: Some(br#"{"compose":1}"#.to_vec()),
        },
        move |res| {
            let _ = tx_comp.send(res);
        },
    );
    let composed_id = timeout(Duration::from_secs(5), rx_comp)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let composed = hub
        .with_session(|s| s.hydrated(composed_id))
        .expect("composed flow");
    assert_eq!(composed.protocol, Protocol::Http2);
    assert_eq!(composed.status, Some(201));
    assert!(composed.response_at.is_some());
    assert!(composed.finished_at.is_some());
    assert!(!composed.conn_info.server_addr.is_empty());
    assert!(!composed.conn_info.tls_version.is_empty());
    assert!(composed.conn_info.tcp_ms.is_some());
    assert!(composed.conn_info.tls_ms.is_some());
    let composed_body = String::from_utf8(composed.response_body.clone()).unwrap();
    assert!(
        composed_body.contains(r#""path":"/v2/compose""#),
        "{composed_body}"
    );
    assert!(
        composed_body.contains(r#"\"compose\":1"#),
        "{composed_body}"
    );

    // 2. Single replay of that Protocol::Http2 flow (`replay_later`)
    let (tx_rep, rx_rep) = tokio::sync::oneshot::channel();
    hub.replay_later(composed_id, ReplayOverride::default(), move |res| {
        let _ = tx_rep.send(res);
    });
    let replay_id = timeout(Duration::from_secs(5), rx_rep)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let replayed = hub
        .with_session(|s| s.hydrated(replay_id))
        .expect("replayed flow");
    assert_eq!(replayed.protocol, Protocol::Http2);
    assert_eq!(replayed.status, Some(201));
    assert!(
        replayed
            .response_headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("x-origin-proto") && v == "h2")
    );

    // 3. Batch replay of that Protocol::Http2 flow (`replay_batch_later`)
    let (tx_batch, rx_batch) = tokio::sync::oneshot::channel();
    hub.replay_batch_later(
        composed_id,
        2,
        2,
        0,
        ReplayOverride::default(),
        move |res| {
            let _ = tx_batch.send(res);
        },
    );
    let batch_ids = timeout(Duration::from_secs(5), rx_batch)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(batch_ids.len(), 2);
    for bid in batch_ids {
        let bflow = hub.with_session(|s| s.hydrated(bid)).unwrap();
        assert_eq!(bflow.protocol, Protocol::Http2);
        assert_eq!(bflow.status, Some(201));
    }

    // Verify single-use H2 handshakes NEVER entered `h2_pool`
    assert!(
        hub.h2_pool.lock().unwrap().is_empty(),
        "replay H2 connections must not enter h2_pool"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 4);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn chunked_plain_http_op_replace_json_and_overflow_passthrough_roundtrip() {
    use std::io::Write as _;

    let dir = std::env::temp_dir().join(format!("zniff-chunked-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    type CapturedReq = (Vec<(String, String)>, Vec<u8>);
    let seen_reqs: Arc<Mutex<Vec<CapturedReq>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_bg = seen_reqs.clone();
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = origin.accept().await {
            let seen_bg = seen_bg.clone();
            tokio::spawn(async move {
                let Ok((head_bytes, rest)) = read_header_block(&mut sock).await else {
                    return;
                };
                let head = parse_head(&head_bytes).unwrap();
                let mut reader = Prefixed {
                    prefix: rest,
                    pos: 0,
                    inner: &mut sock,
                };
                let body = if let Some(len) = content_length(&head.headers) {
                    read_exact(&mut reader, len).await.unwrap()
                } else if is_chunked(&head.headers) {
                    let mut out = Vec::new();
                    loop {
                        let line = read_line(&mut reader).await.unwrap();
                        let n = usize::from_str_radix(
                            String::from_utf8_lossy(&line)
                                .trim()
                                .split(';')
                                .next()
                                .unwrap(),
                            16,
                        )
                        .unwrap();
                        if n == 0 {
                            let _ = read_line(&mut reader).await.unwrap();
                            break;
                        }
                        out.extend(read_exact(&mut reader, n).await.unwrap());
                        let _ = read_exact(&mut reader, 2).await.unwrap();
                    }
                    out
                } else {
                    Vec::new()
                };
                seen_bg.lock().unwrap().push((head.headers.clone(), body));

                if head.start.contains("/big") {
                    // Stream > 1MB chunked response back
                    sock.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                    let chunk_payload = vec![b'z'; 64 * 1024];
                    let header_line = format!("{:x}\r\n", chunk_payload.len());
                    // 17 * 64KB > 1MB (16 * 64KB)
                    for _ in 0..17 {
                        sock.write_all(header_line.as_bytes()).await.unwrap();
                        sock.write_all(&chunk_payload).await.unwrap();
                        sock.write_all(b"\r\n").await.unwrap();
                    }
                    sock.write_all(b"0\r\n\r\n").await.unwrap();
                } else {
                    // Return gzipped chunked JSON response
                    let resp_plain = br#"{"status":"draft","ok":false}"#;
                    let mut gz =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                    gz.write_all(resp_plain).unwrap();
                    let gz_bytes = gz.finish().unwrap();
                    let mid = gz_bytes.len() / 2;
                    let part1 = &gz_bytes[..mid];
                    let part2 = &gz_bytes[mid..];
                    sock.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                    sock.write_all(format!("{:x}\r\n", part1.len()).as_bytes())
                        .await
                        .unwrap();
                    sock.write_all(part1).await.unwrap();
                    sock.write_all(b"\r\n").await.unwrap();
                    sock.write_all(format!("{:x}\r\n", part2.len()).as_bytes())
                        .await
                        .unwrap();
                    sock.write_all(part2).await.unwrap();
                    sock.write_all(b"\r\n0\r\n\r\n").await.unwrap();
                }
            });
        }
    });

    hub.set_rules(
        "json 127.0.0.1/graphql req set /variables/qty 99 op=MutateCart\n\
             replace 127.0.0.1/graphql req old-sku -> new-sku\n\
             replace 127.0.0.1/graphql res draft -> published\n\
             json 127.0.0.1/graphql res set /ok true\n\
             json 127.0.0.1/big req set /touch 1\n\
             json 127.0.0.1/big res set /touch 1\n",
    );

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // Case 1: Chunked <= 1MB GraphQL request matching `op=MutateCart` + `replace` + `json` on both req & gzipped chunked res
    let gql_part1 = br#"{"operationName":"MutateCart","variables":{"sku":"old-sku","qty":1},"#;
    let gql_part2 = br#""query":"mutation MutateCart { updateCart }"}"#;
    let mut client1 = TcpStream::connect(proxy_addr).await.unwrap();
    client1
            .write_all(
                format!(
                    "POST http://{origin_addr}/graphql HTTP/1.1\r\nHost: {origin_addr}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    client1
        .write_all(format!("{:x}\r\n", gql_part1.len()).as_bytes())
        .await
        .unwrap();
    client1.write_all(gql_part1).await.unwrap();
    client1.write_all(b"\r\n").await.unwrap();
    client1
        .write_all(format!("{:x}\r\n", gql_part2.len()).as_bytes())
        .await
        .unwrap();
    client1.write_all(gql_part2).await.unwrap();
    client1.write_all(b"\r\n0\r\n\r\n").await.unwrap();

    let mut raw_resp1 = Vec::new();
    client1.read_to_end(&mut raw_resp1).await.unwrap();
    let split1 = raw_resp1.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let resp_head1 = parse_head(&raw_resp1[..split1]).unwrap();
    assert!(header_value(&resp_head1.headers, "transfer-encoding").is_none());
    assert!(header_value(&resp_head1.headers, "content-encoding").is_none());
    let resp_json1: serde_json::Value = serde_json::from_slice(&raw_resp1[split1..]).unwrap();
    assert_eq!(resp_json1["status"], "published");
    assert_eq!(resp_json1["ok"], true);
    assert_eq!(
        content_length(&resp_head1.headers),
        Some(raw_resp1[split1..].len())
    );

    let (seen_headers1, seen_body1) = seen_reqs.lock().unwrap()[0].clone();
    assert!(header_value(&seen_headers1, "transfer-encoding").is_none());
    assert_eq!(content_length(&seen_headers1), Some(seen_body1.len()));
    let seen_json1: serde_json::Value = serde_json::from_slice(&seen_body1).unwrap();
    assert_eq!(seen_json1["variables"]["sku"], "new-sku");
    assert_eq!(seen_json1["variables"]["qty"], 99);

    // Case 2: Chunked > 1MB request & > 1MB response stream unmodified and record `[rule skipped: body > 1MB]`
    let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
    client2
            .write_all(
                format!(
                    "POST http://{origin_addr}/big HTTP/1.1\r\nHost: {origin_addr}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    let req_chunk = vec![b'x'; 64 * 1024];
    let req_hdr = format!("{:x}\r\n", req_chunk.len());
    for _ in 0..17 {
        client2.write_all(req_hdr.as_bytes()).await.unwrap();
        client2.write_all(&req_chunk).await.unwrap();
        client2.write_all(b"\r\n").await.unwrap();
    }
    client2.write_all(b"0\r\n\r\n").await.unwrap();

    let mut raw_resp2 = Vec::new();
    client2.read_to_end(&mut raw_resp2).await.unwrap();
    // Verify full > 1MB response arrived intact on client
    assert!(raw_resp2.len() > 17 * 64 * 1024);

    // Verify full > 1MB request arrived intact on origin
    let (_, seen_body2) = seen_reqs.lock().unwrap()[1].clone();
    assert_eq!(seen_body2.len(), 17 * 64 * 1024);

    let big_flow = hub.with_session(|s| s.get(2).cloned()).unwrap();
    assert!(
        big_flow.comment.contains("[rule skipped: body > 1MB]"),
        "expected >1MB skip note, got: {}",
        big_flow.comment
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_h2_streaming_without_content_length_applies_replace_and_json_rules() {
    struct ChunksBody(std::collections::VecDeque<Bytes>);
    impl Body for ChunksBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;
        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(self.0.pop_front().map(|b| Ok(Frame::data(b))))
        }
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-h2-stream-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules(
        "replace 127.0.0.1/stream req alpha -> beta\n\
             json 127.0.0.1/stream req set /patched true\n\
             replace 127.0.0.1/stream res pending -> done\n\
             json 127.0.0.1/stream res set /count 7\n",
    );
    let hub = Arc::new(hub);

    let server = origin_tls_config(&origin_ca, &origin_leaf, &leaf_key, vec![b"h2".to_vec()]);
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let tls = acceptor.accept(sock).await.unwrap();
        let service = hyper::service::service_fn(move |req: Request<Incoming>| async move {
            let req_bytes = req.into_body().collect().await.unwrap().to_bytes();
            let req_json: serde_json::Value = serde_json::from_slice(&req_bytes).unwrap();
            assert_eq!(req_json["tag"], "beta");
            assert_eq!(req_json["patched"], true);
            let res_chunks = std::collections::VecDeque::from([
                Bytes::from_static(br#"{"state":"pending","#),
                Bytes::from_static(br#""count":0}"#),
            ]);
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(ChunksBody(res_chunks))
                    .unwrap(),
            )
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(tls), service)
            .await;
    });

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());

    let tls = connect_mitm(proxy_addr, origin_addr, &ca, vec![b"h2".to_vec()]).await;
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, ChunksBody>(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let req_chunks = std::collections::VecDeque::from([
        Bytes::from_static(br#"{"tag":"alpha","#),
        Bytes::from_static(br#""patched":false}"#),
    ]);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/stream")
        .header("content-type", "application/json")
        .body(ChunksBody(req_chunks))
        .unwrap();
    let res = timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let res_json: serde_json::Value = serde_json::from_slice(&res_bytes).unwrap();
    assert_eq!(res_json["state"], "done");
    assert_eq!(res_json["count"], 7);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_upstream_supports_optional_scheme_and_basic_auth() {
    assert_eq!(
        parse_upstream("127.0.0.1:7890"),
        Some(Upstream {
            kind: UpstreamKind::Http,
            host: "127.0.0.1".into(),
            port: 7890,
            auth_header: None,
            socks5_auth: None,
        })
    );
    let expected_auth = format!(
        "Basic {}",
        crate::session::base64_encode(b"alice:p@ss:word")
    );
    assert_eq!(
        parse_upstream("http://alice:p%40ss:word@127.0.0.1:7890/"),
        Some(Upstream {
            kind: UpstreamKind::Http,
            host: "127.0.0.1".into(),
            port: 7890,
            auth_header: Some(expected_auth.clone()),
            socks5_auth: None,
        })
    );
    assert_eq!(
        parse_upstream("alice:p@ss:word@[::1]:7890"),
        Some(Upstream {
            kind: UpstreamKind::Http,
            host: "::1".into(),
            port: 7890,
            auth_header: Some(expected_auth),
            socks5_auth: None,
        })
    );
    assert_eq!(
        parse_upstream("socks5://127.0.0.1"),
        Some(Upstream {
            kind: UpstreamKind::Socks5 { remote_dns: false },
            host: "127.0.0.1".into(),
            port: 1080,
            auth_header: None,
            socks5_auth: None,
        })
    );
    assert_eq!(
        parse_upstream("socks5h://suser:s%40pass@127.0.0.1:1088"),
        Some(Upstream {
            kind: UpstreamKind::Socks5 { remote_dns: true },
            host: "127.0.0.1".into(),
            port: 1088,
            auth_header: None,
            socks5_auth: Some(("suser".into(), "s@pass".into())),
        })
    );
    assert_eq!(parse_upstream("ftp://127.0.0.1:21"), None);
    assert_eq!(parse_upstream(""), None);
}

#[tokio::test]
async fn proxy_auth_enforces_407_exempts_local_routes_and_chains_authenticated_upstream() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-proxy-auth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());

    // Plain HTTP origin that asserts client Proxy-Authorization was stripped.
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let seen_headers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_clone = seen_headers.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = origin.accept().await {
            let Ok((head_bytes, _)) = read_header_block(&mut sock).await else {
                break;
            };
            seen_clone
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head_bytes).into_owned());
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        }
    });

    // HTTPS origin for CONNECT chaining test.
    let (origin_ca, origin_leaf, leaf_key) = test_origin_cert();
    let tls_server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_server));
    let tls_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_origin_addr = tls_origin.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = tls_origin.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(mut tls) = acceptor.accept(sock).await {
                    let _ = read_header_block(&mut tls).await;
                    let _ = tls
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ntls-ok",
                            )
                            .await;
                }
            });
        }
    });

    // Upstream proxy requiring Basic auth `upuser:up@pass`.
    let mut upstream_hub = Hub::new(ca.clone()).unwrap();
    upstream_hub.add_trust_anchor(origin_ca.der().clone());
    upstream_hub.set_proxy_auth("upuser:up@pass");
    let upstream_hub = Arc::new(upstream_hub);
    let upstream_capture =
        Capture::spawn(upstream_hub.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
    let upstream_addr = upstream_capture.addr;

    // Downstream proxy requiring Basic auth `client:s3cr3t` and chaining to authenticated upstream.
    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_proxy_auth("client:s3cr3t");
    hub.set_upstream(&format!("http://upuser:up%40pass@{upstream_addr}"));
    let hub = Arc::new(hub);
    let capture = Capture::spawn(hub.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
    let proxy_addr = capture.addr;

    // 1. Missing Proxy-Authorization -> 407, not recorded.
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/denied HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 407"), "{text}");
        assert!(
            text.to_ascii_lowercase()
                .contains("proxy-authenticate: basic realm=\"zniff\""),
            "{text}"
        );
    }

    // 2. Wrong Proxy-Authorization on CONNECT -> 407.
    {
        let wrong = crate::session::base64_encode(b"client:wrong");
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "CONNECT {tls_origin_addr} HTTP/1.1\r\nHost: {tls_origin_addr}\r\nProxy-Authorization: Basic {wrong}\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 407"), "{text}");
    }
    assert_eq!(hub.with_session(|s| s.len()), 0);

    // 3. Local routes (/zniff-ca.crt and /zniff) remain accessible without Proxy-Authorization.
    for route in ["/zniff-ca.crt", "/zniff"] {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req =
            format!("GET {route} HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200"), "{route} -> {text}");
    }

    // 4. Valid client Proxy-Authorization on plain HTTP -> 200 via authenticated upstream,
    //    and neither client nor upstream Proxy-Authorization reaches origin or session headers.
    let valid_client = crate::session::base64_encode(b"client:s3cr3t");
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/authed HTTP/1.1\r\nHost: {origin_addr}\r\nProxy-Authorization: Basic {valid_client}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).await.unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text.ends_with("\r\n\r\nok"), "{text}");
    }
    let origin_req_head = seen_headers.lock().unwrap()[0].to_ascii_lowercase();
    assert!(
        !origin_req_head.contains("proxy-authorization"),
        "origin must never receive Proxy-Authorization: {origin_req_head}"
    );
    let recorded = hub.with_session(|s| s.get(1).cloned()).unwrap();
    assert!(
        recorded
            .request_headers
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("proxy-authorization"))
    );

    // 5. Replay of plain HTTP flow also authenticates to upstream proxy.
    let replayed_id = replay_once(hub.clone(), Some(recorded), ReplayOverride::default())
        .await
        .unwrap();
    let replayed = hub.with_session(|s| s.get(replayed_id).cloned()).unwrap();
    assert_eq!(replayed.status, Some(200));

    // 6. Valid client Proxy-Authorization on HTTPS CONNECT -> 200 tunnel + MITM via authenticated upstream.
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let connect_req = format!(
            "CONNECT {tls_origin_addr} HTTP/1.1\r\nHost: {tls_origin_addr}\r\nProxy-Authorization: Basic {valid_client}\r\n\r\n"
        );
        s.write_all(connect_req.as_bytes()).await.unwrap();
        let (head, rest) = read_header_block(&mut s).await.unwrap();
        let text = String::from_utf8_lossy(&head);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(rest.is_empty());

        let mut roots = RootCertStore::empty();
        roots.add(ca.cert_der()).unwrap();
        let mut client_cfg = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let connector = TlsConnector::from(Arc::new(client_cfg));
        let mut tls = connector
            .connect(server_name("127.0.0.1").unwrap(), s)
            .await
            .unwrap();
        let inner_req =
            format!("GET /secure HTTP/1.1\r\nHost: {tls_origin_addr}\r\nConnection: close\r\n\r\n");
        tls.write_all(inner_req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        tls.read_to_end(&mut resp).await.unwrap();
        let resp_text = String::from_utf8_lossy(&resp);
        assert!(resp_text.starts_with("HTTP/1.1 200"), "{resp_text}");
        assert!(resp_text.ends_with("\r\n\r\ntls-ok"), "{resp_text}");
    }

    capture.stop();
    upstream_capture.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn pool_evicts_single_entry_at_cap_and_capped_collect_enforces_limit() {
    let dir = std::env::temp_dir().join(format!("zniff-proxy-p0-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_task = tokio::spawn(async move {
        let mut held = Vec::new();
        while held.len() <= HTTP1_POOL_CAP {
            let Ok((s, _)) = listener.accept().await else {
                break;
            };
            held.push(s);
        }
        held
    });

    for idx in 0..HTTP1_POOL_CAP {
        let s = TcpStream::connect(addr).await.unwrap();
        hub.park_idle((format!("h{idx}.pool.test"), 80), s).await;
    }
    assert_eq!(hub.pool.lock().await.len(), HTTP1_POOL_CAP);

    let extra = TcpStream::connect(addr).await.unwrap();
    hub.park_idle(("overflow.pool.test".into(), 80), extra)
        .await;
    let pool_guard = hub.pool.lock().await;
    assert_eq!(
        pool_guard.len(),
        HTTP1_POOL_CAP,
        "pool must evict one entry instead of clearing all entries"
    );
    assert!(pool_guard.contains_key(&("overflow.pool.test".into(), 80)));
    drop(pool_guard);
    accept_task.abort();

    let small = collect_frames_capped(Full::new(Bytes::from(vec![b'a'; 64])), 64)
        .await
        .unwrap();
    assert_eq!(small.len(), 64);
    assert!(
        collect_frames_capped(Full::new(Bytes::from(vec![b'a'; 65])), 64)
            .await
            .is_err()
    );

    let (share_id, _) = hub.begin_share(Duration::from_secs(60), "");
    let flow_id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "GET".into(),
        host: "share.test".into(),
        path: "/".into(),
        url: "http://share.test/".into(),
        request_headers: Vec::new(),
    });
    hub.mark_finished(flow_id);
    let first = hub.share_export_if_changed(&share_id, None, &HashSet::new());
    let rev = match first {
        SharePoll::Ready(exp) => exp.rev,
        other => panic!("expected Ready, got {other:?}"),
    };
    assert_eq!(
        hub.share_export_if_changed(&share_id, Some(rev), &HashSet::new()),
        SharePoll::Unchanged
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn https_resolve_dials_custom_ip_preserving_sni_and_host() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-https-resolve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());

    let origin_ca_key = rcgen::KeyPair::generate().unwrap();
    let mut origin_ca_params = rcgen::CertificateParams::default();
    origin_ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let mut origin_name = rcgen::DistinguishedName::new();
    origin_name.push(rcgen::DnType::CommonName, "resolve-origin-ca");
    origin_ca_params.distinguished_name = origin_name;
    origin_ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let origin_ca = origin_ca_params.self_signed(&origin_ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&origin_ca_params, &origin_ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let mut leaf_params =
        rcgen::CertificateParams::new(vec!["api.resolve.invalid".into()]).unwrap();
    leaf_params.is_ca = rcgen::IsCa::NoCa;
    leaf_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

    let server = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let seen_sni = Arc::new(Mutex::new(None::<String>));
    let seen_req = Arc::new(Mutex::new(String::new()));
    let seen_sni_task = seen_sni.clone();
    let seen_req_task = seen_req.clone();
    tokio::spawn(async move {
        let (sock, _) = origin.accept().await.unwrap();
        let mut tls = acceptor.accept(sock).await.unwrap();
        *seen_sni_task.lock().unwrap() = tls.get_ref().1.server_name().map(str::to_string);
        let mut req_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !req_buf.ends_with(b"\r\n\r\n") {
            if tls.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            req_buf.push(byte[0]);
        }
        *seen_req_task.lock().unwrap() = String::from_utf8_lossy(&req_buf).into_owned();
        tls.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nresolve-ok",
        )
        .await
        .unwrap();
        let _ = tls.shutdown().await;
    });

    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    hub.set_rules(&format!(
        "resolve api.resolve.invalid 127.0.0.1:{}",
        origin_addr.port()
    ));
    let hub = Arc::new(hub);
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_one(proxy, hub.clone());

    let mut tcp = TcpStream::connect(proxy_addr).await.unwrap();
    tcp.write_all(
        b"CONNECT api.resolve.invalid:443 HTTP/1.1\r\nHost: api.resolve.invalid:443\r\n\r\n",
    )
    .await
    .unwrap();
    read_http_head(&mut tcp).await;
    let mut roots = RootCertStore::empty();
    roots.add(ca.cert_der()).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut tls = timeout(
        Duration::from_secs(5),
        TlsConnector::from(Arc::new(client_config)).connect(
            ServerName::try_from("api.resolve.invalid".to_string()).unwrap(),
            tcp,
        ),
    )
    .await
    .expect("客户端 TLS 超时")
    .expect("客户端 TLS");
    tls.write_all(b"GET /hello HTTP/1.1\r\nHost: api.resolve.invalid\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let resp = read_h1_message(&mut tls).await;
    let resp_text = String::from_utf8_lossy(&resp);
    assert!(resp_text.contains("resolve-ok"), "{resp_text}");
    assert_eq!(
        seen_sni.lock().unwrap().as_deref(),
        Some("api.resolve.invalid")
    );
    let req_text = seen_req.lock().unwrap().clone();
    assert!(
        req_text
            .to_ascii_lowercase()
            .contains("host: api.resolve.invalid"),
        "{req_text}"
    );

    let flow = hub.with_session(|s| s.get(1).cloned()).unwrap();
    assert_eq!(flow.host, "api.resolve.invalid");
    assert_eq!(flow.rule_hit, "resolve");
    assert_eq!(flow.conn_info.server_addr, origin_addr.to_string());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn http_resolve_switches_pooled_origin_and_bypasses_upstream() {
    let dir = std::env::temp_dir().join(format!("zniff-http-resolve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let upstream_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upstream_hits_task = upstream_hits.clone();
    let upstream_task = tokio::spawn(async move {
        while let Ok((_, _)) = upstream.accept().await {
            upstream_hits_task.fetch_add(1, Ordering::Relaxed);
        }
    });

    let origin1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr1 = origin1.local_addr().unwrap();
    let seen1 = Arc::new(Mutex::new(String::new()));
    let seen1_task = seen1.clone();
    let origin1_task = tokio::spawn(async move {
        let (mut sock, _) = origin1.accept().await.unwrap();
        let mut req_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !req_buf.ends_with(b"\r\n\r\n") {
            if sock.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            req_buf.push(byte[0]);
        }
        *seen1_task.lock().unwrap() = String::from_utf8_lossy(&req_buf).into_owned();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\norigin1")
            .await
            .unwrap();
        // Keep socket idle-open so HTTP/1.1 connection pooling parks it.
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let origin2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = origin2.local_addr().unwrap();
    let seen2 = Arc::new(Mutex::new(String::new()));
    let seen2_task = seen2.clone();
    tokio::spawn(async move {
        let (mut sock, _) = origin2.accept().await.unwrap();
        let mut req_buf = Vec::new();
        let mut byte = [0u8; 1];
        while !req_buf.ends_with(b"\r\n\r\n") {
            if sock.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            req_buf.push(byte[0]);
        }
        *seen2_task.lock().unwrap() = String::from_utf8_lossy(&req_buf).into_owned();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\norigin2")
            .await
            .unwrap();
    });

    hub.set_upstream(&format!("http://upuser:uppass@{upstream_addr}"));
    hub.set_rules(&format!(
        "resolve app.resolve.invalid 127.0.0.1:{}",
        addr1.port()
    ));
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut c1 = TcpStream::connect(proxy_addr).await.unwrap();
    c1.write_all(
        b"GET http://app.resolve.invalid/check HTTP/1.1\r\nHost: app.resolve.invalid\r\n\r\n",
    )
    .await
    .unwrap();
    let resp1 = read_h1_message(&mut c1).await;
    let text1 = String::from_utf8_lossy(&resp1);
    assert!(text1.contains("origin1"), "{text1}");
    let req1 = seen1.lock().unwrap().clone();
    assert!(req1.starts_with("GET /check HTTP/1.1\r\n"), "{req1}");
    assert!(
        req1.to_ascii_lowercase()
            .contains("host: app.resolve.invalid"),
        "{req1}"
    );
    assert!(
        !req1.to_ascii_lowercase().contains("proxy-authorization:"),
        "{req1}"
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while hub.pool.lock().await.is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !hub.pool.lock().await.is_empty(),
        "first origin connection should have parked in HTTP/1.1 pool"
    );

    hub.set_rules(&format!(
        "resolve app.resolve.invalid 127.0.0.1:{}",
        addr2.port()
    ));
    assert!(
        hub.pool.lock().await.is_empty(),
        "changing resolve rules must invalidate pooled origin connections"
    );

    let mut c2 = TcpStream::connect(proxy_addr).await.unwrap();
    c2.write_all(
            b"GET http://app.resolve.invalid/check2 HTTP/1.1\r\nHost: app.resolve.invalid\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let resp2 = read_h1_message(&mut c2).await;
    let text2 = String::from_utf8_lossy(&resp2);
    assert!(text2.contains("origin2"), "{text2}");
    let req2 = seen2.lock().unwrap().clone();
    assert!(req2.starts_with("GET /check2 HTTP/1.1\r\n"), "{req2}");
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 0);

    let flow1 = hub.with_session(|s| s.get(1).cloned()).unwrap();
    let flow2 = hub.with_session(|s| s.get(2).cloned()).unwrap();
    assert_eq!(flow1.rule_hit, "resolve");
    assert_eq!(flow1.conn_info.server_addr, addr1.to_string());
    assert_eq!(flow2.rule_hit, "resolve");
    assert_eq!(flow2.conn_info.server_addr, addr2.to_string());

    upstream_task.abort();
    origin1_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_connection_refused_marks_flow_failed() {
    let dir = std::env::temp_dir().join(format!("zniff-plain-refused-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed_listener.local_addr().unwrap();
    drop(closed_listener);

    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let hub_serve = Arc::clone(&hub);
    tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let _ = handle_conn(socket, hub_serve).await;
    });

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET http://{closed_addr}/health HTTP/1.1\r\nHost: {closed_addr}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    let _ = timeout(Duration::from_secs(3), client.read_to_end(&mut resp)).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let done = hub.with_session(|s| {
            s.get(1)
                .is_some_and(|f| f.error.is_some() && f.finished_at.is_some())
        });
        if done || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let flow = hub
        .with_session(|s| s.get(1).cloned())
        .expect("flow recorded");
    assert_eq!(flow.status, None);
    assert!(flow.finished_at.is_some());
    let err = flow.error.as_deref().unwrap_or("");
    assert!(err.contains("连接源站"), "unexpected error: {err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_json_mutation_signs_hmac_sha256_on_mutated_body() {
    let dir = std::env::temp_dir().join(format!("zniff-rule-vars-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let captured_req = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_req_clone = Arc::clone(&captured_req);
    let origin_task = tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let raw = read_h1_message(&mut sock).await;
        *captured_req_clone.lock().unwrap() = String::from_utf8_lossy(&raw).into_owned();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });

    hub.set_rules(&format!(
            "resolve api.vicanso.dev {origin_addr}\n\
             json api.vicanso.dev/v1 req set /amount 200\n\
             header api.vicanso.dev/v1 req set X-Signature {{{{hmac_sha256(\"dev-secret\", req.body)}}}}"
        ));

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let body = br#"{"amount":100}"#;
    let req = format!(
        "POST http://api.vicanso.dev/v1/pay HTTP/1.1\r\nHost: api.vicanso.dev\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    client.write_all(body).await.unwrap();
    let resp = read_h1_message(&mut client).await;
    assert!(String::from_utf8_lossy(&resp).contains("200 OK"));

    let seen = captured_req.lock().unwrap().clone();
    let (_, body_part) = seen.split_once("\r\n\r\n").expect("body separator");
    let parsed: serde_json::Value = serde_json::from_str(body_part).unwrap();
    assert_eq!(parsed["amount"], 200);

    let expected_mac = crate::rule_vars::hmac_sha256(b"dev-secret", body_part.as_bytes());
    let mut expected_hex = String::with_capacity(64);
    for b in expected_mac {
        use std::fmt::Write as _;
        let _ = write!(&mut expected_hex, "{b:02x}");
    }
    assert!(
        seen.to_ascii_lowercase()
            .contains(&format!("x-signature: {expected_hex}")),
        "missing updated HMAC signature in upstream request:\n{seen}"
    );

    origin_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plain_http_webhook_mutates_request_and_response() {
    let dir = std::env::temp_dir().join(format!("zniff-webhook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let seen_origin = Arc::new(std::sync::Mutex::new(String::new()));
    let seen_origin_clone = Arc::clone(&seen_origin);
    let origin_task = tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let raw = read_h1_message(&mut sock).await;
        *seen_origin_clone.lock().unwrap() = String::from_utf8_lossy(&raw).into_owned();
        let res_body = r#"{"cipher":"secret-raw"}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            res_body.len(),
            res_body
        );
        sock.write_all(reply.as_bytes()).await.unwrap();
    });

    let hook_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_addr = hook_listener.local_addr().unwrap();
    let hook_task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = hook_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let raw = read_h1_message(&mut sock).await;
                let (_, body) = raw
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|idx| (&raw[..idx], &raw[idx + 4..]))
                    .unwrap_or((&raw, &[]));
                let req_json: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
                let resp_json = if req_json["phase"] == "request" {
                    serde_json::json!({
                        "path": "/v2/decrypted",
                        "headers": [["X-Hooked-Req", "1"]],
                        "body_utf8": "{\"plainReq\":true}",
                        "comment": "req-hooked"
                    })
                } else {
                    serde_json::json!({
                        "status": 202,
                        "headers": {"X-Hooked-Res": "yes", "Content-Type": "application/json"},
                        "body_utf8": "{\"decryptedRes\":true}",
                        "comment": "res-hooked"
                    })
                };
                let out = resp_json.to_string();
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    out.len(),
                    out
                );
                let _ = sock.write_all(http_resp.as_bytes()).await;
            });
        }
    });

    let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener);

    hub.set_rules(&format!(
        "resolve api.vicanso.dev {origin_addr}\n\
             hook api.vicanso.dev/v1 http://{hook_addr}/hook both\n\
             hook api.vicanso.dev/v2 http://{dead_addr}/dead res timeout=300ms"
    ));

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req_body = br#"{"cipherReq":1}"#;
    let req = format!(
        "POST http://api.vicanso.dev/v1/encrypt HTTP/1.1\r\nHost: api.vicanso.dev\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        req_body.len()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    client.write_all(req_body).await.unwrap();
    let resp = read_h1_message(&mut client).await;
    let resp_text = String::from_utf8_lossy(&resp);
    assert!(resp_text.starts_with("HTTP/1.1 202 "), "{resp_text}");
    assert!(
        resp_text.to_ascii_lowercase().contains("x-hooked-res: yes"),
        "{resp_text}"
    );
    assert!(resp_text.contains("{\"decryptedRes\":true}"), "{resp_text}");

    let upstream = seen_origin.lock().unwrap().clone();
    assert!(
        upstream.starts_with("POST /v2/decrypted HTTP/1.1\r\n"),
        "{upstream}"
    );
    assert!(
        upstream.to_ascii_lowercase().contains("x-hooked-req: 1"),
        "{upstream}"
    );
    assert!(upstream.contains("{\"plainReq\":true}"), "{upstream}");

    let flow = hub
        .with_session(|s| s.get(1).cloned())
        .expect("flow recorded");
    assert!(flow.rule_hit.contains("hook"));
    assert!(flow.comment.contains("req-hooked"));
    assert!(flow.comment.contains("res-hooked"));

    // Verify fail-open behavior when a response hook points to a closed local port.
    let origin2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin2_addr = origin2.local_addr().unwrap();
    let origin2_task = tokio::spawn(async move {
        let (mut sock, _) = origin2.accept().await.unwrap();
        let _ = read_h1_message(&mut sock).await;
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .await
            .unwrap();
    });
    hub.set_rules(&format!(
        "resolve api.vicanso.dev {origin2_addr}\n\
             hook api.vicanso.dev/v2 http://{dead_addr}/dead res timeout=300ms"
    ));
    let mut client2 = TcpStream::connect(proxy_addr).await.unwrap();
    client2
            .write_all(b"GET http://api.vicanso.dev/v2/open HTTP/1.1\r\nHost: api.vicanso.dev\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    let resp2 = read_h1_message(&mut client2).await;
    let resp2_text = String::from_utf8_lossy(&resp2);
    assert!(resp2_text.starts_with("HTTP/1.1 200 OK"), "{resp2_text}");
    assert!(resp2_text.ends_with("hello"), "{resp2_text}");
    let flow2 = hub
        .with_session(|s| s.get(2).cloned())
        .expect("flow2 recorded");
    assert!(flow2.comment.contains("[hook failed:"), "{}", flow2.comment);

    origin_task.abort();
    origin2_task.abort();
    hook_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn rule_groups_stack_toggle_and_respect_workspace_precedence() {
    use crate::config::RuleGroup;

    let dir = std::env::temp_dir().join(format!("zniff-rule-groups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let group_file = dir.join("group-ping.txt");
    let ws_file = dir.join("ws-ping.txt");
    std::fs::write(&group_file, "group-ping").unwrap();
    std::fs::write(&ws_file, "ws-wins").unwrap();
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let origin_task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = origin.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let raw = read_h1_message(&mut sock).await;
                let text = String::from_utf8_lossy(&raw).into_owned();
                let body = text;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            });
        }
    });

    hub.set_rule_groups(&[
        RuleGroup {
            name: "GroupA".into(),
            enabled: true,
            rules: format!(
                "resolve api.vicanso.dev {origin_addr}\nheader api.vicanso.dev req set X-Group-A on-a"
            ),
        },
        RuleGroup {
            name: "GroupB".into(),
            enabled: true,
            rules: format!(
                "header api.vicanso.dev res set X-Group-B on-b\nmap-local api.vicanso.dev/ping {} status=201",
                group_file.display()
            ),
        },
    ]);
    assert_eq!(hub.effective_rules_count(), 4);

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let send = |path: &str| {
        let path = path.to_string();
        async move {
            let mut client = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "GET http://api.vicanso.dev{path} HTTP/1.1\r\nHost: api.vicanso.dev\r\nConnection: close\r\n\r\n"
            );
            client.write_all(req.as_bytes()).await.unwrap();
            let resp = read_h1_message(&mut client).await;
            String::from_utf8_lossy(&resp).into_owned()
        }
    };

    // 1. Both groups enabled: GroupA injects X-Group-A, GroupB injects X-Group-B
    let r1 = send("/echo").await;
    assert!(r1.contains("HTTP/1.1 200 OK"), "{r1}");
    assert!(r1.to_ascii_lowercase().contains("x-group-a: on-a"), "{r1}");
    assert!(r1.to_ascii_lowercase().contains("x-group-b: on-b"), "{r1}");

    // 2. Disable GroupB: GroupB's response header disappears while GroupA stays active
    hub.set_rule_groups(&[
        RuleGroup {
            name: "GroupA".into(),
            enabled: true,
            rules: format!(
                "resolve api.vicanso.dev {origin_addr}\nheader api.vicanso.dev req set X-Group-A on-a"
            ),
        },
        RuleGroup {
            name: "GroupB".into(),
            enabled: false,
            rules: format!(
                "header api.vicanso.dev res set X-Group-B on-b\nmap-local api.vicanso.dev/ping {} status=201",
                group_file.display()
            ),
        },
    ]);
    assert_eq!(hub.effective_rules_count(), 2);
    let r2 = send("/echo").await;
    assert!(r2.to_ascii_lowercase().contains("x-group-a: on-a"), "{r2}");
    assert!(!r2.to_ascii_lowercase().contains("x-group-b"), "{r2}");

    // 3. Re-enable GroupB, and add workspace terminal rule overriding GroupB's map-local
    hub.set_rule_groups(&[
        RuleGroup {
            name: "GroupA".into(),
            enabled: true,
            rules: format!(
                "resolve api.vicanso.dev {origin_addr}\nheader api.vicanso.dev req set X-Group-A on-a"
            ),
        },
        RuleGroup {
            name: "GroupB".into(),
            enabled: true,
            rules: format!(
                "header api.vicanso.dev res set X-Group-B on-b\nmap-local api.vicanso.dev/ping {} status=201",
                group_file.display()
            ),
        },
    ]);
    let r_group = send("/ping").await;
    assert!(r_group.starts_with("HTTP/1.1 201 "), "{r_group}");
    assert!(r_group.contains("group-ping"), "{r_group}");

    hub.set_rules(&format!(
        "map-local api.vicanso.dev/ping {} status=209",
        ws_file.display()
    ));
    let r3 = send("/ping").await;
    assert!(r3.starts_with("HTTP/1.1 209 "), "{r3}");
    assert!(r3.contains("ws-wins"), "{r3}");

    origin_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn socks5_and_socks5h_upstream_plain_http_https_and_auth() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-socks5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());

    // 1. Plain HTTP origin (records raw request head)
    let plain_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_addr = plain_origin.local_addr().unwrap();
    let seen_plain: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_plain_bg = Arc::clone(&seen_plain);
    let plain_task = tokio::spawn(async move {
        while let Ok((mut sock, _)) = plain_origin.accept().await {
            let seen_plain_bg = Arc::clone(&seen_plain_bg);
            tokio::spawn(async move {
                let raw = read_h1_message(&mut sock).await;
                seen_plain_bg
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw).into_owned());
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nhttp-via-socks5",
                    )
                    .await;
            });
        }
    });

    // 2. HTTPS origin with leaf cert for `api.vicanso.dev`
    let origin_ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let origin_ca = ca_params.self_signed(&origin_ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &origin_ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let mut leaf_params =
        rcgen::CertificateParams::new(vec!["api.vicanso.dev".to_string()]).unwrap();
    leaf_params.is_ca = rcgen::IsCa::NoCa;
    let origin_leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let tls_cfg = origin_tls_config(
        &origin_ca,
        &origin_leaf,
        &leaf_key,
        vec![b"http/1.1".to_vec()],
    );
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_cfg));
    let tls_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_addr = tls_origin.local_addr().unwrap();
    let tls_task = tokio::spawn(async move {
        while let Ok((sock, _)) = tls_origin.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(mut tls) = acceptor.accept(sock).await {
                    let _ = read_header_block(&mut tls).await;
                    let _ = tls
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\ntls-via-socks5h!",
                        )
                        .await;
                }
            });
        }
    });

    // 3. Mock SOCKS5 server recording ATYP, destination, and optional RFC 1929 auth
    type SocksConnectRecord = (u8, String, u16, bool);
    let socks_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socks_addr = socks_listener.local_addr().unwrap();
    let expected_auth: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
    let seen_connects: Arc<Mutex<Vec<SocksConnectRecord>>> = Arc::new(Mutex::new(Vec::new()));
    let expected_bg = Arc::clone(&expected_auth);
    let connects_bg = Arc::clone(&seen_connects);
    let socks_task = tokio::spawn(async move {
        while let Ok((mut client, _)) = socks_listener.accept().await {
            let expected_bg = Arc::clone(&expected_bg);
            let connects_bg = Arc::clone(&connects_bg);
            tokio::spawn(async move {
                let mut hdr = [0_u8; 2];
                if client.read_exact(&mut hdr).await.is_err() || hdr[0] != 0x05 {
                    return;
                }
                let mut methods = vec![0_u8; hdr[1] as usize];
                if client.read_exact(&mut methods).await.is_err() {
                    return;
                }
                let required = expected_bg.lock().unwrap().clone();
                let used_auth = if let Some((exp_u, exp_p)) = required {
                    if !methods.contains(&0x02) {
                        let _ = client.write_all(&[0x05, 0xFF]).await;
                        return;
                    }
                    let _ = client.write_all(&[0x05, 0x02]).await;
                    let mut u_hdr = [0_u8; 2];
                    if client.read_exact(&mut u_hdr).await.is_err() {
                        return;
                    }
                    let mut user = vec![0_u8; u_hdr[1] as usize];
                    let _ = client.read_exact(&mut user).await;
                    let mut plen = [0_u8; 1];
                    let _ = client.read_exact(&mut plen).await;
                    let mut pass = vec![0_u8; plen[0] as usize];
                    let _ = client.read_exact(&mut pass).await;
                    if user != exp_u.as_bytes() || pass != exp_p.as_bytes() {
                        let _ = client.write_all(&[0x01, 0x01]).await;
                        return;
                    }
                    let _ = client.write_all(&[0x01, 0x00]).await;
                    true
                } else {
                    let _ = client.write_all(&[0x05, 0x00]).await;
                    false
                };

                let mut req = [0_u8; 4];
                if client.read_exact(&mut req).await.is_err() || req[1] != 0x01 {
                    return;
                }
                let atyp = req[3];
                let dest = match atyp {
                    0x01 => {
                        let mut ip = [0_u8; 4];
                        let _ = client.read_exact(&mut ip).await;
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    0x03 => {
                        let mut len = [0_u8; 1];
                        let _ = client.read_exact(&mut len).await;
                        let mut domain = vec![0_u8; len[0] as usize];
                        let _ = client.read_exact(&mut domain).await;
                        String::from_utf8_lossy(&domain).into_owned()
                    }
                    _ => return,
                };
                let mut port_be = [0_u8; 2];
                if client.read_exact(&mut port_be).await.is_err() {
                    return;
                }
                let dest_port = u16::from_be_bytes(port_be);
                connects_bg
                    .lock()
                    .unwrap()
                    .push((atyp, dest.clone(), dest_port, used_auth));

                let forward_addr = if atyp == 0x03 && dest == "api.vicanso.dev" {
                    tls_addr
                } else {
                    std::net::SocketAddr::new(dest.parse().unwrap(), dest_port)
                };
                let Ok(mut upstream) = TcpStream::connect(forward_addr).await else {
                    let _ = client
                        .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                        .await;
                    return;
                };
                let _ = client
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                    .await;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });

    let mut hub = Hub::new(ca.clone()).unwrap();
    hub.add_trust_anchor(origin_ca.der().clone());
    let hub = Arc::new(hub);
    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    // Case A: socks5:// plain HTTP sends origin-form request & NO Proxy-Authorization
    hub.set_upstream(&format!("socks5://{socks_addr}"));
    let mut c1 = TcpStream::connect(proxy_addr).await.unwrap();
    let req1 = format!(
        "GET http://127.0.0.1:{}/plain HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        plain_addr.port(),
        plain_addr.port()
    );
    c1.write_all(req1.as_bytes()).await.unwrap();
    let resp1 = read_h1_message(&mut c1).await;
    let resp1_text = String::from_utf8_lossy(&resp1);
    assert!(resp1_text.contains("http-via-socks5"), "{resp1_text}");
    let plain_req = seen_plain.lock().unwrap()[0].clone();
    assert!(
        plain_req.starts_with("GET /plain HTTP/1.1\r\n"),
        "{plain_req}"
    );
    assert!(
        !plain_req
            .to_ascii_lowercase()
            .contains("proxy-authorization"),
        "{plain_req}"
    );
    assert_eq!(seen_connects.lock().unwrap()[0].0, 0x01);

    // Case B: socks5h:// with RFC 1929 auth sends ATYP=0x03 remote domain over TLS MITM
    *expected_auth.lock().unwrap() = Some(("suser".into(), "s@pass".into()));
    hub.set_upstream(&format!("socks5h://suser:s%40pass@{socks_addr}"));
    let mut c2 = TcpStream::connect(proxy_addr).await.unwrap();
    c2.write_all(b"CONNECT api.vicanso.dev:443 HTTP/1.1\r\nHost: api.vicanso.dev:443\r\n\r\n")
        .await
        .unwrap();
    let (conn_head, _) = read_header_block(&mut c2).await.unwrap();
    assert!(String::from_utf8_lossy(&conn_head).contains("200"));
    let mut roots = RootCertStore::empty();
    roots.add(hub.ca.cert_der()).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tls_connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from("api.vicanso.dev".to_string()).unwrap();
    let mut tls = tls_connector.connect(server_name, c2).await.unwrap();
    tls.write_all(b"GET /tls HTTP/1.1\r\nHost: api.vicanso.dev\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut resp2 = Vec::new();
    let _ = tls.read_to_end(&mut resp2).await;
    let resp2_text = String::from_utf8_lossy(&resp2);
    assert!(resp2_text.contains("tls-via-socks5h!"), "{resp2_text}");
    let last_conn = seen_connects.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last_conn, (0x03, "api.vicanso.dev".into(), 443, true));

    // Case C: Wrong SOCKS5 password closes connection and records authentication failure on flow
    hub.set_upstream(&format!("socks5://suser:wrong@{socks_addr}"));
    let mut c3 = TcpStream::connect(proxy_addr).await.unwrap();
    c3.write_all(req1.as_bytes()).await.unwrap();
    let mut resp3 = Vec::new();
    let _ = timeout(Duration::from_secs(5), c3.read_to_end(&mut resp3))
        .await
        .unwrap();
    assert!(
        resp3.is_empty(),
        "expected closed socket on SOCKS5 auth error"
    );
    let err3 = hub
        .with_session(|s| s.get(3).and_then(|f| f.error.clone()))
        .expect("expected failed flow error recorded");
    assert!(err3.contains("SOCKS5"), "{err3}");

    plain_task.abort();
    tls_task.abort();
    socks_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn pac_route_serves_dynamic_script_without_proxy_auth() {
    let dir = std::env::temp_dir().join(format!("zniff-pac-route-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());
    let hub = Arc::new(Hub::new(ca).unwrap());
    hub.set_proxy_auth("admin:secret");
    hub.set_bypass("*.internal.vicanso.dev, direct.vicanso.dev");

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, hub.clone());

    let mut client = TcpStream::connect(proxy_addr).await.unwrap();
    let req = format!(
        "GET /proxy.pac HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        proxy_addr.port()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let resp = read_h1_message(&mut client).await;
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(
        text.to_ascii_lowercase()
            .contains("content-type: application/x-ns-proxy-autoconfig"),
        "{text}"
    );
    assert!(text.contains("function FindProxyForURL"), "{text}");
    assert!(
        text.contains(&format!("PROXY 127.0.0.1:{}; DIRECT", proxy_addr.port())),
        "{text}"
    );
    assert!(
        text.contains("dnsDomainIs(h, \".internal.vicanso.dev\")"),
        "{text}"
    );
    assert!(text.contains("h === \"direct.vicanso.dev\""), "{text}");

    let mut head_client = TcpStream::connect(proxy_addr).await.unwrap();
    let head_req = format!(
        "HEAD /zniff.pac HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        proxy_addr.port()
    );
    head_client.write_all(head_req.as_bytes()).await.unwrap();
    let (head_bytes, rest) = read_header_block(&mut head_client).await.unwrap();
    let head_text = String::from_utf8_lossy(&head_bytes);
    assert!(head_text.starts_with("HTTP/1.1 200 OK"), "{head_text}");
    assert!(rest.is_empty());

    // /proxy.pac and /zniff.pac are untracked local routes
    assert_eq!(hub.with_session(|s| s.len()), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn allow_hosts_enforces_connect_plain_http_map_remote_replay_and_exempts_local_routes() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("zniff-allow-hosts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(Ca::open(&dir).unwrap());

    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin.local_addr().unwrap();
    let origin_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_clone = Arc::clone(&origin_hits);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = origin.accept().await {
            hits_clone.fetch_add(1, Ordering::Relaxed);
            let _ = read_header_block(&mut sock).await;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        }
    });

    let hub = Arc::new(Hub::new(ca.clone()).unwrap());
    // Strict mode with empty allow-list denies all proxied hosts while keeping local routes open.
    hub.configure_allow_hosts("", true, false);
    assert!(hub.allow_hosts_denies_all());

    let proxy = listen().await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    serve_proxy(proxy, Arc::clone(&hub));

    // Strict empty denies both CONNECT (even if in bypass) and plain HTTP without hitting origin.
    hub.set_bypass("127.0.0.1");
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!("CONNECT {origin_addr} HTTP/1.1\r\nHost: {origin_addr}\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let resp = read_h1_message(&mut s).await;
        assert!(
            String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 403 Forbidden"),
            "{}",
            String::from_utf8_lossy(&resp)
        );
    }
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/blocked HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let resp = read_h1_message(&mut s).await;
        assert!(
            String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 403 Forbidden"),
            "{}",
            String::from_utf8_lossy(&resp)
        );
    }
    assert_eq!(origin_hits.load(Ordering::Relaxed), 0);
    assert_eq!(hub.with_session(|s| s.len()), 0);
    let denied = hub.allow_denied_summary();
    assert_eq!(denied.total, 2);
    assert_eq!(denied.hosts, vec![("127.0.0.1".to_string(), 2)]);

    // Local routes remain exempt even under strict deny-all.
    for route in ["/zniff-ca.crt", "/zniff", "/proxy.pac"] {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req =
            format!("GET {route} HTTP/1.1\r\nHost: {proxy_addr}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let resp = read_h1_message(&mut s).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200 OK"), "{route}: {text}");
        if route == "/proxy.pac" {
            assert!(text.contains("return \"DIRECT\";\n}"), "{text}");
        }
    }

    // Allow allowed.vicanso.dev and 127.0.0.1; keep blocked.vicanso.dev denied.
    hub.configure_allow_hosts("127.0.0.1 allowed.vicanso.dev", true, false);
    assert!(!hub.allow_hosts_denies_all());

    // Plain HTTP to allowed 127.0.0.1 reaches origin and records flow.
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/allowed HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let resp = read_h1_message(&mut s).await;
        assert!(
            String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200 OK"),
            "{}",
            String::from_utf8_lossy(&resp)
        );
    }
    assert_eq!(origin_hits.load(Ordering::Relaxed), 1);

    // map-remote to a disallowed domain returns 403 without dialing origin again.
    hub.set_rules("map-remote 127.0.0.1/redirect http://blocked.vicanso.dev:9/target");
    {
        let mut s = TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "GET http://{origin_addr}/redirect HTTP/1.1\r\nHost: {origin_addr}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let resp = read_h1_message(&mut s).await;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 403 Forbidden"), "{text}");
        assert!(text.contains("blocked.vicanso.dev"), "{text}");
    }
    assert_eq!(origin_hits.load(Ordering::Relaxed), 1);

    // replay_once to disallowed host fails before opening a flow or dialing.
    let replay_err = replay_once(
        Arc::clone(&hub),
        None,
        ReplayOverride {
            method: Some("GET".into()),
            url: Some("http://blocked.vicanso.dev/replay".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        replay_err.to_string().contains("blocked.vicanso.dev"),
        "{replay_err:#}"
    );
    let denied_after = hub.allow_denied_summary();
    assert_eq!(denied_after.total, 4);
    assert_eq!(
        denied_after.hosts,
        vec![
            ("127.0.0.1".to_string(), 2),
            ("blocked.vicanso.dev".to_string(), 2),
        ]
    );
    hub.reset_allow_denied();
    assert_eq!(hub.allow_denied_summary().total, 0);
    assert!(hub.allow_denied_summary().hosts.is_empty());

    // Locked allow_hosts rejects runtime set_allow_hosts mutations.
    hub.configure_allow_hosts("allowed.vicanso.dev", true, true);
    assert!(hub.allow_hosts_locked());
    assert!(hub.set_allow_hosts("*").is_err());
    assert_eq!(hub.allow_hosts_text(), "allowed.vicanso.dev");

    let _ = std::fs::remove_dir_all(&dir);
}
