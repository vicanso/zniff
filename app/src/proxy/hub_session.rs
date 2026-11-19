#[allow(unused_imports)]
use super::*;

impl Hub {
    pub fn paused(&self) -> Option<(u64, BreakSide)> {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .front()
            .map(|slot| (slot.id, slot.side))
    }

    pub fn paused_queue_len(&self) -> usize {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    /// Ids and sides from front to back. The front is the flow `resolve_break` edits.
    pub fn paused_queue(&self) -> Vec<(u64, BreakSide)> {
        self.break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|slot| (slot.id, slot.side))
            .collect()
    }

    /// Moves `id` to the front and restarts its 60s timer. Other slots stay paused.
    pub fn promote_break(&self, id: u64) -> bool {
        let mut queue = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(index) = queue.iter().position(|slot| slot.id == id) else {
            return false;
        };
        if index > 0
            && let Some(slot) = queue.remove(index)
        {
            queue.push_front(slot);
        }
        if let Some(front) = queue.front_mut() {
            front.touched = Instant::now();
        }
        drop(queue);
        self.touch();
        true
    }

    #[cfg(test)]
    pub fn paused_id(&self) -> Option<u64> {
        self.paused().map(|(id, _)| id)
    }

    pub fn set_pinned(&self, id: u64, pinned: bool) -> bool {
        let ok = self.with_session(|session| session.set_pinned(id, pinned));
        if ok {
            self.touch();
        }
        ok
    }

    pub fn set_comment(&self, id: u64, comment: impl Into<String>) -> bool {
        let ok = self.with_session(|session| session.set_comment(id, comment));
        if ok {
            self.touch();
        }
        ok
    }

    pub fn append_comment(&self, id: u64, note: &str) {
        let changed = self.with_session(|session| {
            let Some(flow) = session.get(id) else {
                return false;
            };
            if flow.comment.contains(note) {
                return false;
            }
            let next = if flow.comment.is_empty() {
                note.to_string()
            } else {
                format!("{} {note}", flow.comment)
            };
            session.set_comment(id, next)
        });
        if changed {
            self.touch();
        }
    }

    pub fn resolve_break(&self, decision: BreakDecision) -> bool {
        let mut queue = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(slot) = queue.pop_front() else {
            return false;
        };
        if let Some(next) = queue.front_mut() {
            next.touched = Instant::now();
        }
        drop(queue);
        let ok = slot
            .reply
            .map(|reply| reply.send(decision).is_ok())
            .unwrap_or(false);
        self.touch();
        ok
    }

    /// Releases every paused flow in the breakpoint FIFO queue without dropping them.
    pub fn resolve_all_breaks(&self) -> usize {
        let slots: Vec<BreakSlot> = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drain(..)
            .collect();
        let count = slots.len();
        for slot in slots {
            if let Some(reply) = slot.reply {
                let _ = reply.send(BreakDecision {
                    drop: false,
                    headers: None,
                    body: None,
                    status: None,
                });
            }
        }
        if count > 0 {
            self.touch();
        }
        count
    }

    pub fn touch_break(&self) {
        if let Some(slot) = self
            .break_queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .front_mut()
        {
            slot.touched = Instant::now();
        }
    }

    pub(super) async fn wait_break(&self, id: u64, side: BreakSide) -> BreakDecision {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            let mut queue = self
                .break_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if queue.len() >= BREAK_QUEUE_CAP {
                // Evict oldest paused flow so its oneshot sender drops and auto-continues.
                let _ = queue.pop_front();
                if let Some(next) = queue.front_mut() {
                    next.touched = Instant::now();
                }
            }
            queue.push_back(BreakSlot {
                id,
                side,
                touched: Instant::now(),
                reply: Some(tx),
            });
        }
        self.touch();
        let decision = loop {
            let remaining = {
                let mut queue = self
                    .break_queue
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                match queue.iter_mut().enumerate().find(|(_, slot)| slot.id == id) {
                    Some((0, slot)) => {
                        Duration::from_secs(60).saturating_sub(slot.touched.elapsed())
                    }
                    Some((_, slot)) => {
                        slot.touched = Instant::now();
                        Duration::from_millis(200)
                    }
                    None => Duration::ZERO,
                }
            };
            if remaining.is_zero() {
                break BreakDecision {
                    drop: false,
                    headers: None,
                    body: None,
                    status: None,
                };
            }
            match timeout(remaining, &mut rx).await {
                Ok(Ok(decision)) => break decision,
                Ok(Err(_)) => {
                    break BreakDecision {
                        drop: false,
                        headers: None,
                        body: None,
                        status: None,
                    };
                }
                Err(_) => continue,
            }
        };
        {
            let mut queue = self
                .break_queue
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let was_front = queue.front().map(|slot| slot.id) == Some(id);
            queue.retain(|slot| slot.id != id);
            if was_front && let Some(next) = queue.front_mut() {
                next.touched = Instant::now();
            }
        }
        self.touch();
        decision
    }

    pub(super) fn cached_h2(
        &self,
        key: &(String, u16),
    ) -> Option<(H2Sender, u64, crate::session::ConnInfo, Arc<AtomicBool>)> {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let expired = pool
            .get(key)
            .is_some_and(|entry| entry.used.elapsed() > H2_IDLE);
        if expired {
            pool.remove(key);
            return None;
        }
        let entry = pool.get_mut(key)?;
        entry.used = Instant::now();
        Some((
            entry.sender.clone(),
            entry.generation,
            entry.conn_info.clone(),
            Arc::clone(&entry.first_used),
        ))
    }

    /// Returns the generation stored with this sender. Replacing a key does
    /// not wipe the other origins. A full pool drops every entry only when
    /// the key is new.
    pub(super) fn store_h2(
        &self,
        key: (String, u16),
        sender: H2Sender,
        conn_info: crate::session::ConnInfo,
    ) -> (u64, Arc<AtomicBool>) {
        let generation = self.h2_generation.fetch_add(1, Ordering::Relaxed);
        let first_used = Arc::new(AtomicBool::new(false));
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let replacing = pool.contains_key(&key);
        if !replacing && pool.len() >= H2_POOL_CAP {
            pool.retain(|_, v| v.used.elapsed() < H2_IDLE);
            if pool.len() >= H2_POOL_CAP
                && let Some(oldest_key) = pool
                    .iter()
                    .min_by_key(|(_, v)| v.used)
                    .map(|(k, _)| k.clone())
            {
                pool.remove(&oldest_key);
            }
        }
        pool.insert(
            key,
            PooledH2 {
                sender,
                used: Instant::now(),
                generation,
                conn_info,
                first_used: Arc::clone(&first_used),
            },
        );
        (generation, first_used)
    }

    /// Removes the entry only when it is still this generation, so an old
    /// connection task cannot delete the replacement.
    pub(super) fn drop_h2_generation(&self, key: &(String, u16), generation: u64) {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        let matches = pool
            .get(key)
            .is_some_and(|entry| entry.generation == generation);
        if matches {
            pool.remove(key);
        }
    }

    pub(super) fn touch_h2(&self, key: &(String, u16), generation: u64) {
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(entry) = pool.get_mut(key)
            && entry.generation == generation
        {
            entry.used = Instant::now();
        }
    }

    #[cfg(test)]
    pub(super) fn expire_h2_cache(&self) {
        let stale = Instant::now()
            .checked_sub(H2_IDLE + Duration::from_secs(1))
            .expect("idle stamp");
        let mut pool = self.h2_pool.lock().unwrap_or_else(|err| err.into_inner());
        for entry in pool.values_mut() {
            entry.used = stale;
        }
    }

    #[cfg(test)]
    pub(super) fn take_h2_refresh(&self) -> bool {
        self.refresh_h2.swap(false, Ordering::AcqRel)
    }

    pub(super) async fn take_idle(&self, key: (String, u16)) -> Option<TcpStream> {
        let current_epoch = self.origin_epoch.load(Ordering::Acquire);
        let mut pool = self.pool.lock().await;
        let slots = pool.get_mut(&key)?;
        while let Some((epoch, stream)) = slots.pop() {
            if epoch != current_epoch {
                continue;
            }
            let mut probe = [0u8; 1];
            match stream.try_read(&mut probe) {
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Some(stream),
                _ => continue,
            }
        }
        None
    }

    pub(super) async fn park_idle(&self, key: (String, u16), stream: TcpStream) {
        let mut probe = [0u8; 1];
        if !matches!(
            stream.try_read(&mut probe),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock
        ) {
            return;
        }
        let epoch = self.origin_epoch.load(Ordering::Acquire);
        let mut pool = self.pool.lock().await;
        if pool.len() >= HTTP1_POOL_CAP && !pool.contains_key(&key) {
            pool.retain(|_, slots| {
                slots.retain(|(ep, _)| *ep == epoch);
                !slots.is_empty()
            });
            if pool.len() >= HTTP1_POOL_CAP
                && let Some(evict_key) = pool.keys().next().cloned()
            {
                pool.remove(&evict_key);
            }
        }
        let slots = pool.entry(key).or_default();
        if slots.len() >= 4 {
            slots.remove(0);
        }
        slots.push((epoch, stream));
    }

    #[cfg(test)]
    pub fn add_trust_anchor(&mut self, der: CertificateDer<'static>) {
        let _ = self.roots.add(der);
        self.default_client_config = ClientConfig::builder()
            .with_root_certificates(self.roots.clone())
            .with_no_client_auth();
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let (out, rev_changed) = {
            let mut session = self.session.lock().unwrap_or_else(|err| err.into_inner());
            let before = session.rev();
            let out = f(&mut session);
            let after = session.rev();
            (out, after != before)
        };
        if rev_changed {
            self.state_tx.send_modify(|v| *v = v.wrapping_add(1));
        }
        out
    }

    pub fn subscribe_state(&self) -> watch::Receiver<u64> {
        self.state_tx.subscribe()
    }

    #[cfg(test)]
    pub fn state_receiver_count(&self) -> usize {
        self.state_tx.receiver_count()
    }

    pub(crate) fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.state_tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    pub(crate) fn preview_hook_body(
        &self,
        flow: &crate::session::Flow,
        is_response: bool,
    ) -> Option<crate::webhook::PreviewLookup> {
        if !self.rules_enabled() {
            return None;
        }
        let body = if is_response {
            &flow.response_body
        } else {
            &flow.request_body
        };
        if body.is_empty() {
            return None;
        }
        let ops = crate::present::graphql_operations(&flow.path, &flow.request_body);
        let specs = {
            let guard = self.rules.lock().unwrap_or_else(|err| err.into_inner());
            rules::preview_hooks_with_ops(&guard, &flow.host, &flow.path, &ops, is_response)
        };
        if specs.is_empty() {
            return None;
        }
        let rev = self.rules_revision();
        let headers = if is_response {
            flow.response_headers.clone()
        } else {
            flow.request_headers.clone()
        };
        let gen_arc = Arc::clone(&self.generation);
        let tx = self.state_tx.clone();
        Some(crate::webhook::lookup_or_spawn_preview(
            (flow.id, is_response, rev, body.len()),
            specs,
            flow.id,
            is_response,
            flow.method.clone(),
            flow.url.clone(),
            flow.status,
            headers,
            body.clone(),
            move || {
                gen_arc.fetch_add(1, Ordering::Relaxed);
                tx.send_modify(|v| *v = v.wrapping_add(1));
            },
        ))
    }

    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    pub(crate) fn decode_proto_body(
        &self,
        flow: &crate::session::Flow,
        is_response: bool,
    ) -> Option<crate::proto_schema::ProtoDecodeOutput> {
        if !self.rules_enabled() {
            return None;
        }
        let body = if is_response {
            &flow.response_body
        } else {
            &flow.request_body
        };
        if body.is_empty() {
            return None;
        }
        let specs = {
            let guard = self.rules.lock().unwrap_or_else(|err| err.into_inner());
            rules::proto_specs(&guard, &flow.host, &flow.path)
        };
        crate::proto_schema::decode_with_specs(&specs, &flow.path, body, is_response)
    }

    /// At most one list refresh per [`BODY_UI_TOUCH`] while a body is streaming.
    pub(super) fn touch_body(&self) {
        let now = body_clock_ms();
        let prev = self.body_touch_ms.load(Ordering::Relaxed);
        if now.saturating_sub(prev) < BODY_UI_TOUCH.as_millis() as u64 {
            return;
        }
        if self
            .body_touch_ms
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.touch();
        }
    }

    pub(super) fn record_chunk(
        &self,
        id: u64,
        direction: Direction,
        wire: u64,
        decoded: &[u8],
        encoding_error: Option<&str>,
    ) {
        if wire == 0 && decoded.is_empty() && encoding_error.is_none() {
            return;
        }
        self.with_session(|session| {
            if wire > 0 {
                session.add_transfer(id, direction, wire);
            }
            if !decoded.is_empty() {
                session.append_body(id, direction, decoded);
            }
            if let Some(error) = encoding_error {
                session.set_encoding_error(id, direction, error);
            }
        });
        if encoding_error.is_some() {
            self.touch();
        } else {
            self.touch_body();
        }
    }

    /// Ids that match `filter`, oldest first, with the session revision taken
    /// in the same lock as the plan. Spill reads happen in `finish`, outside the lock.
    pub fn matching_ids(&self, filter: &str) -> (Vec<(u64, Option<std::time::SystemTime>)>, u64) {
        let (plan, rev) = self.with_session(|session| (session.plan_filter(filter), session.rev()));
        (plan.finish(), rev)
    }

    pub(crate) fn open(&self, draft: FlowDraft) -> u64 {
        let ctx = current_client_ctx();
        let client_ip = ctx
            .as_ref()
            .map(|c| c.label.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .unwrap_or_default();
        let id = self.with_session(|session| {
            let id = session.open(draft);
            if !client_ip.is_empty() {
                session.set_client_ip(id, client_ip);
            }
            id
        });
        if let Some(c) = ctx {
            c.opened_ids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(id);
        }
        self.touch();
        id
    }

    pub(super) fn set_response(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        protocol: Option<Protocol>,
    ) {
        self.with_session(|session| session.set_response(id, status, headers, protocol));
        self.touch();
    }

    pub(super) fn clear_response(&self, id: u64) {
        self.with_session(|session| session.clear_response(id));
        self.touch();
    }

    pub(super) fn set_conn_info(&self, id: u64, conn_info: crate::session::ConnInfo) {
        self.with_session(|session| session.set_conn_info(id, conn_info));
        self.touch();
    }

    pub(super) fn set_error(&self, id: u64, error: impl Into<String>) {
        self.with_session(|session| session.set_error(id, error));
        self.touch();
    }

    pub(super) fn append_body(&self, id: u64, direction: Direction, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.with_session(|session| session.append_body(id, direction, data));
        self.touch_body();
    }

    pub(super) fn add_transfer(&self, id: u64, direction: Direction, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.with_session(|session| session.add_transfer(id, direction, bytes));
        self.touch_body();
    }

    pub fn mark_finished(&self, id: u64) {
        let flushed = self
            .with_session(|session| session.begin_finish(id, std::time::SystemTime::now()))
            .flush();
        if !flushed.is_empty() {
            self.with_session(|session| session.commit_finish(id, flushed));
        }
        self.touch();
    }

    pub fn clear_session(&self) {
        self.with_session(|session| session.clear());
        self.touch();
    }

    pub fn clear_unpinned(&self) -> usize {
        let removed = self.with_session(|session| session.clear_unpinned());
        if removed > 0 {
            self.touch();
        }
        removed
    }

    /// Exports the flows the list is showing. An empty filter exports the session.
    pub fn export_har_matching_with_redact(
        &self,
        filter: &str,
        redact: &crate::share::RedactOptions,
    ) -> serde_json::Value {
        let (matched, _) = self.matching_ids(filter);
        let flows = self.with_session(|session| {
            matched
                .into_iter()
                .filter_map(|(id, _)| session.hydrated(id))
                .collect::<Vec<_>>()
        });
        crate::har::export_har_with_redact(&flows, redact)
    }

    pub fn import_har(&self, value: &serde_json::Value) -> usize {
        self.import_flows(crate::har::import_har(value))
    }

    pub fn import_flows(&self, items: Vec<crate::har::Imported>) -> usize {
        let count = items.len();
        for item in items {
            let id = self.open(FlowDraft {
                protocol: item.protocol,
                method: item.method,
                host: item.host,
                path: item.path,
                url: item.url,
                request_headers: item.request_headers,
            });
            self.with_session(|session| session.set_started(id, item.started));
            if let Some(status) = item.status {
                self.set_response(id, status, item.response_headers, Some(item.protocol));
            }
            self.append_body(id, Direction::Request, &item.request_body);
            self.append_body(id, Direction::Response, &item.response_body);
            if let Some(error) = item.error {
                self.set_error(id, error);
            }
            self.mark_finished(id);
            self.with_session(|session| {
                session.restore_imported(
                    id,
                    item.response_at,
                    item.finished_at,
                    item.request_bytes,
                    item.response_bytes,
                );
                if let Some(comment) = item.comment {
                    session.set_comment(id, comment);
                }
                if !item.conn_info.is_empty() {
                    session.set_conn_info(id, item.conn_info);
                }
                for msg in item.messages {
                    session.add_message(id, msg.direction, msg.kind, msg.text);
                }
                if item.messages_truncated {
                    session.mark_messages_truncated(id);
                }
            });
            self.touch();
        }
        count
    }

    /// Replays one stored request on a fresh runtime. Map and break rules are
    /// not applied. The callback runs on the worker thread and may only touch
    /// mutexes, not nsh signals.
    pub fn replay_later<F>(self: &Arc<Self>, id: u64, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(Result<u64>) + Send + 'static,
    {
        let flow = self.with_session(|session| session.hydrated(id));
        let Some(flow) = flow else {
            on_done(Err(anyhow!("没有这条记录")));
            return;
        };
        if flow.protocol == Protocol::Tunnel || flow.method.eq_ignore_ascii_case("CONNECT") {
            on_done(Err(anyhow!("隧道不能重放")));
            return;
        }
        if flow.relay_source.is_some() {
            on_done(Err(anyhow!("中继流量只能查看")));
            return;
        }
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_once(hub, Some(flow), edit))
            })();
            on_done(result);
        });
    }

    /// Replays one stored request `count` times (with up to `concurrency` parallel
    /// workers) and `interval_ms` between launches on a fresh runtime. Map and break
    /// rules are not applied.
    pub fn replay_batch_later<F>(
        self: &Arc<Self>,
        id: u64,
        count: u32,
        concurrency: u32,
        interval_ms: u64,
        edit: ReplayOverride,
        on_done: F,
    ) where
        F: FnOnce(Result<Vec<u64>>) + Send + 'static,
    {
        let flow = self.with_session(|session| session.hydrated(id));
        let Some(flow) = flow else {
            on_done(Err(anyhow!("没有这条记录")));
            return;
        };
        if flow.protocol == Protocol::Tunnel || flow.method.eq_ignore_ascii_case("CONNECT") {
            on_done(Err(anyhow!("隧道不能重放")));
            return;
        }
        if flow.relay_source.is_some() {
            on_done(Err(anyhow!("中继流量只能查看")));
            return;
        }
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_batch(
                    hub,
                    flow,
                    count,
                    concurrency,
                    interval_ms,
                    edit,
                ))
            })();
            on_done(result);
        });
    }

    /// Sends a standalone composed request on a fresh runtime without a source
    /// flow. Map and break rules are not applied.
    pub fn send_composed_later<F>(self: &Arc<Self>, edit: ReplayOverride, on_done: F)
    where
        F: FnOnce(Result<u64>) + Send + 'static,
    {
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("重放运行时")?;
                runtime.block_on(replay_once(hub, None, edit))
            })();
            on_done(result);
        });
    }

    pub(super) fn add_message(&self, id: u64, direction: Direction, kind: &str, text: String) {
        self.with_session(|session| session.add_message(id, direction, kind, text));
        self.touch();
    }

    pub(super) fn retarget(&self, id: u64, host: String, path: String, url: String) {
        self.with_session(|session| session.retarget(id, host, path, url));
        self.touch();
    }

    pub(super) fn replace_request(&self, id: u64, headers: Vec<(String, String)>, body: Vec<u8>) {
        self.with_session(|session| session.replace_request(id, headers, body));
        self.touch();
    }

    pub(super) fn replace_response(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) {
        self.with_session(|session| session.replace_response(id, status, headers, body));
        self.touch();
    }

    pub(super) fn replace_response_head(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
    ) {
        self.with_session(|session| {
            let body = session
                .get(id)
                .map(|flow| flow.response_body.clone())
                .unwrap_or_default();
            session.replace_response(id, status, headers, body);
        });
        self.touch();
    }

    pub(super) fn set_encoding_error(
        &self,
        id: u64,
        direction: Direction,
        error: impl Into<String>,
    ) {
        self.with_session(|session| session.set_encoding_error(id, direction, error));
        self.touch();
    }

    pub(super) fn mark_protocol(&self, id: u64, protocol: Protocol) {
        self.with_session(|session| session.set_protocol(id, protocol));
        self.touch();
    }
}
