#[allow(unused_imports)]
use super::*;

pub(super) async fn plain_http<S>(client: &mut S, head: &Head, hub: Arc<Hub>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (method, host, port, path) = request_target(head)?;
    if !hub.host_allowed(&host) {
        hub.record_allow_denied(&host);
        return write_host_not_allowed(client, &host, method.eq_ignore_ascii_case("HEAD")).await;
    }
    let client = Prefixed {
        prefix: Vec::new(),
        pos: 0,
        inner: client,
    };
    let sanitized_headers: Vec<(String, String)> = head
        .headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("proxy-authorization"))
        .cloned()
        .collect();
    let id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: method.clone(),
        host: host.clone(),
        path: path.clone(),
        url: format!("http://{}{path}", display_authority(&host, port, "http")),
        request_headers: sanitized_headers.clone(),
    });
    let res = plain_http_inner(
        client,
        head,
        Arc::clone(&hub),
        id,
        method,
        host,
        port,
        path,
        sanitized_headers,
    )
    .await;
    if let Err(ref err) = res {
        let should_mark = hub.with_session(|s| {
            s.get(id)
                .is_some_and(|flow| flow.error.is_none() && flow.status.is_none())
        });
        if should_mark {
            hub.set_error(id, explain_flow_error(err));
        }
    }
    res
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn plain_http_inner<S>(
    mut client: Prefixed<&mut S>,
    head: &Head,
    hub: Arc<Hub>,
    id: u64,
    method: String,
    mut host: String,
    mut port: u16,
    mut path: String,
    mut headers: Vec<(String, String)>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut body_override: Option<Vec<u8>> = None;
    let mut request_taken = false;
    let mut request_recorded = false;
    let mut request_overflowed = false;
    let mut break_overrode_body = false;
    let mut strip_request_encoding = false;
    let has_expect = header_has_expect_100_continue(&head.headers);
    let websocket = header_is_websocket(&headers);
    let flow_now_ms = crate::rule_vars::VarContext::current_ms();
    let plan = if websocket {
        hub.note_resolve_hit(id, &host);
        rules::Effect::default()
    } else {
        let mut ops = crate::present::graphql_operations(&path, &[]);
        if !has_expect && hub.needs_request_body_buffer(&host, &path) {
            if let Some(len) = content_length(&head.headers) {
                if len <= crate::session::BODY_CAP && len > 0 {
                    let raw = read_exact(&mut client, len).await?;
                    request_taken = true;
                    let req_encoding = header_value(&head.headers, "content-encoding");
                    note_decoded(&hub, id, Direction::Request, &raw, req_encoding.as_deref());
                    request_recorded = true;
                    let decoded = hub.with_session(|s| {
                        s.get(id)
                            .map(|f| f.request_body.clone())
                            .unwrap_or_default()
                    });
                    ops = crate::present::graphql_operations(&path, &decoded);
                    body_override = Some(raw);
                } else if len > crate::session::BODY_CAP {
                    request_overflowed = true;
                }
            } else if is_chunked(&head.headers) {
                match read_chunked_limited(&mut client, crate::session::BODY_CAP).await? {
                    LimitedRead::Whole(raw) => {
                        request_taken = true;
                        let req_encoding = header_value(&head.headers, "content-encoding");
                        note_decoded(&hub, id, Direction::Request, &raw, req_encoding.as_deref());
                        request_recorded = true;
                        let decoded = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        ops = crate::present::graphql_operations(&path, &decoded);
                        body_override = Some(raw);
                    }
                    LimitedRead::Overflow { wire_prefix } => {
                        client.rewind(wire_prefix);
                        request_overflowed = true;
                    }
                }
            }
        }
        hub.note_rules_with_ops(id, &host, &path, &ops);
        hub.plan_with_ops(&host, &path, &ops)
    };
    if !plan.delay.is_zero() {
        tokio::time::sleep(plan.delay).await;
    }
    if !plan.request_headers.is_empty() {
        let init_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        let expanded = expand_header_ops_for_flow(
            &plan.request_headers,
            &method,
            &host,
            &path,
            &headers,
            &init_body,
            None,
            flow_now_ms,
            false,
        );
        rules::apply_header_ops(&mut headers, &expanded);
        let existing = hub.with_session(|s| {
            s.get(id)
                .map(|f| f.request_body.clone())
                .unwrap_or_default()
        });
        hub.replace_request(id, headers.clone(), existing);
    }
    let response_ops = plan.response_headers.clone();
    let cors = plan.cors;
    let throttle_up_bps = plan.throttle_up_bps;
    let throttle_down_bps = plan.throttle_down_bps;
    let break_response: Option<rules::BreakCondition> = match &plan.action {
        Some(Action::BreakResponse(cond)) => Some(cond.clone()),
        _ => None,
    };
    if cors && method.eq_ignore_ascii_case("OPTIONS") && plan.action.is_none() {
        if !request_taken {
            let _ = drain_request(&mut client, &headers, &hub, id).await;
        }
        let mut resp_headers = Vec::new();
        rules::ensure_cors(&mut resp_headers);
        let expanded_res = expand_header_ops_for_flow(
            &response_ops,
            &method,
            &host,
            &path,
            &resp_headers,
            &[],
            Some(204),
            flow_now_ms,
            false,
        );
        rules::apply_header_ops(&mut resp_headers, &expanded_res);
        write_static(&mut client, &hub, id, 204, "No Content", &resp_headers, &[]).await?;
        return Ok(());
    }
    let mut scheme = "http";
    match &plan.action {
        Some(Action::MapLocal {
            path_prefix,
            file,
            status,
            fallback,
        }) => match rules::resolve_map_local_target(file.as_deref(), path_prefix, &path) {
            Ok(resolved) => {
                if !request_taken {
                    let _ = drain_request(&mut client, &headers, &hub, id).await;
                }
                let expanded_res = expand_header_ops_for_flow(
                    &response_ops,
                    &method,
                    &host,
                    &path,
                    &[],
                    &[],
                    Some(*status),
                    flow_now_ms,
                    false,
                );
                return write_mapped_file(
                    &mut client,
                    &hub,
                    id,
                    resolved.as_deref(),
                    *status,
                    &expanded_res,
                    cors,
                    throttle_down_bps,
                )
                .await;
            }
            Err(rules::MapLocalError::NotFound(_)) if *fallback => {
                hub.append_comment(id, "map-local(fallback)");
            }
            Err(err) => {
                if !request_taken {
                    let _ = drain_request(&mut client, &headers, &hub, id).await;
                }
                let body = err.message();
                hub.set_error(id, body.clone());
                write_static(
                    &mut client,
                    &hub,
                    id,
                    502,
                    "Bad Gateway",
                    &[("Content-Type".into(), "text/plain".into())],
                    body.as_bytes(),
                )
                .await?;
                return Ok(());
            }
        },
        Some(Action::BreakRequest(cond)) => {
            let mut should_break = cond.matches_headers(&headers);
            if should_break && cond.needs_body() {
                if request_overflowed {
                    hub.append_comment(id, "[break skipped: body > 1MB]");
                    should_break = false;
                } else {
                    let decoded = current_decoded_request_body(
                        &hub,
                        id,
                        body_override.as_deref(),
                        break_overrode_body || strip_request_encoding,
                    );
                    should_break = cond.matches_body(&decoded);
                }
            }
            if should_break {
                let decision = hub.wait_break(id, BreakSide::Request).await;
                if decision.drop {
                    if !request_taken && !has_expect {
                        let _ = drain_request(&mut client, &headers, &hub, id).await;
                    }
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    let _ = client.shutdown().await;
                    return Ok(());
                }
                if let Some(next) = decision.headers {
                    let existing = hub.with_session(|s| {
                        s.get(id)
                            .map(|f| f.request_body.clone())
                            .unwrap_or_default()
                    });
                    hub.replace_request(id, next.clone(), existing);
                    headers = next;
                }
                if let Some(b) = decision.body {
                    body_override = Some(b);
                    break_overrode_body = true;
                    strip_request_encoding = true;
                }
            }
        }
        Some(Action::MapRemote { url, path_prefix }) => {
            if let Some((next_scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                if !hub.host_allowed(&next_host) {
                    hub.record_allow_denied(&next_host);
                    if !request_taken {
                        let _ = drain_request(&mut client, &headers, &hub, id).await;
                    }
                    hub.set_error(id, format!("目标域名未在允许列表中: {next_host}"));
                    hub.mark_finished(id);
                    return write_host_not_allowed(
                        &mut client,
                        &next_host,
                        method.eq_ignore_ascii_case("HEAD"),
                    )
                    .await;
                }
                scheme = if next_scheme == "https" {
                    "https"
                } else {
                    "http"
                };
                host = next_host;
                port = next_port;
                path = rules::map_remote_path(&next_path, path_prefix, &path);
                let authority = display_authority(&host, port, scheme);
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!("{scheme}://{authority}{path}"),
                );
                if let Some(pos) = headers
                    .iter()
                    .position(|(k, _)| k.eq_ignore_ascii_case("host"))
                {
                    headers[pos].1 = authority.clone();
                } else {
                    headers.push(("Host".to_string(), authority.clone()));
                }
                let existing = hub.with_session(|s| {
                    s.get(id)
                        .map(|f| f.request_body.clone())
                        .unwrap_or_default()
                });
                hub.replace_request(id, headers.clone(), existing);
            }
        }
        Some(Action::BreakResponse(_)) | None => {}
        Some(Action::Block { drop_conn, status }) => {
            if !request_taken {
                let _ = drain_request(&mut client, &headers, &hub, id).await;
            }
            if *drop_conn {
                hub.set_error(id, "规则阻断 (连接已断开)");
                hub.mark_finished(id);
                let _ = client.shutdown().await;
                return Ok(());
            }
            let mut resp_headers = vec![("Content-Length".to_string(), "0".to_string())];
            if cors {
                rules::ensure_cors(&mut resp_headers);
            }
            let expanded_res = expand_header_ops_for_flow(
                &response_ops,
                &method,
                &host,
                &path,
                &resp_headers,
                &[],
                Some(*status),
                flow_now_ms,
                false,
            );
            rules::apply_header_ops(&mut resp_headers, &expanded_res);
            write_static(
                &mut client,
                &hub,
                id,
                *status,
                reason_phrase(*status),
                &resp_headers,
                &[],
            )
            .await?;
            return Ok(());
        }
    }
    if !plan.query_ops.is_empty() {
        let cur_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        let ctx = crate::rule_vars::VarContext {
            method: &method,
            host: &host,
            target: &path,
            headers: &headers,
            body: &cur_body,
            status: None,
            now_ms: flow_now_ms,
        };
        let expanded_query = crate::rule_vars::expand_query_ops(&plan.query_ops, &ctx);
        if let Some(next) = rules::apply_query_ops(&path, &expanded_query) {
            path = next;
            hub.retarget(
                id,
                host.clone(),
                path.clone(),
                format!(
                    "{scheme}://{}{path}",
                    display_authority(&host, port, scheme)
                ),
            );
        }
    }
    if !plan.request_replaces.is_empty() {
        let cur_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        let ctx = crate::rule_vars::VarContext {
            method: &method,
            host: &host,
            target: &path,
            headers: &headers,
            body: &cur_body,
            status: None,
            now_ms: flow_now_ms,
        };
        let expanded_replaces = crate::rule_vars::expand_replaces(&plan.request_replaces, &ctx);
        if let Some(next) = rules::replace_target(&path, &expanded_replaces) {
            path = next;
            hub.retarget(
                id,
                host.clone(),
                path.clone(),
                format!(
                    "{scheme}://{}{path}",
                    display_authority(&host, port, scheme)
                ),
            );
        }
    }
    if !break_overrode_body
        && (!plan.request_replaces.is_empty() || !plan.request_json_ops.is_empty())
    {
        if has_expect && body_override.is_none() {
            if !plan.request_json_ops.is_empty() {
                hub.append_comment(
                    id,
                    "[json rule skipped: request body withheld by Expect: 100-continue]",
                );
            }
        } else if request_overflowed {
            hub.append_comment(id, "[rule skipped: body > 1MB]");
        } else if let Some(raw) = body_override.take() {
            if !raw.is_empty() {
                let encoding = header_value(&head.headers, "content-encoding");
                if let Some(next) = rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.request_replaces,
                    &plan.request_json_ops,
                    Some((&method, &host, &path, &headers, None, flow_now_ms)),
                ) {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    body_override = Some(raw);
                }
            } else {
                body_override = Some(raw);
            }
        } else if let Some(len) = content_length(&head.headers) {
            if len > crate::session::BODY_CAP {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
            } else if len > 0 {
                request_taken = true;
                let raw = read_exact(&mut client, len).await?;
                let encoding = header_value(&head.headers, "content-encoding");
                if let Some(next) = rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.request_replaces,
                    &plan.request_json_ops,
                    Some((&method, &host, &path, &headers, None, flow_now_ms)),
                ) {
                    strip_request_encoding = true;
                    body_override = Some(next);
                } else {
                    note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                    request_recorded = true;
                    body_override = Some(raw);
                }
            }
        } else if is_chunked(&head.headers) {
            match read_chunked_limited(&mut client, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => {
                    request_taken = true;
                    let encoding = header_value(&head.headers, "content-encoding");
                    if !raw.is_empty()
                        && let Some(next) = rewrite_body(
                            &hub,
                            id,
                            &raw,
                            encoding.as_deref(),
                            &plan.request_replaces,
                            &plan.request_json_ops,
                            Some((&method, &host, &path, &headers, None, flow_now_ms)),
                        )
                    {
                        strip_request_encoding = true;
                        body_override = Some(next);
                    } else {
                        if !raw.is_empty() {
                            note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                        }
                        request_recorded = true;
                        body_override = Some(raw);
                    }
                }
                LimitedRead::Overflow { wire_prefix } => {
                    client.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                }
            }
        }
    }
    if !break_overrode_body
        && !request_overflowed
        && !websocket
        && plan.hooks.iter().any(|h| h.phase.runs_request())
    {
        let authority = display_authority(&host, port, scheme);
        let cur_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        if let Some(hook_mut) = crate::webhook::run_request_hooks(
            &hub,
            id,
            &plan.hooks,
            &method,
            scheme,
            &host,
            &authority,
            &path,
            &headers,
            &cur_body,
        )
        .await
        {
            if let Some(next_path) = hook_mut.path {
                path = next_path;
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!("{scheme}://{authority}{path}"),
                );
            }
            if let Some(next_headers) = hook_mut.headers {
                headers = next_headers;
            }
            if let Some(next_body) = hook_mut.body {
                body_override = Some(next_body);
                strip_request_encoding = true;
            }
            let synced_body = current_decoded_request_body(
                &hub,
                id,
                body_override.as_deref(),
                strip_request_encoding,
            );
            hub.replace_request(id, headers.clone(), synced_body);
        }
    }
    if crate::rule_vars::header_ops_use_req_context(&plan.request_headers) {
        let signed_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        let expanded = expand_header_ops_for_flow(
            &plan.request_headers,
            &method,
            &host,
            &path,
            &headers,
            &signed_body,
            None,
            flow_now_ms,
            true,
        );
        if !expanded.is_empty() {
            rules::apply_header_ops(&mut headers, &expanded);
            let existing = if body_override.is_some()
                && (strip_request_encoding || break_overrode_body || !request_recorded)
            {
                body_override.clone().unwrap_or_default()
            } else {
                hub.with_session(|s| {
                    s.get(id)
                        .map(|f| f.request_body.clone())
                        .unwrap_or_default()
                })
            };
            hub.replace_request(id, headers.clone(), existing);
        }
    }
    if scheme == "https" {
        let body = if let Some(body) = body_override {
            headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !(strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
                    && !name.eq_ignore_ascii_case("content-length")
            });
            headers.push(("Content-Length".into(), body.len().to_string()));
            if strip_request_encoding || break_overrode_body || !request_recorded {
                hub.replace_request(id, headers.clone(), body.clone());
            }
            if !request_taken {
                let _ = discard_request(&mut client, &head.headers).await;
            }
            body
        } else if !request_taken {
            read_client_request_body(&mut client, &head.headers, has_expect, &hub, id).await?
        } else {
            Vec::new()
        };
        return exchange_tls(
            &mut client,
            &hub,
            id,
            &method,
            &host,
            port,
            &path,
            &headers,
            &body,
            &plan,
            break_response.as_ref(),
            throttle_up_bps,
            throttle_down_bps,
            flow_now_ms,
        )
        .await;
    }
    let pool_key = pool_key(&host, port, &hub);
    let (mut origin, mut conn_info) = match hub.take_idle(pool_key.clone()).await {
        Some(stream) => {
            let server_addr = stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_default();
            (
                stream,
                crate::session::ConnInfo {
                    reused: true,
                    server_addr,
                    ..Default::default()
                },
            )
        }
        None => match timeout(IO_TIMEOUT, dial_timed(&host, port, &hub, false)).await {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => return Err(err.context("连接源站")),
            Err(_) => return Err(anyhow!("连接源站超时")),
        },
    };
    origin.set_nodelay(true).ok();
    if let Some(body) = &body_override {
        headers.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("transfer-encoding")
                && !(strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
                && !name.eq_ignore_ascii_case("content-length")
        });
        headers.push(("Content-Length".into(), body.len().to_string()));
    }
    let upstream = hub.upstream_for_host(&host);
    let via_http_upstream = upstream.as_ref().is_some_and(Upstream::is_http_proxy);
    let target = if via_http_upstream {
        format!("http://{}{path}", display_authority(&host, port, "http"))
    } else {
        path.clone()
    };
    let mut outbound = format!("{method} {target} HTTP/1.1\r\n");
    let mut has_host = false;
    let mut has_connection = false;
    let mut connection_close = false;
    for (name, value) in &headers {
        if name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-authenticate")
            || (body_override.is_some()
                && (name.eq_ignore_ascii_case("content-length")
                    || name.eq_ignore_ascii_case("transfer-encoding")))
            || (strip_request_encoding && name.eq_ignore_ascii_case("content-encoding"))
            || (websocket && name.eq_ignore_ascii_case("sec-websocket-extensions"))
        {
            continue;
        }
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
            connection_close = value.eq_ignore_ascii_case("close");
        }
        outbound.push_str(name);
        outbound.push_str(": ");
        outbound.push_str(value);
        outbound.push_str("\r\n");
    }
    if !has_host {
        outbound.push_str(&format!(
            "Host: {}\r\n",
            display_authority(&host, port, "http")
        ));
    }
    if via_http_upstream
        && let Some(auth) = upstream.as_ref().and_then(|u| u.auth_header.as_deref())
    {
        outbound.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    if !has_connection {
        outbound.push_str("Connection: keep-alive\r\n");
    }
    if let Some(body) = &body_override {
        outbound.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    outbound.push_str("\r\n");
    if origin.write_all(outbound.as_bytes()).await.is_err() {
        let (next_origin, next_info) =
            match timeout(IO_TIMEOUT, dial_timed(&host, port, &hub, false)).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(err)) => return Err(err.context("连接源站")),
                Err(_) => return Err(anyhow!("连接源站超时")),
            };
        origin = next_origin;
        conn_info = next_info;
        origin.set_nodelay(true).ok();
        origin.write_all(outbound.as_bytes()).await?;
    }
    hub.set_conn_info(id, conn_info);
    let encoding = header_value(&headers, "content-encoding");
    let mut request_body_copied = false;
    if let Some(body) = &body_override {
        write_all_throttled(&mut origin, body, throttle_up_bps).await?;
        if strip_request_encoding || break_overrode_body || !request_recorded {
            hub.replace_request(id, headers.clone(), body.clone());
        }
        if !request_taken {
            let _ = discard_request(&mut client, &head.headers).await;
        }
    } else if !has_expect {
        copy_message_body(
            &mut client,
            &mut origin,
            &head.headers,
            &hub,
            id,
            Direction::Request,
            encoding.clone(),
            throttle_up_bps,
            false,
        )
        .await?;
        request_body_copied = true;
    }
    let (mut resp_bytes, mut resp_rest) = read_header_block(&mut origin)
        .await
        .context("读取源站响应")?;
    let mut resp = parse_head(&resp_bytes)?;
    let (mut status, mut reason) = status_and_reason(&resp.start);
    let mut interim_count = 0;
    while (100..=199).contains(&status) && status != 101 {
        interim_count += 1;
        if interim_count > 16 {
            anyhow::bail!("过多 1xx 中间响应");
        }
        client.write_all(&resp_bytes).await?;
        if has_expect && status == 100 && !request_body_copied && body_override.is_none() {
            copy_message_body(
                &mut client,
                &mut origin,
                &head.headers,
                &hub,
                id,
                Direction::Request,
                encoding.clone(),
                throttle_up_bps,
                false,
            )
            .await?;
            request_body_copied = true;
        }
        let (next_bytes, next_rest) = read_header_block_prefixed(&mut origin, resp_rest).await?;
        resp_bytes = next_bytes;
        resp_rest = next_rest;
        resp = parse_head(&resp_bytes)?;
        let (s, r) = status_and_reason(&resp.start);
        status = s;
        reason = r;
    }
    if websocket {
        resp_bytes = strip_ws_extension(&resp_bytes);
        resp = parse_head(&resp_bytes)?;
        let (s, r) = status_and_reason(&resp.start);
        status = s;
        reason = r;
    }
    let mut resp_headers = resp.headers.clone();
    if !websocket {
        let expanded_res = expand_header_ops_for_flow(
            &response_ops,
            &method,
            &host,
            &path,
            &resp_headers,
            &[],
            Some(status),
            flow_now_ms,
            false,
        );
        rules::apply_header_ops(&mut resp_headers, &expanded_res);
        if cors {
            rules::ensure_cors(&mut resp_headers);
        }
    }
    if websocket {
        client.write_all(&resp_bytes).await?;
    }
    hub.set_response(id, status, resp_headers.clone(), None);
    if status == 101 && header_is_websocket(&headers) {
        hub.mark_protocol(id, Protocol::WebSocket);
        if !resp_rest.is_empty() {
            client.write_all(&resp_rest).await?;
        }
        let copied = copy_with_messages(&mut client, &mut origin, &hub, id, &resp_rest).await;
        hub.mark_finished(id);
        copied?;
        return Ok(());
    }
    let has_resp_hooks = !websocket && plan.hooks.iter().any(|h| h.phase.runs_response());
    let no_resp_body = response_has_no_body(&method, status);
    let mut origin = Prefixed {
        prefix: resp_rest,
        pos: 0,
        inner: origin,
    };
    if !websocket
        && break_response.is_none()
        && (has_resp_hooks
            || (!no_resp_body
                && (!plan.response_replaces.is_empty() || !plan.response_json_ops.is_empty())))
    {
        let buffered_resp = if no_resp_body {
            Some(Vec::new())
        } else if let Some(len) = content_length(&resp_headers) {
            if len > crate::session::BODY_CAP {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
                None
            } else {
                Some(read_exact(&mut origin, len).await?)
            }
        } else if is_chunked(&resp_headers) {
            match read_chunked_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        } else {
            match read_eof_limited(&mut origin, crate::session::BODY_CAP).await? {
                LimitedRead::Whole(raw) => Some(raw),
                LimitedRead::Overflow { wire_prefix } => {
                    origin.rewind(wire_prefix);
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    None
                }
            }
        };
        if let Some(raw) = buffered_resp {
            let encoding = header_value(&resp_headers, "content-encoding");
            let mut next_body = if !raw.is_empty() {
                rewrite_body(
                    &hub,
                    id,
                    &raw,
                    encoding.as_deref(),
                    &plan.response_replaces,
                    &plan.response_json_ops,
                    Some((
                        &method,
                        &host,
                        &path,
                        &resp_headers,
                        Some(status),
                        flow_now_ms,
                    )),
                )
            } else {
                None
            };
            if has_resp_hooks {
                let flow_url = format!(
                    "{scheme}://{}{path}",
                    display_authority(&host, port, scheme)
                );
                let (mutated_body, head_changed) = apply_response_hooks(
                    &hub,
                    id,
                    &plan.hooks,
                    &method,
                    &flow_url,
                    &raw,
                    encoding.as_deref(),
                    next_body,
                    &mut status,
                    &mut resp_headers,
                )
                .await;
                next_body = mutated_body;
                if head_changed {
                    reason = reason_phrase(status).to_string();
                }
            }
            if let Some(next) = next_body {
                resp_headers.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case("content-encoding")
                        && !name.eq_ignore_ascii_case("transfer-encoding")
                        && !name.eq_ignore_ascii_case("content-length")
                });
                resp_headers.push(("Content-Length".into(), next.len().to_string()));
                hub.replace_response(id, status, resp_headers.clone(), next.clone());
                write_bytes(
                    &mut client,
                    status,
                    &reason,
                    &resp_headers,
                    &next,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            resp_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && !name.eq_ignore_ascii_case("content-length")
            });
            resp_headers.push(("Content-Length".into(), raw.len().to_string()));
            hub.replace_response_head(id, status, resp_headers.clone());
            write_bytes(
                &mut client,
                status,
                &reason,
                &resp_headers,
                &raw,
                throttle_down_bps,
            )
            .await?;
            note_decoded(&hub, id, Direction::Response, &raw, encoding.as_deref());
            hub.mark_finished(id);
            return Ok(());
        }
    }
    if let Some(ref cond) = break_response
        && cond.matches_headers(&resp_headers)
    {
        match read_capped_body(
            &mut origin,
            &resp_headers,
            crate::session::BODY_CAP,
            no_resp_body,
        )
        .await
        {
            Ok(Some(body)) => {
                let encoding = header_value(&resp_headers, "content-encoding");
                if !body.is_empty() {
                    note_decoded(&hub, id, Direction::Response, &body, encoding.as_deref());
                }
                let body_matched = if cond.needs_body() {
                    let decoded = hub.with_session(|s| {
                        s.get(id)
                            .map(|f| f.response_body.clone())
                            .unwrap_or_default()
                    });
                    cond.matches_body(&decoded)
                } else {
                    true
                };
                if !body_matched {
                    write_bytes(
                        &mut client,
                        status,
                        reason_phrase(status),
                        &resp_headers,
                        &body,
                        throttle_down_bps,
                    )
                    .await?;
                    hub.mark_finished(id);
                    return Ok(());
                }
                let decision = hub.wait_break(id, BreakSide::Response).await;
                if decision.drop {
                    hub.clear_response(id);
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    let _ = client.shutdown().await;
                    return Ok(());
                }
                let mut next_headers = decision.headers.unwrap_or(resp_headers);
                let replaced_body = decision.body.is_some();
                let next_body = decision.body.unwrap_or(body);
                if replaced_body {
                    next_headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-encoding"));
                }
                if let Some(code) = decision.status {
                    status = code;
                }
                let reason = reason_phrase(status);
                if replaced_body {
                    hub.replace_response(id, status, next_headers.clone(), next_body.clone());
                } else {
                    hub.replace_response_head(id, status, next_headers.clone());
                }
                write_bytes(
                    &mut client,
                    status,
                    reason,
                    &next_headers,
                    &next_body,
                    throttle_down_bps,
                )
                .await?;
                hub.mark_finished(id);
                return Ok(());
            }
            Ok(None) => {
                hub.set_error(id, "响应断点跳过，body 超过 1 MB");
            }
            Err(err) => {
                let text = format!("响应断点读 body 失败: {err}");
                hub.set_error(id, text.clone());
                write_static(
                    &mut client,
                    &hub,
                    id,
                    502,
                    "Bad Gateway",
                    &[("Content-Type".into(), "text/plain".into())],
                    text.as_bytes(),
                )
                .await?;
                return Ok(());
            }
        }
    }
    if !websocket {
        let head = render_response_head(status, &reason, &resp_headers);
        client.write_all(head.as_bytes()).await?;
    }
    let encoding = header_value(&resp_headers, "content-encoding");
    let reusable = copy_message_body(
        &mut origin,
        &mut client,
        &resp_headers,
        &hub,
        id,
        Direction::Response,
        encoding,
        throttle_down_bps,
        no_resp_body,
    )
    .await?;
    hub.mark_finished(id);
    let response_close = header_value(&resp.headers, "connection")
        .is_some_and(|value| value.eq_ignore_ascii_case("close"));
    let req_has_body =
        content_length(&head.headers).is_some_and(|l| l > 0) || is_chunked(&head.headers);
    let request_body_settled =
        !has_expect || !req_has_body || request_body_copied || body_override.is_some();
    if reusable
        && request_body_settled
        && !connection_close
        && !response_close
        && status != 101
        && let Some(stream) = origin.into_idle()
    {
        hub.park_idle(pool_key, stream).await;
    }
    Ok(())
}
