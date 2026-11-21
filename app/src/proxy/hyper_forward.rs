#[allow(unused_imports)]
use super::*;

pub(super) struct H1Job {
    pub(super) req: Request<RecordingBody<Incoming>>,
    pub(super) reply:
        tokio::sync::oneshot::Sender<Result<(Response<Incoming>, crate::session::ConnInfo)>>,
}

#[derive(Clone)]
pub(super) struct H1Client {
    pub(super) tx: tokio::sync::mpsc::UnboundedSender<H1Job>,
}

pub(super) trait HttpSender: Send {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send;
}

impl HttpSender for H1Client {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send
    {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let queued = self.tx.send(H1Job { req, reply });
        async move {
            queued.context("上游 HTTP/1.1 已经关闭")?;
            rx.await.context("上游 HTTP/1.1 没有返回")?
        }
    }
}

pub(super) struct LaneInner {
    pub(super) sender: H2Sender,
    pub(super) last_ok: Instant,
    pub(super) broken: bool,
    pub(super) generation: u64,
    pub(super) origin_epoch: u64,
    pub(super) conn_info: crate::session::ConnInfo,
    pub(super) first_used: Arc<AtomicBool>,
}

/// One browser HTTP/2 connection. The lane can drop a dead origin sender
/// and dial again without waiting for Chrome to open a new connection.
#[derive(Clone)]
pub(super) struct H2Lane {
    pub(super) lane: Arc<tokio::sync::Mutex<LaneInner>>,
    pub(super) hub: Arc<Hub>,
    pub(super) host: String,
    pub(super) port: u16,
}

impl HttpSender for H2Lane {
    fn send(
        &mut self,
        req: Request<RecordingBody<Incoming>>,
    ) -> impl std::future::Future<Output = Result<(Response<Incoming>, crate::session::ConnInfo)>> + Send
    {
        let lane = self.lane.clone();
        let hub = self.hub.clone();
        let host = self.host.clone();
        let port = self.port;
        async move {
            let mut pending = Some(req);
            for attempt in 0..2 {
                let Some(req) = pending.take() else {
                    break;
                };
                let key = (host.clone(), port);
                let (mut sender, generation, conn_info, first_used) = {
                    let mut guard = lane.lock().await;
                    let epoch_now = hub.origin_epoch.load(Ordering::Acquire);
                    let stale = guard.broken
                        || guard.origin_epoch != epoch_now
                        || guard.sender.is_closed()
                        || guard.last_ok.elapsed() > H2_IDLE;
                    #[cfg(test)]
                    let stale = stale || hub.take_h2_refresh();
                    if stale {
                        let old = guard.generation;
                        hub.drop_h2_generation(&key, old);
                        match dial_h2(&host, port, &hub).await {
                            Ok((next, generation, conn_info, first_used)) => {
                                guard.sender = next;
                                guard.generation = generation;
                                guard.origin_epoch = hub.origin_epoch.load(Ordering::Acquire);
                                guard.conn_info = conn_info;
                                guard.first_used = first_used;
                                guard.broken = false;
                                guard.last_ok = Instant::now();
                            }
                            Err(err) => {
                                guard.broken = true;
                                return Err(err.context("HTTP/2 上游"));
                            }
                        }
                    }
                    (
                        guard.sender.clone(),
                        guard.generation,
                        guard.conn_info.clone(),
                        Arc::clone(&guard.first_used),
                    )
                };
                // `try_send_request` gives the request back only when it never
                // left. Retrying after the body was written would repeat a POST.
                // A later failure only retires this generation, so another
                // stream's replacement stays in the pool.
                match sender.try_send_request(req).await {
                    Ok(response) => {
                        let info = if !first_used.swap(true, Ordering::AcqRel) {
                            conn_info
                        } else {
                            conn_info.as_reused()
                        };
                        let mut guard = lane.lock().await;
                        if guard.generation == generation {
                            guard.last_ok = Instant::now();
                            guard.broken = false;
                        }
                        drop(guard);
                        hub.touch_h2(&key, generation);
                        return Ok((response, info));
                    }
                    Err(mut err) => {
                        let mut guard = lane.lock().await;
                        if guard.generation == generation {
                            guard.broken = true;
                        }
                        drop(guard);
                        hub.drop_h2_generation(&key, generation);
                        pending = err.take_message();
                        if attempt == 0 && pending.is_some() {
                            continue;
                        }
                        return Err(anyhow::Error::from(err.into_error()).context("HTTP/2 上游"));
                    }
                }
            }
            Err(anyhow!("HTTP/2 上游"))
        }
    }
}

fn inner_request_host_from(uri: &Uri, headers: &HeaderMap, default_port: u16) -> Option<String> {
    if let Some(host) = uri.host() {
        return Some(host.to_string());
    }
    let host_hdr = headers
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    split_host_port(host_hdr, default_port)
        .ok()
        .map(|(host, _)| host)
}

fn host_not_allowed_response(
    host: &str,
    omit_body: bool,
) -> Result<Response<RecordingBody<Incoming>>> {
    let body_text = host_not_allowed_body(host);
    let headers = vec![
        (
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        ),
        ("Content-Length".to_string(), body_text.len().to_string()),
        ("Cache-Control".to_string(), "no-store".to_string()),
    ];
    let body = if omit_body {
        Vec::new()
    } else {
        body_text.into_bytes()
    };
    response_from_headers(403, &headers, &body)
}

pub(super) async fn forward_request(
    sender: &mut impl HttpSender,
    mut req: Request<Incoming>,
    hub: Arc<Hub>,
    host: &str,
    port: u16,
    scheme: &str,
    protocol: Protocol,
) -> Result<Response<RecordingBody<Incoming>>> {
    let inner_host = inner_request_host_from(req.uri(), req.headers(), port);
    let blocked_host = if !hub.host_allowed(host) {
        Some(host)
    } else {
        inner_host.as_deref().filter(|h| !hub.host_allowed(h))
    };
    if let Some(blocked) = blocked_host {
        return host_not_allowed_response(blocked, *req.method() == Method::HEAD);
    }
    let websocket = is_websocket(&req);
    let upgrade = if websocket {
        Some(hyper::upgrade::on(&mut req))
    } else {
        None
    };
    let encoding = req
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (mut parts, body) = req.into_parts();
    let mut incoming = Some(body);
    let mut path = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let mut host = host.to_string();
    let mut port = port;
    let mut scheme = scheme.to_string();
    let authority = display_authority(&host, port, &scheme);
    let id = hub.open(FlowDraft {
        protocol,
        method: parts.method.to_string(),
        host: host.clone(),
        path: path.clone(),
        url: format!("{scheme}://{authority}{path}"),
        request_headers: header_pairs(&parts.headers),
    });
    let req_method = parts.method.clone();
    let mut body_override: Option<Vec<u8>> = None;
    let mut request_prefixed: Option<Bytes> = None;
    let mut request_prefix_recorded = true;
    let mut request_recorded = false;
    let mut break_overrode_body = false;
    let mut strip_request_encoding = false;
    let mut headers_replaced = false;
    let mut mapped_remote = false;
    let mut break_response = false;
    let flow_now_ms = crate::rule_vars::VarContext::current_ms();
    let mut response_ops = Vec::new();
    let mut req_header_ops = Vec::new();
    let mut response_replaces = Vec::new();
    let mut response_json_ops = Vec::new();
    let mut req_hooks = Vec::new();
    let mut cors = false;
    let mut throttle_up_bps = None;
    let mut throttle_down_bps = None;
    if !websocket {
        let mut ops = crate::present::graphql_operations(&path, &[]);
        if hub.needs_request_body_buffer(&host, &path) {
            let known = parts
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.parse::<usize>().ok());
            let has_body = incoming.as_ref().is_some_and(|b| !b.is_end_stream());
            if has_body && known.is_none_or(|len| len <= crate::session::BODY_CAP) {
                match take_limited(
                    incoming.take().expect("request body"),
                    crate::session::BODY_CAP,
                )
                .await?
                {
                    Take::Whole(bytes) => {
                        note_decoded(&hub, id, Direction::Request, &bytes, encoding.as_deref());
                        request_recorded = true;
                        let decoded = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        ops = crate::present::graphql_operations(&path, &decoded);
                        body_override = Some(bytes.to_vec());
                    }
                    Take::Overflow { prefix, rest } => {
                        note_decoded(&hub, id, Direction::Request, &prefix, encoding.as_deref());
                        request_prefixed = Some(prefix);
                        incoming = Some(rest);
                    }
                }
            }
        }
        hub.note_rules_with_ops(id, &host, &path, &ops);
        let plan = hub.plan_with_ops(&host, &path, &ops);
        if !plan.delay.is_zero() {
            tokio::time::sleep(plan.delay).await;
        }
        response_ops = plan.response_headers.clone();
        req_header_ops = plan.request_headers.clone();
        response_replaces = plan.response_replaces.clone();
        response_json_ops = plan.response_json_ops.clone();
        req_hooks = plan.hooks.clone();
        cors = plan.cors;
        throttle_up_bps = plan.throttle_up_bps;
        throttle_down_bps = plan.throttle_down_bps;
        if !req_header_ops.is_empty() {
            let mut pairs = header_pairs(&parts.headers);
            let init_body = current_decoded_request_body(
                &hub,
                id,
                body_override.as_deref(),
                break_overrode_body || strip_request_encoding,
            );
            let expanded = expand_header_ops_for_flow(
                &req_header_ops,
                req_method.as_str(),
                &host,
                &path,
                &pairs,
                &init_body,
                None,
                flow_now_ms,
                false,
            );
            rules::apply_header_ops(&mut pairs, &expanded);
            replace_header_map(&mut parts.headers, &pairs);
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, pairs, existing);
            headers_replaced = true;
        }
        if cors && parts.method == Method::OPTIONS && plan.action.is_none() {
            if let Some(left) = incoming.take() {
                drain_incoming(left).await?;
            }
            let mut resp_headers = Vec::new();
            rules::ensure_cors(&mut resp_headers);
            let expanded_res = expand_header_ops_for_flow(
                &response_ops,
                req_method.as_str(),
                &host,
                &path,
                &resp_headers,
                &[],
                Some(204),
                flow_now_ms,
                false,
            );
            rules::apply_header_ops(&mut resp_headers, &expanded_res);
            resp_headers.push(("Content-Length".into(), "0".into()));
            return synthetic_headers(&hub, id, StatusCode::NO_CONTENT, resp_headers, Vec::new());
        }
        match &plan.action {
            Some(Action::MapLocal {
                path_prefix,
                file,
                status,
                fallback,
            }) => match rules::resolve_map_local_target(file.as_deref(), path_prefix, &path) {
                Ok(resolved) => {
                    if let Some(left) = incoming.take() {
                        drain_and_record_request(&hub, id, left, encoding.as_deref()).await?;
                    }
                    let expanded_res = expand_header_ops_for_flow(
                        &response_ops,
                        req_method.as_str(),
                        &host,
                        &path,
                        &[],
                        &[],
                        Some(*status),
                        flow_now_ms,
                        false,
                    );
                    return mapped_file_response(
                        &hub,
                        id,
                        resolved.as_deref(),
                        *status,
                        &expanded_res,
                        cors,
                    )
                    .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                }
                Err(rules::MapLocalError::NotFound(_)) if *fallback => {
                    hub.append_comment(id, "map-local(fallback)");
                }
                Err(err) => {
                    if let Some(left) = incoming.take() {
                        drain_and_record_request(&hub, id, left, encoding.as_deref()).await?;
                    }
                    let text = err.message();
                    hub.set_error(id, text.clone());
                    return synthetic_response(
                        &hub,
                        id,
                        StatusCode::BAD_GATEWAY,
                        "text/plain",
                        text.into_bytes(),
                    )
                    .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                }
            },
            Some(Action::BreakRequest) => {
                let decision = hub.wait_break(id, BreakSide::Request).await;
                if decision.drop {
                    if let Some(left) = incoming.take() {
                        let _ = drain_and_record_request(&hub, id, left, encoding.as_deref()).await;
                    }
                    hub.set_error(id, "断点丢弃");
                    hub.mark_finished(id);
                    anyhow::bail!("断点丢弃");
                }
                if let Some(next) = decision.headers {
                    replace_header_map(&mut parts.headers, &next);
                    headers_replaced = true;
                }
                if let Some(b) = decision.body {
                    body_override = Some(b);
                    break_overrode_body = true;
                    strip_request_encoding = true;
                }
            }
            Some(Action::MapRemote { url, path_prefix }) => {
                if let Some((next_scheme, next_host, next_port, next_path)) = parse_http_url(url) {
                    if !hub.host_allowed(&next_host) {
                        if let Some(left) = incoming.take() {
                            let _ =
                                drain_and_record_request(&hub, id, left, encoding.as_deref()).await;
                        }
                        hub.set_error(id, format!("目标域名未在允许列表中: {next_host}"));
                        hub.mark_finished(id);
                        return host_not_allowed_response(&next_host, req_method == Method::HEAD);
                    }
                    let same = next_scheme == scheme && next_host == host && next_port == port;
                    host = next_host;
                    port = next_port;
                    path = rules::map_remote_path(&next_path, path_prefix, &path);
                    scheme = next_scheme.to_string();
                    let authority = display_authority(&host, port, &scheme);
                    hub.retarget(
                        id,
                        host.clone(),
                        path.clone(),
                        format!("{scheme}://{authority}{path}"),
                    );
                    if let Ok(val) = http::HeaderValue::from_str(&authority) {
                        parts.headers.insert(http::header::HOST, val);
                        let pairs = header_pairs(&parts.headers);
                        let existing = hub.with_session(|s| {
                            s.get(id)
                                .map(|f| f.request_body.clone())
                                .unwrap_or_default()
                        });
                        hub.replace_request(id, pairs, existing);
                        headers_replaced = true;
                    }
                    if !same {
                        mapped_remote = true;
                    }
                }
            }
            Some(Action::BreakResponse) => break_response = true,
            Some(Action::Block { drop_conn, status }) => {
                if let Some(left) = incoming.take() {
                    let _ = drain_and_record_request(&hub, id, left, encoding.as_deref()).await;
                }
                if *drop_conn {
                    hub.set_error(id, "规则阻断 (连接已断开)");
                    hub.mark_finished(id);
                    anyhow::bail!("规则阻断 (连接已断开)");
                }
                let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::FORBIDDEN);
                let mut resp_headers = vec![("Content-Length".into(), "0".into())];
                if cors {
                    rules::ensure_cors(&mut resp_headers);
                }
                let expanded_res = expand_header_ops_for_flow(
                    &response_ops,
                    req_method.as_str(),
                    &host,
                    &path,
                    &resp_headers,
                    &[],
                    Some(*status),
                    flow_now_ms,
                    false,
                );
                rules::apply_header_ops(&mut resp_headers, &expanded_res);
                return synthetic_headers(&hub, id, code, resp_headers, Vec::new());
            }
            None => {}
        }
        if !plan.query_ops.is_empty() {
            let hdrs = header_pairs(&parts.headers);
            let cur_body = current_decoded_request_body(
                &hub,
                id,
                body_override.as_deref(),
                break_overrode_body || strip_request_encoding,
            );
            let ctx = crate::rule_vars::VarContext {
                method: req_method.as_str(),
                host: &host,
                target: &path,
                headers: &hdrs,
                body: &cur_body,
                status: None,
                now_ms: flow_now_ms,
            };
            let expanded_query = crate::rule_vars::expand_query_ops(&plan.query_ops, &ctx);
            if let Some(next) = rules::apply_query_ops(&path, &expanded_query) {
                path = next;
                let authority = display_authority(&host, port, &scheme);
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!("{scheme}://{authority}{path}"),
                );
            }
        }
        if !plan.request_replaces.is_empty() {
            let hdrs = header_pairs(&parts.headers);
            let cur_body = current_decoded_request_body(
                &hub,
                id,
                body_override.as_deref(),
                break_overrode_body || strip_request_encoding,
            );
            let ctx = crate::rule_vars::VarContext {
                method: req_method.as_str(),
                host: &host,
                target: &path,
                headers: &hdrs,
                body: &cur_body,
                status: None,
                now_ms: flow_now_ms,
            };
            let expanded_replaces = crate::rule_vars::expand_replaces(&plan.request_replaces, &ctx);
            if let Some(next) = rules::replace_target(&path, &expanded_replaces) {
                path = next;
                let authority = display_authority(&host, port, &scheme);
                hub.retarget(
                    id,
                    host.clone(),
                    path.clone(),
                    format!("{scheme}://{authority}{path}"),
                );
            }
        }
        if !break_overrode_body
            && (!plan.request_replaces.is_empty() || !plan.request_json_ops.is_empty())
        {
            let known = parts
                .headers
                .get(http::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.parse::<usize>().ok());
            if let Some(taken) = body_override.take() {
                if !taken.is_empty() {
                    let req_hdrs = header_pairs(&parts.headers);
                    if let Some(next) = rewrite_body(
                        &hub,
                        id,
                        &taken,
                        encoding.as_deref(),
                        &plan.request_replaces,
                        &plan.request_json_ops,
                        Some((
                            req_method.as_str(),
                            &host,
                            &path,
                            &req_hdrs,
                            None,
                            flow_now_ms,
                        )),
                    ) {
                        strip_request_encoding = true;
                        body_override = Some(next);
                    } else {
                        body_override = Some(taken);
                    }
                } else {
                    body_override = Some(taken);
                }
            } else if request_prefixed.is_some()
                || known.is_some_and(|len| len > crate::session::BODY_CAP)
            {
                hub.append_comment(id, "[rule skipped: body > 1MB]");
            } else if incoming.as_ref().is_some_and(|b| !b.is_end_stream()) && known != Some(0) {
                match take_limited(
                    incoming.take().expect("request body"),
                    crate::session::BODY_CAP,
                )
                .await?
                {
                    Take::Whole(raw) => {
                        let req_hdrs = header_pairs(&parts.headers);
                        if !raw.is_empty()
                            && let Some(next) = rewrite_body(
                                &hub,
                                id,
                                &raw,
                                encoding.as_deref(),
                                &plan.request_replaces,
                                &plan.request_json_ops,
                                Some((
                                    req_method.as_str(),
                                    &host,
                                    &path,
                                    &req_hdrs,
                                    None,
                                    flow_now_ms,
                                )),
                            )
                        {
                            strip_request_encoding = true;
                            body_override = Some(next);
                        } else {
                            note_decoded(&hub, id, Direction::Request, &raw, encoding.as_deref());
                            request_recorded = true;
                            body_override = Some(raw.to_vec());
                        }
                    }
                    Take::Overflow { prefix, rest } => {
                        hub.append_comment(id, "[rule skipped: body > 1MB]");
                        request_prefixed = Some(prefix);
                        request_prefix_recorded = false;
                        incoming = Some(rest);
                    }
                }
            }
        }
    } else if hub.resolve_for_host(&host).is_some() {
        hub.note_resolve_hit(id, &host);
    }
    if mapped_remote
        && body_override.is_none()
        && request_prefixed.is_none()
        && !websocket
        && !parts.headers.contains_key(http::header::CONTENT_LENGTH)
        && let Some(left) = incoming.take()
    {
        let raw = collect_request_bytes_capped(
            &hub,
            id,
            left,
            encoding.as_deref(),
            MAX_BUFFERED_REQUEST_BODY,
        )
        .await?;
        request_recorded = true;
        body_override = Some(raw.to_vec());
    }
    if !break_overrode_body
        && request_prefixed.is_none()
        && !websocket
        && req_hooks.iter().any(|h| h.phase.runs_request())
    {
        let authority = display_authority(&host, port, &scheme);
        let mut pairs = header_pairs(&parts.headers);
        let cur_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        if let Some(hook_mut) = crate::webhook::run_request_hooks(
            &hub,
            id,
            &req_hooks,
            req_method.as_str(),
            &scheme,
            &host,
            &authority,
            &path,
            &pairs,
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
                pairs = next_headers;
                replace_header_map(&mut parts.headers, &pairs);
                headers_replaced = true;
            }
            if let Some(next_body) = hook_mut.body {
                body_override = Some(next_body);
                strip_request_encoding = true;
            }
            if headers_replaced || strip_request_encoding {
                let synced = current_decoded_request_body(
                    &hub,
                    id,
                    body_override.as_deref(),
                    strip_request_encoding,
                );
                hub.replace_request(id, pairs, synced);
            }
        }
    }
    if crate::rule_vars::header_ops_use_req_context(&req_header_ops) {
        let mut pairs = header_pairs(&parts.headers);
        let signed_body = current_decoded_request_body(
            &hub,
            id,
            body_override.as_deref(),
            break_overrode_body || strip_request_encoding,
        );
        let expanded = expand_header_ops_for_flow(
            &req_header_ops,
            req_method.as_str(),
            &host,
            &path,
            &pairs,
            &signed_body,
            None,
            flow_now_ms,
            true,
        );
        if !expanded.is_empty() {
            rules::apply_header_ops(&mut pairs, &expanded);
            replace_header_map(&mut parts.headers, &pairs);
            headers_replaced = true;
        }
    }
    let outbound_body = if let Some(bytes) = body_override {
        parts.headers.remove(http::header::CONTENT_LENGTH);
        parts.headers.remove(http::header::TRANSFER_ENCODING);
        if strip_request_encoding {
            parts.headers.remove(http::header::CONTENT_ENCODING);
        }
        if let Ok(value) = http::HeaderValue::from_str(&bytes.len().to_string()) {
            parts.headers.insert(http::header::CONTENT_LENGTH, value);
        }
        if let Some(left) = incoming.take() {
            drain_incoming(left).await?;
        }
        if strip_request_encoding || break_overrode_body || !request_recorded {
            hub.replace_request(id, header_pairs(&parts.headers), bytes.clone());
        } else if headers_replaced {
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, header_pairs(&parts.headers), existing);
        }
        RecordingBody::from_bytes(Bytes::from(bytes)).with_throttle(throttle_up_bps)
    } else if let Some(prefix) = request_prefixed {
        if headers_replaced {
            let existing = hub.with_session(|s| {
                s.get(id)
                    .map(|f| f.request_body.clone())
                    .unwrap_or_default()
            });
            hub.replace_request(id, header_pairs(&parts.headers), existing);
        }
        let body = if request_prefix_recorded {
            RecordingBody::prefixed(
                prefix,
                incoming.take().expect("request body"),
                hub.clone(),
                id,
                Direction::Request,
                encoding.as_deref(),
            )
        } else {
            RecordingBody::stream_with_prefix(
                prefix,
                incoming.take().expect("request body"),
                hub.clone(),
                id,
                Direction::Request,
                encoding.as_deref(),
            )
        };
        body.with_throttle(throttle_up_bps)
    } else if websocket {
        RecordingBody::passthrough(incoming.take().expect("request body"))
    } else {
        if headers_replaced {
            hub.replace_request(id, header_pairs(&parts.headers), Vec::new());
        }
        RecordingBody::new(
            incoming.take().expect("request body"),
            hub.clone(),
            id,
            Direction::Request,
            encoding.as_deref(),
        )
        .with_throttle(throttle_up_bps)
    };
    let send_result = if mapped_remote {
        fetch_remote(
            &hub,
            req_method.clone(),
            &scheme,
            &host,
            port,
            &path,
            &parts.headers,
            outbound_body,
            websocket,
        )
        .await
    } else {
        parts.version = match protocol {
            Protocol::Http2 => Version::HTTP_2,
            _ => Version::HTTP_11,
        };
        let authority = display_authority(&host, port, &scheme);
        parts.uri = absolute_uri(&scheme, &authority, &path)?;
        strip_hop_by_hop(&mut parts.headers, websocket);
        if websocket {
            parts
                .headers
                .remove(HeaderName::from_static("sec-websocket-extensions"));
        }
        let outbound = Request::from_parts(parts, outbound_body);
        sender.send(outbound).await
    };
    let mut response = match send_result {
        Ok((response, conn_info)) => {
            hub.set_conn_info(id, conn_info);
            response
        }
        Err(err) => {
            // The HTTP/2 lane already dropped its own generation. A blanket
            // drop here would remove the replacement it just opened.
            hub.set_error(id, explain_flow_error(&err));
            return Err(err);
        }
    };
    if websocket && response.status() == StatusCode::SWITCHING_PROTOCOLS {
        hub.mark_protocol(id, Protocol::WebSocket);
        if let Some(upgrade) = upgrade {
            let server_up = hyper::upgrade::on(&mut response);
            let hub_up = hub.clone();
            tokio::spawn(async move {
                splice_upgrades(upgrade, server_up, hub_up, id).await;
            });
        }
    }
    let resp_encoding = response
        .headers()
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (mut parts, body) = response.into_parts();
    if websocket {
        parts
            .headers
            .remove(HeaderName::from_static("sec-websocket-extensions"));
    } else if !response_ops.is_empty() || cors {
        let mut pairs = header_pairs(&parts.headers);
        let expanded_res = expand_header_ops_for_flow(
            &response_ops,
            req_method.as_str(),
            &host,
            &path,
            &pairs,
            &[],
            Some(parts.status.as_u16()),
            flow_now_ms,
            false,
        );
        rules::apply_header_ops(&mut pairs, &expanded_res);
        if cors {
            rules::ensure_cors(&mut pairs);
        }
        replace_header_map(&mut parts.headers, &pairs);
    }
    let has_resp_hooks = !websocket && req_hooks.iter().any(|h| h.phase.runs_response());
    let no_resp_body = response_has_no_body(req_method.as_str(), parts.status.as_u16());
    if !websocket
        && !break_response
        && (has_resp_hooks
            || (!no_resp_body && (!response_replaces.is_empty() || !response_json_ops.is_empty())))
    {
        let known = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.parse::<usize>().ok());
        if known.is_some_and(|len| len > crate::session::BODY_CAP) {
            hub.append_comment(id, "[rule skipped: body > 1MB]");
        } else if no_resp_body || body.is_end_stream() || known == Some(0) {
            if has_resp_hooks {
                let mut status_code = parts.status.as_u16();
                let mut resp_pairs = header_pairs(&parts.headers);
                let flow_url = format!(
                    "{scheme}://{}{path}",
                    display_authority(&host, port, &scheme)
                );
                let (mutated_body, head_changed) = apply_response_hooks(
                    &hub,
                    id,
                    &req_hooks,
                    req_method.as_str(),
                    &flow_url,
                    &[],
                    resp_encoding.as_deref(),
                    None,
                    &mut status_code,
                    &mut resp_pairs,
                )
                .await;
                if mutated_body.is_some() || head_changed {
                    let next_bytes = mutated_body.unwrap_or_default();
                    let next_status = StatusCode::from_u16(status_code).unwrap_or(parts.status);
                    resp_pairs.retain(|(name, _)| {
                        !name.eq_ignore_ascii_case("content-encoding")
                            && !name.eq_ignore_ascii_case("transfer-encoding")
                            && !name.eq_ignore_ascii_case("content-length")
                    });
                    if !no_resp_body || !next_bytes.is_empty() {
                        resp_pairs.push(("Content-Length".into(), next_bytes.len().to_string()));
                    }
                    return synthetic_headers(&hub, id, next_status, resp_pairs, next_bytes)
                        .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                }
            }
        } else {
            match take_limited(body, crate::session::BODY_CAP).await? {
                Take::Whole(raw) => {
                    let mut status_code = parts.status.as_u16();
                    let mut resp_pairs = header_pairs(&parts.headers);
                    let mut next_body = if !raw.is_empty() {
                        rewrite_body(
                            &hub,
                            id,
                            &raw,
                            resp_encoding.as_deref(),
                            &response_replaces,
                            &response_json_ops,
                            Some((
                                req_method.as_str(),
                                &host,
                                &path,
                                &resp_pairs,
                                Some(status_code),
                                flow_now_ms,
                            )),
                        )
                    } else {
                        None
                    };
                    let mut head_changed = false;
                    if has_resp_hooks {
                        let flow_url = format!(
                            "{scheme}://{}{path}",
                            display_authority(&host, port, &scheme)
                        );
                        let (mutated_body, changed) = apply_response_hooks(
                            &hub,
                            id,
                            &req_hooks,
                            req_method.as_str(),
                            &flow_url,
                            &raw,
                            resp_encoding.as_deref(),
                            next_body,
                            &mut status_code,
                            &mut resp_pairs,
                        )
                        .await;
                        next_body = mutated_body;
                        head_changed = changed;
                    }
                    if let Some(next) = next_body {
                        if head_changed {
                            parts.status =
                                StatusCode::from_u16(status_code).unwrap_or(parts.status);
                            replace_header_map(&mut parts.headers, &resp_pairs);
                        }
                        parts.headers.remove(http::header::CONTENT_ENCODING);
                        parts.headers.remove(http::header::TRANSFER_ENCODING);
                        parts.headers.remove(http::header::CONTENT_LENGTH);
                        if let Ok(value) = http::HeaderValue::from_str(&next.len().to_string()) {
                            parts.headers.insert(http::header::CONTENT_LENGTH, value);
                        }
                        return synthetic_headers(
                            &hub,
                            id,
                            parts.status,
                            header_pairs(&parts.headers),
                            next,
                        )
                        .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
                    }
                    if head_changed {
                        parts.status = StatusCode::from_u16(status_code).unwrap_or(parts.status);
                        replace_header_map(&mut parts.headers, &resp_pairs);
                    }
                    parts.headers.remove(http::header::TRANSFER_ENCODING);
                    parts.headers.remove(http::header::CONTENT_LENGTH);
                    if let Ok(value) = http::HeaderValue::from_str(&raw.len().to_string()) {
                        parts.headers.insert(http::header::CONTENT_LENGTH, value);
                    }
                    hub.set_response(
                        id,
                        parts.status.as_u16(),
                        header_pairs(&parts.headers),
                        None,
                    );
                    note_decoded(
                        &hub,
                        id,
                        Direction::Response,
                        &raw,
                        resp_encoding.as_deref(),
                    );
                    hub.mark_finished(id);
                    return Ok(Response::from_parts(
                        parts,
                        RecordingBody::from_bytes(raw).with_throttle(throttle_down_bps),
                    ));
                }
                Take::Overflow { prefix, rest } => {
                    hub.append_comment(id, "[rule skipped: body > 1MB]");
                    hub.set_response(
                        id,
                        parts.status.as_u16(),
                        header_pairs(&parts.headers),
                        None,
                    );
                    let body = RecordingBody::stream_with_prefix(
                        prefix,
                        rest,
                        hub,
                        id,
                        Direction::Response,
                        resp_encoding.as_deref(),
                    )
                    .with_throttle(throttle_down_bps);
                    return Ok(Response::from_parts(parts, body));
                }
            }
        }
    }
    if break_response && !websocket {
        return hold_hyper_response(hub, id, parts, body, resp_encoding)
            .await
            .map(|res| res.map(|b| b.with_throttle(throttle_down_bps)));
    }
    hub.set_response(
        id,
        parts.status.as_u16(),
        header_pairs(&parts.headers),
        None,
    );
    let body = if websocket {
        RecordingBody::passthrough(body)
    } else {
        RecordingBody::new(body, hub, id, Direction::Response, resp_encoding.as_deref())
            .with_throttle(throttle_down_bps)
    };
    Ok(Response::from_parts(parts, body))
}
