#[allow(unused_imports)]
use super::*;

pub(crate) fn clear_captured(
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    shown: Shown,
) {
    controls.hub.clear_session();
    selected.set(None);
    baseline.set(None);
    reset_compose(compose);
    close_sequence_diagram(seq_builder);
    clear_shown(shown);
    crate::session::trim_memory();
}

pub(crate) fn clear_unpinned_captured(
    controls: &Controls,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    compose: Compose,
    seq_builder: SeqBuilder,
    shown: Shown,
) {
    controls.hub.clear_unpinned();
    if let Some(id) = selected.get()
        && controls
            .hub
            .with_session(|session| session.get(id).is_none())
    {
        selected.set(None);
        reset_compose(compose);
        clear_shown(shown);
    }
    if let Some(id) = baseline.get()
        && controls
            .hub
            .with_session(|session| session.get(id).is_none())
    {
        baseline.set(None);
    }
    if seq_builder.open.get() {
        let remaining_empty = controls.hub.with_session(|session| {
            seq_builder
                .items
                .update(|items| items.retain(|item| session.get(item.id).is_some()));
            seq_builder.items.with(Vec::is_empty)
        });
        if remaining_empty {
            close_sequence_diagram(seq_builder);
        } else {
            refresh_seq_diagram(controls, seq_builder);
        }
    }
    crate::session::trim_memory();
}

pub(crate) fn save_flow_comment(controls: &Controls, id: Option<u64>, shown: Shown) {
    let Some(id) = id else {
        shown.editing_comment.set(false);
        return;
    };
    let clean = shown.comment.get().trim().to_string();
    shown.comment.set(clean.clone());
    shown.editing_comment.set(false);
    let _ = controls.hub.set_comment(id, &clean);
}

/// Next id in the filtered list. `delta` is `-1` for up and `1` for down.
/// An empty list stays unselected. With nothing selected, down takes the
/// first row and up the last. Either end clamps. An id that is not in the
/// list is treated as no selection.
pub(crate) fn neighbor_by<T>(
    items: &[T],
    id_of: impl Fn(&T) -> u64,
    selected: Option<u64>,
    delta: i32,
) -> Option<u64> {
    if items.is_empty() {
        return None;
    }
    let end_id = || {
        if delta < 0 {
            id_of(&items[items.len() - 1])
        } else {
            id_of(&items[0])
        }
    };
    let Some(current) = selected else {
        return Some(end_id());
    };
    let Some(index) = items.iter().position(|item| id_of(item) == current) else {
        return Some(end_id());
    };
    let last = items.len() as i64 - 1;
    let next = (index as i64 + i64::from(delta)).clamp(0, last) as usize;
    Some(id_of(&items[next]))
}

#[cfg(test)]
pub(crate) fn neighbor(ids: &[u64], selected: Option<u64>, delta: i32) -> Option<u64> {
    neighbor_by(ids, |id| *id, selected, delta)
}

pub(crate) fn step_selected(
    rows: Signal<Vec<Arc<RowLine>>>,
    selected: Signal<Option<u64>>,
    compose: Compose,
    delta: i32,
) {
    let next = rows.with(|rows| neighbor_by(rows, |row| row.id, selected.get(), delta));
    if selected.get() != next {
        reset_compose(compose);
        selected.set(next);
    }
}

pub(crate) fn replay_flow(controls: &Arc<Controls>, id: Option<u64>, edit: ReplayOverride) {
    let hub = controls.hub.clone();
    let Some(id) = id else {
        if edit.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            let controls = Arc::clone(controls);
            hub.send_composed_later(edit, move |result| match result {
                Ok(new_id) => set_status(&controls, StatusMsg::Replayed(new_id)),
                Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
            });
        } else {
            set_status(controls, StatusMsg::NeedSelection);
        }
        return;
    };
    if refuse_relay(controls, Some(id)) {
        return;
    }
    let controls = Arc::clone(controls);
    hub.replay_later(id, edit, move |result| match result {
        Ok(new_id) => set_status(&controls, StatusMsg::Replayed(new_id)),
        Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
    });
}

pub(crate) fn repeat_flow_batch(controls: &Arc<Controls>, id: Option<u64>, shown: Shown) {
    let Some(id) = id else {
        shown.repeating.set(false);
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    let count = present::parse_repeat_count(&shown.repeat_count.get());
    let interval_ms = present::parse_repeat_interval_ms(&shown.repeat_interval.get());
    let concurrency = present::parse_repeat_concurrency(&shown.repeat_concurrency.get());
    shown.repeat_count.set(count.to_string());
    shown.repeat_interval.set(interval_ms.to_string());
    shown.repeat_concurrency.set(concurrency.to_string());
    shown.repeating.set(false);
    if refuse_relay(controls, Some(id)) {
        return;
    }
    let hub = controls.hub.clone();
    let controls = Arc::clone(controls);
    hub.replay_batch_later(
        id,
        count,
        concurrency,
        interval_ms,
        ReplayOverride::default(),
        move |result| match result {
            Ok(ids) => set_status(
                &controls,
                StatusMsg::Repeated {
                    id,
                    count: ids.len() as u32,
                },
            ),
            Err(err) => set_status(&controls, StatusMsg::ReplayFailed(format!("{err:#}"))),
        },
    );
}

pub(crate) fn parse_header_text(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn export_har_file(controls: Arc<Controls>, filter: &str) {
    let redact = redact_options(&controls);
    let value = controls
        .hub
        .export_har_matching_with_redact(filter, &redact);
    let bytes = match serde_json::to_vec_pretty(&value) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
            return;
        }
    };
    let secs = clock::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("zniff-{secs}.har"))
        .filter(nsh_core::file_dialog::FileFilter::new("HAR", ["har"]));
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, &bytes) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::ExportCancelled),
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn export_response_web(controls: Arc<Controls>, id: Option<u64>) {
    let Some(id) = id else {
        set_status(&controls, StatusMsg::NeedSelection);
        return;
    };
    let rel = format!("api/flows/{id}/body");
    spawn_detached(async move {
        let Some(url) = proxy::resolve_web_api_url(&rel).await else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
            return;
        };
        if open_url(url).await {
            set_status(&controls, StatusMsg::Saved(format!("flow-{id}")));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn export_har_file(controls: Arc<Controls>, filter: &str) {
    let redact_on = lock_bool(&controls.redact_export);
    let redact_strict = lock_bool(&controls.redact_strict);
    let redact_fields = lock_string(&controls.redact_fields);
    let rel = format!(
        "api/export.har?filter={}&redact={}&redact_strict={}&redact_fields={}",
        url_encode_query_component(filter.trim()),
        if redact_on { "1" } else { "0" },
        if redact_strict { "1" } else { "0" },
        url_encode_query_component(&redact_fields),
    );
    spawn_detached(async move {
        let Some(url) = proxy::resolve_web_api_url(&rel).await else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
            return;
        };
        if open_url(url).await {
            set_status(&controls, StatusMsg::Exported("zniff.har".into()));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn export_html_file(controls: Arc<Controls>, filter: &str, zh: bool) {
    let (matched, _) = controls.hub.matching_ids(filter);
    let flows = controls.hub.with_session(|session| {
        matched
            .into_iter()
            .filter_map(|(id, _)| session.hydrated(id))
            .collect::<Vec<_>>()
    });
    let redact = redact_options(&controls);
    let json = share::document_with_redact(&flows, clock::now(), filter, &redact);
    let html = share::page(&json, "", zh);
    let secs = clock::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("zniff-{secs}.html"))
        .filter(nsh_core::file_dialog::FileFilter::new(
            "HTML",
            ["html", "htm"],
        ));
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, html.as_bytes()) {
                    set_status(&controls, StatusMsg::ExportFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Exported(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::ExportCancelled),
            Err(err) => set_status(&controls, StatusMsg::ExportFailed(err)),
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn export_html_file(controls: Arc<Controls>, filter: &str, zh: bool) {
    let redact_on = lock_bool(&controls.redact_export);
    let redact_strict = lock_bool(&controls.redact_strict);
    let redact_fields = lock_string(&controls.redact_fields);
    let rel = format!(
        "api/export.html?filter={}&redact={}&redact_strict={}&redact_fields={}&lang={}",
        url_encode_query_component(filter.trim()),
        if redact_on { "1" } else { "0" },
        if redact_strict { "1" } else { "0" },
        url_encode_query_component(&redact_fields),
        if zh { "zh" } else { "en" },
    );
    spawn_detached(async move {
        let Some(url) = proxy::resolve_web_api_url(&rel).await else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
            return;
        };
        if open_url(url).await {
            set_status(&controls, StatusMsg::Exported("zniff.html".into()));
        } else {
            set_status(
                &controls,
                StatusMsg::ExportFailed("open_url blocked".into()),
            );
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn begin_share(
    controls: &Arc<Controls>,
    minutes: u64,
    filter: &str,
    share_id: Signal<String>,
) {
    if !lock_bool(&controls.running) {
        set_status(controls, StatusMsg::ShareNeedsCapture);
        return;
    }
    let (local_base, local_only) = share_base(controls);
    let relay_url = lock_string(&controls.relay_url);
    let relay_token = lock_string(&controls.relay_token);
    let route = match relay::share_route(&relay_url, &relay_token, &local_base, local_only) {
        Ok(route) => route,
        Err(relay::ShareRouteError::BadUrl) => {
            set_status(controls, StatusMsg::RelayBadUrl);
            return;
        }
        Err(relay::ShareRouteError::EmptyToken) => {
            set_status(controls, StatusMsg::RelayNeedToken);
            return;
        }
    };
    stop_relay_push(controls);
    let redact = redact_options(controls);
    let (id, _expires) = controls.hub.begin_share_with_redact(
        Duration::from_secs(minutes.saturating_mul(60)),
        filter,
        redact,
    );
    let (url, via_relay, local_only) = match route {
        relay::ShareRoute::Local { base, local_only } => {
            (format!("{base}/s/{id}"), false, local_only)
        }
        relay::ShareRoute::Relay(base) => {
            let url = base.share_url(&id);
            let status_controls = Arc::clone(controls);
            let shown = url.clone();
            let handle = relay::spawn_push(
                Arc::clone(&controls.hub),
                id.clone(),
                base,
                relay_token.trim().to_string(),
                move |event| match event {
                    relay::PushEvent::Failed(err) => {
                        set_status(&status_controls, StatusMsg::RelayPushFailed(err));
                    }
                    relay::PushEvent::Connected => {
                        set_status(&status_controls, StatusMsg::RelayConnected(shown.clone()));
                    }
                },
            );
            relay::remember_push(handle.clone());
            *controls
                .relay_push
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = Some(handle);
            (url, true, false)
        }
    };
    share_id.set(id);
    set_clipboard(url.clone());
    set_status(
        controls,
        StatusMsg::Shared {
            url,
            minutes,
            local_only,
            relay: via_relay,
        },
    );
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn stop_relay_push(controls: &Controls) {
    if let Some(handle) = controls
        .relay_push
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        handle.stop();
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn begin_share(
    controls: &Arc<Controls>,
    minutes: u64,
    filter: &str,
    share_id: Signal<String>,
) {
    if !lock_bool(&controls.running) {
        set_status(controls, StatusMsg::ShareNeedsCapture);
        return;
    }
    let redact_opts = share::RedactOptions::new(
        lock_bool(&controls.redact_export),
        &lock_string(&controls.redact_fields),
    )
    .with_strict(lock_bool(&controls.redact_strict));
    let body = serde_json::json!({
        "op": "begin_share",
        "minutes": minutes,
        "filter": filter,
        "redact": redact_opts.enabled,
        "redact_strict": redact_opts.strict,
        "redact_fields": redact_opts.to_wire_fields(),
        "relay_url": lock_string(&controls.relay_url),
        "relay_token": lock_string(&controls.relay_token),
    })
    .to_string()
    .into_bytes();
    let controls = Arc::clone(controls);
    spawn(async move {
        let request =
            HttpRequest::post("api/hub", body).header("content-type", "application/json");
        let Ok(response) = fetch(request).await else {
            set_status(&controls, StatusMsg::RelayPushFailed("没有连上".into()));
            return;
        };
        let value: serde_json::Value =
            serde_json::from_str(&response.text()).unwrap_or(serde_json::json!({}));
        if let Some(err) = value.get("error").and_then(serde_json::Value::as_str) {
            let message = if err.contains("口令") {
                StatusMsg::RelayNeedToken
            } else if err.contains("地址") {
                StatusMsg::RelayBadUrl
            } else if err.contains("开始") {
                StatusMsg::ShareNeedsCapture
            } else {
                StatusMsg::RelayPushFailed(err.to_string())
            };
            set_status(&controls, message);
            return;
        }
        let Some(url) = value.get("url").and_then(serde_json::Value::as_str) else {
            set_status(&controls, StatusMsg::RelayPushFailed("没有分享地址".into()));
            return;
        };
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        share_id.set(id);
        set_clipboard(url.to_string());
        let relay = value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|url| !url.contains("127.0.0.1"));
        set_status(
            &controls,
            StatusMsg::Shared {
                url: url.to_string(),
                minutes,
                local_only: url.contains("127.0.0.1"),
                relay,
            },
        );
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn end_share(controls: &Controls, share_id: Signal<String>) {
    controls.hub.end_share();
    share_id.set(String::new());
    set_status(controls, StatusMsg::ShareStopped);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn end_share(controls: &Controls, share_id: Signal<String>) {
    stop_relay_push(controls);
    controls.hub.end_share();
    share_id.set(String::new());
    set_status(controls, StatusMsg::ShareStopped);
}

/// Address other people can open. `local_only` when the listener is not on every interface.
pub(crate) fn share_base(controls: &Controls) -> (String, bool) {
    let port = lock_port(&controls.listen_port);
    if lock_bool(&controls.listen_all)
        && let Some(ip) = choose_lan(&lan_addrs(), routed_v4(), "").map(|addr| addr.ip)
    {
        return (format!("http://{ip}:{port}"), false);
    }
    (format!("http://127.0.0.1:{port}"), true)
}

pub(crate) fn apply_saved_prefs(hub: &Hub) {
    let saved = saved_config();
    hub.set_bypass(&saved.bypass_hosts);
    hub.set_rules(&saved.rules);
    hub.set_upstream(&saved.upstream_proxy);
    hub.set_proxy_auth(&config::runtime_proxy_auth(&saved.proxy_auth));
    hub.set_rules_enabled(saved.rules_enabled);
    hub.set_breaks_enabled(saved.breaks_enabled);
    hub.set_show_tunnel_failures(saved.show_tunnel_failures);
    hub.set_relay_ingress(saved.relay_accept, &saved.relay_token);
}

pub(crate) fn refuse_relay(controls: &Controls, id: Option<u64>) -> bool {
    let Some(id) = id else {
        return false;
    };
    let locked = controls.hub.with_session(|session| {
        session
            .get(id)
            .is_some_and(|flow| flow.relay_source.is_some())
    });
    if locked {
        set_status(controls, StatusMsg::RelayViewOnly);
    }
    locked
}
