#[allow(unused_imports)]
use super::*;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const WEB_LONG_POLL_WAIT_MS: u64 = 8_000;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const WEB_ACTIVE_FLOOR_MS: u64 = 400;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn build_web_state_url(
    seeded: bool,
    last_stamp: Option<(u64, u64)>,
    immediate: bool,
) -> String {
    if !seeded {
        return "api/state".to_string();
    }
    if !immediate && let Some((gen_num, rev_num)) = last_stamp {
        return format!(
            "api/state?lite=1&since_gen={gen_num}&since_rev={rev_num}&wait_ms={WEB_LONG_POLL_WAIT_MS}"
        );
    }
    "api/state?lite=1".to_string()
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn web_poll_delay_ms(
    capturing: bool,
    state_changed: bool,
    fetch_ok: bool,
    idle_streak: u32,
) -> u64 {
    if !fetch_ok {
        match idle_streak {
            0 | 1 => 1_000,
            2 | 3 => 2_000,
            _ => 4_000,
        }
    } else if state_changed {
        WEB_ACTIVE_FLOOR_MS
    } else if capturing {
        500
    } else {
        1_500
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, PartialEq, Eq)]
struct WebPollSnapshot {
    capturing: bool,
    gen_num: u64,
    rev_num: u64,
    paused_len: usize,
    share_id: String,
    relay_accept: bool,
}

#[cfg(target_arch = "wasm32")]
struct WebPollUi {
    port_text: Signal<String>,
    listen_all: Signal<bool>,
    profiles: ProfileSignals,
    seq_builder: SeqBuilder,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct WebPollState {
    seeded: bool,
    seen_error: String,
    last_snap: Option<WebPollSnapshot>,
    idle_streak: u32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct WebFlowsSync {
    seeded: bool,
    last_stamp: Option<(u64, u64)>,
    last_synced: Option<(u64, u64, String)>,
    in_flight: Option<(u64, u64, String)>,
    seq: u64,
}

#[cfg(target_arch = "wasm32")]
struct WebPollCursor<'a> {
    filter: &'a str,
    picked: Option<u64>,
    baseline: Option<u64>,
}

#[cfg(target_arch = "wasm32")]
fn apply_web_state_snapshot(
    controls: &Controls,
    ui: &WebPollUi,
    state: &mut WebPollState,
    value: &serde_json::Value,
) -> Option<WebPollSnapshot> {
    let gen_num = value
        .get("generation")
        .and_then(serde_json::Value::as_u64)?;
    let rev_num = value
        .get("session_rev")
        .and_then(serde_json::Value::as_u64)?;
    controls.hub.apply_state(value);
    let mut relay_accept = ui.profiles.relay_accept.get_untracked();
    if let Some(prefs) = value.get("prefs") {
        if let Some(on) = prefs
            .get("relay_accept")
            .and_then(serde_json::Value::as_bool)
        {
            relay_accept = on;
            if ui.profiles.relay_accept.get_untracked() != on {
                ui.profiles.relay_accept.set(on);
            }
        }
        let cfg_err = prefs
            .get("config_error")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        *controls
            .config_error
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = cfg_err;
        if let Some(cfg_path) = prefs.get("config_path").and_then(serde_json::Value::as_str) {
            *controls
                .config_path
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = cfg_path.to_string();
        }
    }
    if !state.seeded {
        state.seeded = true;
        if let Some(port) = json_port(value) {
            set_port(&controls.listen_port, port);
            ui.port_text.set(port.to_string());
        }
        if let Some(all) = value.get("listen_all").and_then(serde_json::Value::as_bool) {
            set_bool(&controls.listen_all, all);
            ui.listen_all.set(all);
        }
        if let Some(prefs) = value.get("prefs") {
            seed_web_prefs(controls, prefs, ui.profiles, ui.seq_builder);
        }
    }
    let capturing = value
        .get("capturing")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    set_bool(&controls.running, capturing);
    let listen = value
        .get("listen")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    *controls
        .listen
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = listen.to_string();
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !error.is_empty() && error != state.seen_error.as_str() {
        state.seen_error = error.to_string();
        set_status(
            controls,
            StatusMsg::BindFailed {
                port: json_port(value).unwrap_or(0),
                err: error.to_string(),
            },
        );
    } else if error.is_empty() {
        state.seen_error.clear();
    }
    let paused_len = value
        .get("paused_queue")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let share_id = value
        .get("share_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(WebPollSnapshot {
        capturing,
        gen_num,
        rev_num,
        paused_len,
        share_id,
        relay_accept,
    })
}

#[cfg(target_arch = "wasm32")]
async fn sync_web_flows_if_needed(
    controls: &Controls,
    effective_filter: Signal<String>,
    flows_sync: &Rc<RefCell<WebFlowsSync>>,
) {
    let (trimmed_filter, desired_sync, should_fetch, my_seq) = {
        let mut sync = flows_sync.borrow_mut();
        let trimmed_filter = effective_filter.get_untracked().trim().to_string();
        let desired_sync = sync
            .last_stamp
            .map(|(gen_num, rev_num)| (gen_num, rev_num, trimmed_filter.clone()));
        let already_synced = matches!(
            (&sync.last_synced, &desired_sync),
            (Some(prev), Some(next)) if prev == next
        );
        let already_in_flight = matches!(
            (&sync.in_flight, &desired_sync),
            (Some(cur), Some(next)) if cur == next
        );
        if already_synced || already_in_flight {
            (trimmed_filter, desired_sync, false, 0)
        } else {
            sync.seq = sync.seq.wrapping_add(1);
            sync.in_flight = desired_sync.clone();
            (trimmed_filter, desired_sync, true, sync.seq)
        }
    };
    if !should_fetch {
        return;
    }
    let flows_url = if trimmed_filter.is_empty() {
        "api/flows".to_string()
    } else {
        format!(
            "api/flows?filter={}",
            url_encode_query_component(&trimmed_filter)
        )
    };
    let result = fetch(HttpRequest::get(flows_url)).await;
    {
        let mut sync = flows_sync.borrow_mut();
        if sync.seq == my_seq {
            sync.in_flight = None;
        }
    }
    let Ok(response) = result else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text()) else {
        return;
    };
    let still_current = {
        let sync = flows_sync.borrow();
        sync.seq == my_seq && effective_filter.get_untracked().trim() == trimmed_filter.as_str()
    };
    if !still_current {
        return;
    }
    controls.hub.apply_flows(&value);
    flows_sync.borrow_mut().last_synced = desired_sync;
}

#[cfg(target_arch = "wasm32")]
struct DetailFetchGuard<'a> {
    hub: &'a Hub,
    id: u64,
}

#[cfg(target_arch = "wasm32")]
impl Drop for DetailFetchGuard<'_> {
    fn drop(&mut self) {
        self.hub.end_detail_fetch(self.id);
    }
}

#[cfg(target_arch = "wasm32")]
async fn hydrate_selected_web_details(controls: &Controls, picked: Option<u64>, base: Option<u64>) {
    for maybe_id in [picked, if base != picked { base } else { None }] {
        let Some(id) = maybe_id else {
            continue;
        };
        for _ in 0..2 {
            let needs = controls
                .hub
                .with_session(|session| session.cached_needs_detail(id));
            if !needs || !controls.hub.begin_detail_fetch(id) {
                break;
            }
            let _guard = DetailFetchGuard {
                hub: &controls.hub,
                id,
            };
            let Ok(response) = fetch(HttpRequest::get(format!("api/flows/{id}"))).await else {
                break;
            };
            if response.ok() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text()) {
                    controls.hub.fill_detail(&value);
                } else {
                    break;
                }
            } else if response.status() == 404 {
                controls.hub.drop_cached_flow(id);
                proxy::notify_poll_wake();
                break;
            } else {
                break;
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn needs_selected_detail(controls: &Controls, picked: Option<u64>, base: Option<u64>) -> bool {
    [picked, if base != picked { base } else { None }]
        .into_iter()
        .flatten()
        .any(|id| {
            controls
                .hub
                .with_session(|session| session.cached_needs_detail(id))
        })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn start_web_detail_watch(
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
) {
    effect(move || {
        let picked = selected.get();
        let base = baseline.get();
        if needs_selected_detail(&controls, picked, base) {
            let controls = controls.clone();
            spawn(async move {
                hydrate_selected_web_details(&controls, picked, base).await;
            });
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn wait_next_web_poll(
    controls: &Controls,
    effective_filter: Signal<String>,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    web_bg: Signal<bool>,
    cursor: &WebPollCursor<'_>,
    delay_ms: u64,
) -> bool {
    const SLICE_MS: u64 = 150;
    let mut elapsed = 0u64;
    while elapsed < delay_ms {
        if web_bg.get_untracked() {
            return false;
        }
        if proxy::take_poll_wake() {
            return true;
        }
        let step = (delay_ms - elapsed).min(SLICE_MS);
        timer(Duration::from_millis(step)).await;
        elapsed += step;
        if web_bg.get_untracked() {
            return false;
        }
        if proxy::take_poll_wake() {
            return true;
        }
        if effective_filter.get_untracked().trim() != cursor.filter {
            return true;
        }
        let cur_picked = selected.get_untracked();
        let cur_baseline = baseline.get_untracked();
        if (cur_picked != cursor.picked || cur_baseline != cursor.baseline)
            && needs_selected_detail(controls, cur_picked, cur_baseline)
        {
            return true;
        }
    }
    false
}

/// The browser does not capture. This loop copies the server process into the
/// same signals the desktop tick already reads, using 8s long-polling while in
/// foreground and pausing completely while the tab is hidden (`Background`).
#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_web_poll(
    controls: Arc<Controls>,
    port_text: Signal<String>,
    listen_all: Signal<bool>,
    profiles: ProfileSignals,
    effective_filter: Signal<String>,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    web_bg: Signal<bool>,
) {
    let flows_sync = Rc::new(RefCell::new(WebFlowsSync::default()));
    let filter_controls = controls.clone();
    let filter_sync = flows_sync.clone();
    let seen_filter = Rc::new(RefCell::new(None::<String>));
    effect(move || {
        let cur_filter = effective_filter.get().trim().to_string();
        let first = seen_filter.borrow().is_none();
        let changed = seen_filter.borrow().as_deref() != Some(cur_filter.as_str());
        *seen_filter.borrow_mut() = Some(cur_filter);
        if first || !changed || !filter_sync.borrow().seeded || web_bg.get_untracked() {
            return;
        }
        let controls = filter_controls.clone();
        let flows_sync = filter_sync.clone();
        spawn(async move {
            sync_web_flows_if_needed(&controls, effective_filter, &flows_sync).await;
        });
    });

    spawn(async move {
        let ui = WebPollUi {
            port_text,
            listen_all,
            profiles,
            seq_builder,
        };
        let mut state = WebPollState::default();
        let mut immediate = false;
        loop {
            if state.seeded && web_bg.get_untracked() {
                while web_bg.get_untracked() {
                    timer(Duration::from_millis(200)).await;
                }
                let _ = proxy::take_poll_wake();
                immediate = true;
            }
            if proxy::take_poll_wake() {
                immediate = true;
            }
            let last_stamp = state.last_snap.as_ref().map(|s| (s.gen_num, s.rev_num));
            let url = build_web_state_url(state.seeded, last_stamp, immediate);
            immediate = false;
            let mut next_snap = None;
            let mut fetch_ok = false;
            if let Ok(response) = fetch(HttpRequest::get(url)).await {
                proxy::note_web_state_url(&response.head.url);
                if response.ok()
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.text())
                {
                    fetch_ok = true;
                    next_snap = apply_web_state_snapshot(&controls, &ui, &mut state, &value);
                }
            }
            let state_changed = match (&state.last_snap, &next_snap) {
                (Some(prev), Some(next)) => prev != next,
                (None, Some(_)) => true,
                _ => false,
            };
            if let Some(snap) = next_snap {
                state.last_snap = Some(snap);
            }
            if fetch_ok && state_changed {
                state.idle_streak = 0;
            } else {
                state.idle_streak = state.idle_streak.saturating_add(1);
            }
            let state_stamp = state.last_snap.as_ref().map(|s| (s.gen_num, s.rev_num));
            {
                let mut sync = flows_sync.borrow_mut();
                sync.seeded = state.seeded;
                sync.last_stamp = state_stamp;
            }
            let trimmed_filter = effective_filter.get_untracked().trim().to_string();
            if fetch_ok {
                sync_web_flows_if_needed(&controls, effective_filter, &flows_sync).await;
            }
            let picked = selected.get_untracked();
            let base = baseline.get_untracked();
            hydrate_selected_web_details(&controls, picked, base).await;
            let capturing = state.last_snap.as_ref().is_some_and(|s| s.capturing);
            let delay_ms = web_poll_delay_ms(capturing, state_changed, fetch_ok, state.idle_streak);
            let cursor = WebPollCursor {
                filter: &trimmed_filter,
                picked,
                baseline: base,
            };
            if wait_next_web_poll(
                &controls,
                effective_filter,
                selected,
                baseline,
                web_bg,
                &cursor,
                delay_ms,
            )
            .await
            {
                state.idle_streak = 0;
                immediate = true;
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn seed_web_prefs(
    controls: &Controls,
    prefs: &serde_json::Value,
    profiles: ProfileSignals,
    seq_builder: SeqBuilder,
) {
    let mut cfg = web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    if let Some(text) = prefs.get("bypass").and_then(serde_json::Value::as_str) {
        cfg.bypass_hosts = text.to_string();
        profiles.bypass.set(text.to_string());
    }
    if let Some(text) = prefs.get("rules").and_then(serde_json::Value::as_str) {
        cfg.rules = text.to_string();
        profiles.rules.set(text.to_string());
    }
    if let Some(items) = prefs
        .get("rule_groups")
        .and_then(|v| serde_json::from_value::<Vec<config::RuleGroup>>(v.clone()).ok())
    {
        let clean = config::sanitize_rule_groups(items);
        cfg.rule_groups = clean.clone();
        profiles.rule_groups.set(clean);
    }
    if let Some(text) = prefs.get("upstream").and_then(serde_json::Value::as_str) {
        cfg.upstream_proxy = text.to_string();
        profiles.upstream.set(text.to_string());
    }
    if let Some(on) = prefs
        .get("rules_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.rules_enabled = on;
        profiles.rules_on.set(on);
    }
    if let Some(on) = prefs
        .get("breaks_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.breaks_enabled = on;
        profiles.breaks_on.set(on);
    }
    if let Some(on) = prefs
        .get("show_tunnels")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.show_tunnel_failures = on;
        profiles.show_tunnels.set(on);
    }
    if let Some(text) = prefs.get("filter").and_then(serde_json::Value::as_str) {
        cfg.filter = text.to_string();
        profiles.filter.set(text.to_string());
    }
    if let Some(items) = prefs
        .get("saved_profiles")
        .and_then(|v| serde_json::from_value::<Vec<config::SavedProfile>>(v.clone()).ok())
    {
        cfg.saved_profiles = items.clone();
        profiles.saved.set(items);
    }
    if let Some(items) = prefs
        .get("profile_history")
        .and_then(|v| serde_json::from_value::<Vec<config::SavedProfile>>(v.clone()).ok())
    {
        cfg.profile_history = items.clone();
        profiles.history.set(items);
    }
    if let Some(active) = prefs
        .get("active_profile")
        .and_then(serde_json::Value::as_str)
    {
        cfg.active_profile = active.to_string();
        let initial_name = if !active.trim().is_empty() {
            active.to_string()
        } else {
            matched_saved_profile(
                &cfg.saved_profiles,
                &cfg.filter,
                &cfg.bypass_hosts,
                &cfg.rules,
                &cfg.upstream_proxy,
            )
            .unwrap_or_default()
        };
        profiles.name.set(initial_name);
    }
    if let Some(on) = prefs
        .get("relay_accept")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.relay_accept = on;
        profiles.relay_accept.set(on);
    }
    if let Some(auth) = prefs.get("proxy_auth").and_then(serde_json::Value::as_str) {
        cfg.proxy_auth = auth.to_string();
        profiles.proxy_auth.set(auth.to_string());
    }
    if let Some(url) = prefs.get("relay_url").and_then(serde_json::Value::as_str) {
        cfg.relay_url = url.to_string();
        profiles.relay_url.set(url.to_string());
        *controls
            .relay_url
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = url.to_string();
    }
    if let Some(on) = prefs
        .get("redact_export")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.redact_export = on;
        seq_builder.redact_export.set(on);
        set_bool(&controls.redact_export, on);
    }
    if let Some(on) = prefs
        .get("redact_strict")
        .and_then(serde_json::Value::as_bool)
    {
        cfg.redact_strict = on;
        seq_builder.redact_strict.set(on);
        set_bool(&controls.redact_strict, on);
    }
    if let Some(fields) = prefs
        .get("redact_fields")
        .and_then(serde_json::Value::as_str)
    {
        cfg.redact_fields = fields.to_string();
        seq_builder.redact_fields.set(fields.to_string());
        *controls
            .redact_fields
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = fields.to_string();
    }
    *web_config_slot()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = cfg;
}
