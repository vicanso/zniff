#[allow(unused_imports)]
use super::*;

pub async fn replay_batch(
    hub: Arc<Hub>,
    source: crate::session::Flow,
    count: u32,
    concurrency: u32,
    interval_ms: u64,
    edit: ReplayOverride,
) -> Result<Vec<u64>> {
    let total = count.clamp(
        crate::present::REPEAT_COUNT_MIN,
        crate::present::REPEAT_COUNT_MAX,
    );
    let workers = concurrency
        .clamp(
            crate::present::REPEAT_CONCURRENCY_MIN,
            crate::present::REPEAT_CONCURRENCY_MAX,
        )
        .min(total);
    let delay_ms = interval_ms.min(crate::present::REPEAT_INTERVAL_MAX_MS);
    let source_id = source.id;
    let base_comment = source.comment.clone();
    if workers <= 1 {
        let mut ids = Vec::with_capacity(total as usize);
        for idx in 0..total {
            if idx > 0 && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let (replayed_id, _sent) =
                replay_once_attempt(Arc::clone(&hub), Some(source.clone()), edit.clone()).await?;
            hub.set_comment(
                replayed_id,
                repeat_tag_comment(source_id, &base_comment, idx + 1, total),
            );
            ids.push(replayed_id);
        }
        return Ok(ids);
    }

    let sem = Arc::new(tokio::sync::Semaphore::new(workers as usize));
    let mut join_set = tokio::task::JoinSet::new();
    for idx in 0..total {
        if idx > 0 && delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let permit = Arc::clone(&sem)
            .acquire_owned()
            .await
            .context("并发信号量")?;
        let hub_task = Arc::clone(&hub);
        let source_task = source.clone();
        let edit_task = edit.clone();
        let base_comment_task = base_comment.clone();
        join_set.spawn(async move {
            let _permit = permit;
            let (replayed_id, _sent) =
                replay_once_attempt(Arc::clone(&hub_task), Some(source_task), edit_task).await?;
            hub_task.set_comment(
                replayed_id,
                repeat_tag_comment(source_id, &base_comment_task, idx + 1, total),
            );
            Ok::<(u32, u64), anyhow::Error>((idx, replayed_id))
        });
    }
    let mut pairs = Vec::with_capacity(total as usize);
    while let Some(res) = join_set.join_next().await {
        let pair = res.context("批量重放任务")??;
        pairs.push(pair);
    }
    pairs.sort_by_key(|(idx, _)| *idx);
    Ok(pairs.into_iter().map(|(_, id)| id).collect())
}

pub(super) fn repeat_tag_comment(
    source_id: u64,
    base_comment: &str,
    index: u32,
    total: u32,
) -> String {
    let tag = format!("repeat:{source_id} ({index}/{total})");
    let trimmed = base_comment.trim();
    if trimmed.is_empty() {
        tag
    } else {
        format!("{trimmed} · {tag}")
    }
}

pub(crate) async fn replay_once(
    hub: Arc<Hub>,
    source: Option<crate::session::Flow>,
    edit: ReplayOverride,
) -> Result<u64> {
    let (id, sent) = replay_once_attempt(hub, source, edit).await?;
    sent?;
    Ok(id)
}

pub(super) async fn replay_once_attempt(
    hub: Arc<Hub>,
    source: Option<crate::session::Flow>,
    edit: ReplayOverride,
) -> Result<(u64, Result<()>)> {
    let custom_method = edit.method.unwrap_or_default();
    let custom_method = custom_method.trim();
    let method = if custom_method.is_empty() {
        source
            .as_ref()
            .map(|flow| flow.method.clone())
            .unwrap_or_else(|| "GET".to_string())
    } else {
        custom_method.to_string()
    };
    let custom_url = edit.url.unwrap_or_default();
    let custom_url = custom_url.trim().to_string();
    let url_overridden = !custom_url.is_empty();
    let url_for_parse: &str = if url_overridden {
        &custom_url
    } else if let Some(flow) = source.as_ref() {
        &flow.url
    } else {
        ""
    };
    let Some((scheme, host, port, path)) = parse_http_url(url_for_parse) else {
        anyhow::bail!(
            "{}",
            if url_overridden || source.is_none() {
                "编辑后的 URL 不能重放"
            } else {
                "这条记录没有可重放的 URL"
            }
        );
    };
    let mut headers = edit.headers.unwrap_or_else(|| {
        source
            .as_ref()
            .map(|flow| flow.request_headers.clone())
            .unwrap_or_default()
    });
    if url_overridden || source.is_none() {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("host"));
    }
    let body = edit.body.unwrap_or_else(|| {
        source
            .as_ref()
            .map(|flow| flow.request_body.clone())
            .unwrap_or_default()
    });
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: method.clone(),
        host: host.clone(),
        path: path.clone(),
        url: format!(
            "{scheme}://{}{path}",
            display_authority(&host, port, scheme)
        ),
        request_headers: headers.clone(),
    });
    if let Some(flow) = source.as_ref()
        && !flow.client_ip.is_empty()
    {
        hub.with_session(|session| session.set_client_ip(id, flow.client_ip.clone()));
    }
    let prefer_h2 = url_overridden
        || matches!(
            source.as_ref().map(|flow| flow.protocol),
            None | Some(Protocol::Http2)
        );
    hub.replace_request(id, headers.clone(), body.clone());
    let sent = if scheme == "https" {
        replay_tls(
            &hub, id, &method, &host, port, &path, &headers, &body, prefer_h2,
        )
        .await
    } else {
        replay_http(&hub, id, &method, &host, port, &path, &headers, &body).await
    };
    if let Err(err) = &sent {
        hub.set_error(id, explain_flow_error(err));
    }
    Ok((id, sent))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn replay_http(
    hub: &Hub,
    id: u64,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()> {
    let (mut origin, conn_info) = timeout(IO_TIMEOUT, dial_timed(host, port, hub, false))
        .await
        .context("连接源站超时")??;
    hub.set_conn_info(id, conn_info);
    let target = if hub.upstream_for_host(host).is_some() {
        format!("http://{}{path}", display_authority(host, port, "http"))
    } else {
        path.to_string()
    };
    write_replay(
        hub,
        id,
        &mut origin,
        method,
        host,
        port,
        "http",
        &target,
        headers,
        body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn replay_tls(
    hub: &Hub,
    id: u64,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    prefer_h2: bool,
) -> Result<()> {
    let alpn: &[Vec<u8>] = if prefer_h2 {
        &[b"h2".to_vec(), b"http/1.1".to_vec()]
    } else {
        &[b"http/1.1".to_vec()]
    };
    let (origin, selected, conn_info) = connect_origin(host, port, alpn, hub).await?;
    hub.set_conn_info(id, conn_info);
    if selected == b"h2" {
        return replay_h2(hub, id, origin, method, host, port, path, headers, body).await;
    }
    write_replay(
        hub, id, origin, method, host, port, "https", path, headers, body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn replay_h2<S>(
    hub: &Hub,
    id: u64,
    origin: S,
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    hub.mark_protocol(id, Protocol::Http2);
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake::<_, RecordingBody<Incoming>>(TokioIo::new(origin))
        .await
        .context("上游 HTTP/2")?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::debug!("重放 HTTP/2 连接结束: {err}");
        }
    });
    let req_method = Method::from_bytes(method.as_bytes()).context("重放 HTTP 方法无效")?;
    let authority = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case(":authority"))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| display_authority(host, port, "https"));
    let uri = absolute_uri("https", &authority, path)?;
    let mut header_map = HeaderMap::new();
    let mut had_content_length = false;
    for (name, value) in headers {
        if name.starts_with(':') || name.eq_ignore_ascii_case("host") {
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            had_content_length = true;
            continue;
        }
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("keep-alive")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || (name.eq_ignore_ascii_case("te") && !value.trim().eq_ignore_ascii_case("trailers"))
        {
            continue;
        }
        let Ok(hname) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(hval) = http::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        header_map.append(hname, hval);
    }
    if (!body.is_empty() || had_content_length)
        && let Ok(val) = http::HeaderValue::from_str(&body.len().to_string())
    {
        header_map.insert(http::header::CONTENT_LENGTH, val);
    }
    let mut outbound = Request::builder()
        .version(Version::HTTP_2)
        .method(req_method)
        .uri(uri)
        .body(RecordingBody::from_bytes(Bytes::copy_from_slice(body)))
        .context("组装上游 URI")?;
    *outbound.headers_mut() = header_map;
    if !body.is_empty() {
        hub.add_transfer(id, Direction::Request, body.len() as u64);
    }
    sender.ready().await.context("HTTP/2 上游")?;
    let response = timeout(IO_TIMEOUT, sender.send_request(outbound))
        .await
        .context("连接源站超时")?
        .context("HTTP/2 上游")?;
    let status = response.status().as_u16();
    let resp_headers = header_pairs(response.headers());
    let resp_encoding = header_value(&resp_headers, "content-encoding");
    hub.set_response(id, status, resp_headers, Some(Protocol::Http2));
    if !response_has_no_body(method, status) {
        let mut resp_body = response.into_body();
        let mut decoder = Decoder::from_header(resp_encoding.as_deref());
        if let Some(error) = decoder.error() {
            hub.set_encoding_error(id, Direction::Response, error.to_string());
        }
        while let Some(frame) = resp_body.frame().await {
            let frame = frame.context("读取响应体")?;
            if let Ok(data) = frame.into_data()
                && !data.is_empty()
            {
                let decoded = decoder.push(&data);
                let error = decoder.error().map(|err| err.to_string());
                hub.record_chunk(
                    id,
                    Direction::Response,
                    data.len() as u64,
                    &decoded,
                    error.as_deref(),
                );
            }
        }
        let tail = decoder.finish();
        let error = decoder.error().map(|err| err.to_string());
        hub.record_chunk(id, Direction::Response, 0, &tail, error.as_deref());
    }
    hub.mark_finished(id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn write_replay<S>(
    hub: &Hub,
    id: u64,
    mut origin: S,
    method: &str,
    host: &str,
    port: u16,
    scheme: &str,
    target: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut outbound = format!("{method} {target} HTTP/1.1\r\n");
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        outbound.push_str(name);
        outbound.push_str(": ");
        outbound.push_str(value);
        outbound.push_str("\r\n");
    }
    if !has_host {
        outbound.push_str(&format!(
            "Host: {}\r\n",
            display_authority(host, port, scheme)
        ));
    }
    if scheme == "http"
        && let Some(auth) = hub.upstream_for_host(host).and_then(|u| u.auth_header)
    {
        outbound.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    outbound.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    origin.write_all(outbound.as_bytes()).await?;
    origin.write_all(body).await?;
    if !body.is_empty() {
        hub.add_transfer(id, Direction::Request, body.len() as u64);
    }
    let (mut resp_bytes, mut resp_rest) = read_header_block(&mut origin).await?;
    let mut resp = parse_head(&resp_bytes)?;
    let (mut status, _) = status_and_reason(&resp.start);
    let mut interim_count = 0;
    while (100..=199).contains(&status) && status != 101 {
        interim_count += 1;
        if interim_count > 16 {
            anyhow::bail!("过多 1xx 中间响应");
        }
        let (next_bytes, next_rest) = read_header_block_prefixed(&mut origin, resp_rest).await?;
        resp_bytes = next_bytes;
        resp_rest = next_rest;
        resp = parse_head(&resp_bytes)?;
        status = status_and_reason(&resp.start).0;
    }
    hub.set_response(id, status, resp.headers.clone(), None);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    let mut sink = tokio::io::sink();
    let _ = copy_message_body(
        &mut origin,
        &mut sink,
        &resp.headers,
        hub,
        id,
        Direction::Response,
        header_value(&resp.headers, "content-encoding"),
        None,
        response_has_no_body(method, status),
    )
    .await?;
    hub.mark_finished(id);
    Ok(())
}
