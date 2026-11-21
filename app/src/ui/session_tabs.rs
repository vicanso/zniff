//! Read-only imported session tabs (.har / .chlsj) and cross-tab baseline diff (E10 / ADR-22).

#[allow(unused_imports)]
use super::*;

pub(crate) const LIVE_TAB_ID: u64 = 0;
pub(crate) const MAX_IMPORTED_TABS: usize = 8;

/// Lightweight reactive summary of a session tab for the tab bar UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabSummary {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) count: usize,
    pub(crate) read_only: bool,
}

/// Cross-tab baseline reference storing the owning tab id, flow id, and hydrated snapshot.
#[derive(Clone, Debug)]
pub(crate) struct BaselineRef {
    pub(crate) tab_id: u64,
    pub(crate) flow_id: u64,
    pub(crate) snapshot: session::Flow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TabLimitError;

struct ImportedSessionTab {
    id: u64,
    name: String,
    session: session::Session,
}

/// Manages the Live tab alongside up to `MAX_IMPORTED_TABS` read-only imported sessions.
pub(crate) struct SessionTabsStore {
    active_tab: u64,
    next_tab_id: u64,
    tabs: Vec<ImportedSessionTab>,
    baseline: Option<BaselineRef>,
    version: u64,
}

impl Default for SessionTabsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTabsStore {
    pub(crate) fn new() -> Self {
        Self {
            active_tab: LIVE_TAB_ID,
            next_tab_id: 1,
            tabs: Vec::new(),
            baseline: None,
            version: 0,
        }
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn active_tab_id(&self) -> u64 {
        self.active_tab
    }

    pub(crate) fn is_active_read_only(&self) -> bool {
        self.active_tab != LIVE_TAB_ID
    }

    #[cfg(test)]
    pub(crate) fn imported_count(&self) -> usize {
        self.tabs.len()
    }

    pub(crate) fn open_session(
        &mut self,
        name: String,
        session: session::Session,
    ) -> Result<u64, TabLimitError> {
        if self.tabs.len() >= MAX_IMPORTED_TABS {
            return Err(TabLimitError);
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let clean_name = if name.trim().is_empty() {
            format!("import-{id}.har")
        } else {
            name.trim().to_string()
        };
        self.tabs.push(ImportedSessionTab {
            id,
            name: clean_name,
            session,
        });
        self.active_tab = id;
        self.version = self.version.wrapping_add(1);
        Ok(id)
    }

    pub(crate) fn switch_to(&mut self, tab_id: u64) -> bool {
        if tab_id != LIVE_TAB_ID && !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return false;
        }
        if self.active_tab != tab_id {
            self.active_tab = tab_id;
            self.version = self.version.wrapping_add(1);
        }
        true
    }

    pub(crate) fn close_tab(&mut self, tab_id: u64) -> bool {
        if tab_id == LIVE_TAB_ID {
            return false;
        }
        let Some(pos) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        self.tabs.remove(pos);
        if self.active_tab == tab_id {
            self.active_tab = if pos > 0 {
                self.tabs[pos - 1].id
            } else {
                LIVE_TAB_ID
            };
        }
        if self.baseline.as_ref().is_some_and(|b| b.tab_id == tab_id) {
            self.baseline = None;
        }
        self.version = self.version.wrapping_add(1);
        true
    }

    pub(crate) fn summaries(&self, live_count: usize) -> Vec<TabSummary> {
        let mut out = Vec::with_capacity(self.tabs.len() + 1);
        out.push(TabSummary {
            id: LIVE_TAB_ID,
            name: String::new(),
            count: live_count,
            read_only: false,
        });
        for tab in &self.tabs {
            out.push(TabSummary {
                id: tab.id,
                name: tab.name.clone(),
                count: tab.session.len(),
                read_only: true,
            });
        }
        out
    }

    pub(crate) fn active_session(&self) -> Option<&session::Session> {
        if self.active_tab == LIVE_TAB_ID {
            None
        } else {
            self.tabs
                .iter()
                .find(|tab| tab.id == self.active_tab)
                .map(|tab| &tab.session)
        }
    }

    pub(crate) fn set_baseline_ref(&mut self, tab_id: u64, flow: session::Flow) {
        self.baseline = Some(BaselineRef {
            tab_id,
            flow_id: flow.id,
            snapshot: flow,
        });
        self.version = self.version.wrapping_add(1);
    }

    pub(crate) fn clear_baseline(&mut self) {
        if self.baseline.take().is_some() {
            self.version = self.version.wrapping_add(1);
        }
    }

    pub(crate) fn clear_baseline_for_tab(&mut self, tab_id: u64) {
        if self.baseline.as_ref().is_some_and(|b| b.tab_id == tab_id) {
            self.baseline = None;
            self.version = self.version.wrapping_add(1);
        }
    }

    pub(crate) fn baseline_ref(&self) -> Option<&BaselineRef> {
        self.baseline.as_ref()
    }

    pub(crate) fn baseline_id_for_tab(&self, tab_id: u64) -> Option<u64> {
        self.baseline
            .as_ref()
            .filter(|b| b.tab_id == tab_id)
            .map(|b| b.flow_id)
    }

    pub(crate) fn baseline_key(&self) -> Option<(u64, u64)> {
        self.baseline.as_ref().map(|b| (b.tab_id, b.flow_id))
    }
}

pub(crate) fn tab_display_name(path: &str) -> String {
    let trimmed = path.trim();
    std::path::Path::new(trimmed)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if trimmed.is_empty() {
                "import.har".to_string()
            } else {
                trimmed.to_string()
            }
        })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn build_imported_session(
    items: Vec<crate::har::Imported>,
) -> (session::Session, usize) {
    let count = items.len();
    let mut session = session::Session::default();
    for item in items {
        if item.protocol == session::Protocol::Marker {
            session.add_marker_at(item.path, item.started);
            continue;
        }
        let id = session.open(session::FlowDraft {
            protocol: item.protocol,
            method: item.method,
            host: item.host,
            path: item.path,
            url: item.url,
            request_headers: item.request_headers,
        });
        session.set_started(id, item.started);
        if let Some(status) = item.status {
            session.set_response(id, status, item.response_headers, Some(item.protocol));
        }
        session.append_body(id, Direction::Request, &item.request_body);
        session.append_body(id, Direction::Response, &item.response_body);
        if let Some(error) = item.error {
            session.set_error(id, error);
        }
        session.mark_finished(id);
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
    }
    (session, count)
}

fn lock_tabs(controls: &Controls) -> std::sync::MutexGuard<'_, SessionTabsStore> {
    controls
        .session_tabs
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_imported_flows_tab(
    controls: &Controls,
    path: &str,
    items: Vec<crate::har::Imported>,
) {
    let (session, count) = build_imported_session(items);
    let result = lock_tabs(controls).open_session(tab_display_name(path), session);
    match result {
        Ok(_) => set_status(
            controls,
            StatusMsg::Imported {
                count,
                path: path.to_string(),
            },
        ),
        Err(TabLimitError) => set_status(controls, StatusMsg::TabLimitReached(MAX_IMPORTED_TABS)),
    }
}

pub(crate) fn with_active_session<R>(
    controls: &Controls,
    f: impl FnOnce(&session::Session) -> R,
) -> R {
    let guard = lock_tabs(controls);
    if let Some(session) = guard.active_session() {
        return f(session);
    }
    drop(guard);
    controls.hub.with_session(|s| f(s))
}

pub(crate) fn active_matching_ids(
    controls: &Controls,
    filter: &str,
) -> Vec<(u64, Option<SystemTime>)> {
    let guard = lock_tabs(controls);
    if let Some(session) = guard.active_session() {
        return session.matching_with_previous_start(filter);
    }
    drop(guard);
    controls.hub.matching_ids(filter).0
}

pub(crate) fn active_tab_id(controls: &Controls) -> u64 {
    lock_tabs(controls).active_tab_id()
}

pub(crate) fn is_active_tab_read_only(controls: &Controls) -> bool {
    lock_tabs(controls).is_active_read_only()
}

pub(crate) fn session_tabs_version(controls: &Controls) -> u64 {
    lock_tabs(controls).version()
}

pub(crate) fn session_tab_summaries(controls: &Controls, live_count: usize) -> Vec<TabSummary> {
    lock_tabs(controls).summaries(live_count)
}

pub(crate) fn baseline_key_for_controls(controls: &Controls) -> Option<(u64, u64)> {
    lock_tabs(controls).baseline_key()
}

pub(crate) fn baseline_id_for_active_tab(controls: &Controls) -> Option<u64> {
    let store = lock_tabs(controls);
    store.baseline_id_for_tab(store.active_tab_id())
}

/// Returns `true` and notifies the user if the active tab is a read-only imported tab.
pub(crate) fn guard_read_only_tab(controls: &Controls) -> bool {
    if is_active_tab_read_only(controls) {
        set_status(controls, StatusMsg::ReadOnlyTab);
        true
    } else {
        false
    }
}

/// Resolves the baseline flow for line-level diff, supporting both same-tab and cross-tab comparison.
pub(crate) fn resolve_baseline_flow(
    controls: &Controls,
    selected_id: Option<u64>,
    fallback_baseline: Option<u64>,
) -> Option<session::Flow> {
    let (active_tab, base_ref) = {
        let store = lock_tabs(controls);
        (store.active_tab_id(), store.baseline_ref().cloned())
    };
    if let Some(base) = base_ref {
        if base.tab_id == active_tab && selected_id == Some(base.flow_id) {
            return None;
        }
        if base.tab_id == LIVE_TAB_ID {
            return controls
                .hub
                .with_session(|session| session.hydrated(base.flow_id))
                .or(Some(base.snapshot));
        }
        return Some(base.snapshot);
    }
    let base_id = fallback_baseline.filter(|&id| selected_id != Some(id))?;
    with_active_session(controls, |session| session.hydrated(base_id))
}

#[derive(Clone)]
pub(crate) struct SessionTabBarProps {
    pub(crate) tabs: Signal<Vec<TabSummary>>,
    pub(crate) active_tab: Signal<u64>,
    pub(crate) controls: Arc<Controls>,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) baseline: Signal<Option<u64>>,
    pub(crate) compose: Compose,
    pub(crate) seq_builder: SeqBuilder,
}

fn switch_active_tab(props: &SessionTabBarProps, tab_id: u64) {
    let changed = {
        let mut store = lock_tabs(&props.controls);
        let prev = store.active_tab_id();
        store.switch_to(tab_id) && prev != tab_id
    };
    if changed {
        props.active_tab.set(tab_id);
        props.selected.set(None);
        reset_compose(props.compose);
        close_sequence_diagram(props.seq_builder);
        props
            .baseline
            .set(baseline_id_for_active_tab(&props.controls));
    }
}

fn close_imported_tab(props: &SessionTabBarProps, tab_id: u64) {
    let (closed, next_active, next_baseline) = {
        let mut store = lock_tabs(&props.controls);
        let prev = store.active_tab_id();
        let closed = store.close_tab(tab_id);
        let next = store.active_tab_id();
        let base = store.baseline_id_for_tab(next);
        (closed && prev == tab_id, next, base)
    };
    props.active_tab.set(next_active);
    props.baseline.set(next_baseline);
    if closed {
        props.selected.set(None);
        reset_compose(props.compose);
        close_sequence_diagram(props.seq_builder);
    }
}

/// Compact session tab bar rendered above the flow list only when >= 1 imported tab is open.
pub(crate) fn session_tab_bar() -> impl IntoElement {
    let Some(props) = use_context::<SessionTabBarProps>() else {
        return view().into_element();
    };
    let tabs = props.tabs;
    when(
        move || tabs.with(|list| list.len() > 1),
        move || {
            let props = props.clone();
            view()
                .w_full()
                .h(px(32.0))
                .flex_shrink_0()
                .bg(bg_chrome())
                .border_b_1()
                .border_b_color(border_strong())
                .px_2()
                .flex_row()
                .items_center()
                .child(
                    scroll()
                        .w_full()
                        .horizontal()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(view().flex_row().items_center().gap_1().child(each(
                            move || props.tabs.get(),
                            |tab| tab.id,
                            move |item, _index| session_tab_chip(props.clone(), item),
                        ))),
                )
        },
    )
    .into_element()
}

fn session_tab_chip(props: SessionTabBarProps, item: ReadSignal<TabSummary>) -> impl IntoElement {
    let tab_id = item.with_untracked(|t| t.id);
    let is_ro = item.with_untracked(|t| t.read_only);
    let active_tab = props.active_tab;
    let langs = ui_langs();
    let switch_props = props.clone();
    let close_props = props;

    view()
        .h(px(24.0))
        .px_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || {
            if active_tab.get() == tab_id {
                accent_blue_border()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .bg(move || {
            if active_tab.get() == tab_id {
                bg_card()
            } else {
                AdaptiveColor::TRANSPARENT
            }
        })
        .hover(|s| s.bg(bg_control_hover()))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1p5()
        .cursor(Cursor::Pointer)
        .on_press(move || switch_active_tab(&switch_props, tab_id))
        .child(when(
            move || !is_ro,
            move || {
                view().w(px(6.0)).h(px(6.0)).rounded_full().bg(move || {
                    if active_tab.get() == tab_id {
                        status_ok_fg()
                    } else {
                        fg_muted()
                    }
                })
            },
        ))
        .child(
            text(move || -> SharedString {
                item.with(|tab| {
                    let label = if tab.read_only {
                        tab.name.as_str()
                    } else {
                        t(ui_lang(langs), "实时会话", "Live")
                    };
                    format!("{label} ({})", tab.count).into()
                })
            })
            .max_w(px(172.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Middle)
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if active_tab.get() == tab_id {
                    fg_primary()
                } else {
                    fg_secondary()
                }
            }),
        )
        .child(when(
            move || is_ro,
            move || {
                view()
                    .px_1()
                    .py_0p5()
                    .rounded(px(4.0))
                    .bg(bg_control())
                    .child(
                        text(tr("只读", "RO"))
                            .text_size(px(9.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg_muted()),
                    )
            },
        ))
        .child(when(
            move || is_ro,
            move || {
                let close_props = close_props.clone();
                view()
                    .w(px(16.0))
                    .h(px(16.0))
                    .rounded(px(4.0))
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(bg_control_hover()))
                    .cursor(Cursor::Pointer)
                    .label(tr("关闭标签", "Close tab"))
                    .on_press(move || close_imported_tab(&close_props, tab_id))
                    .child(glyph_px(9.0, || Glyph::Close, || color(fg_muted())))
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session(host: &str, path: &str, status: u16, body: &[u8]) -> session::Session {
        let mut s = session::Session::default();
        let id = s.open(session::FlowDraft {
            protocol: session::Protocol::Http11,
            method: "GET".to_string(),
            host: host.to_string(),
            path: path.to_string(),
            url: format!("https://{host}{path}"),
            request_headers: vec![("Host".to_string(), host.to_string())],
        });
        s.set_response(
            id,
            status,
            vec![("Content-Type".to_string(), "application/json".to_string())],
            Some(session::Protocol::Http11),
        );
        s.append_body(id, Direction::Response, body);
        s.mark_finished(id);
        s
    }

    #[test]
    fn tab_creation_and_display_name() {
        assert_eq!(
            tab_display_name("/tmp/prod-capture.har"),
            "prod-capture.har"
        );
        assert_eq!(tab_display_name("session.chlsj"), "session.chlsj");
        assert_eq!(tab_display_name("   "), "import.har");

        let mut store = SessionTabsStore::new();
        assert_eq!(store.active_tab_id(), LIVE_TAB_ID);
        assert!(!store.is_active_read_only());

        let s1 = sample_session("api.example.com", "/v1/users", 200, br#"{"v":1}"#);
        let id1 = store.open_session("prod.har".into(), s1).expect("open tab");
        assert_eq!(id1, 1);
        assert_eq!(store.active_tab_id(), id1);
        assert!(store.is_active_read_only());
        assert_eq!(store.imported_count(), 1);

        let summaries = store.summaries(5);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, LIVE_TAB_ID);
        assert_eq!(summaries[0].count, 5);
        assert!(!summaries[0].read_only);
        assert_eq!(summaries[1].id, id1);
        assert_eq!(summaries[1].name, "prod.har");
        assert_eq!(summaries[1].count, 1);
        assert!(summaries[1].read_only);
    }

    #[test]
    fn tab_cap_enforcement_blocks_ninth_until_one_closes() {
        let mut store = SessionTabsStore::new();
        let mut ids = Vec::new();
        for i in 0..MAX_IMPORTED_TABS {
            let s = sample_session("cap.test", &format!("/{i}"), 200, b"ok");
            let id = store
                .open_session(format!("cap-{i}.har"), s)
                .expect("within cap");
            ids.push(id);
        }
        assert_eq!(store.imported_count(), MAX_IMPORTED_TABS);

        let overflow = sample_session("cap.test", "/overflow", 200, b"no");
        assert_eq!(
            store.open_session("overflow.har".into(), overflow),
            Err(TabLimitError)
        );
        assert_eq!(store.imported_count(), MAX_IMPORTED_TABS);

        assert!(store.close_tab(ids[0]));
        assert_eq!(store.imported_count(), MAX_IMPORTED_TABS - 1);
        let next = sample_session("cap.test", "/after-close", 200, b"yes");
        assert!(store.open_session("after.har".into(), next).is_ok());
        assert_eq!(store.imported_count(), MAX_IMPORTED_TABS);
    }

    #[test]
    fn tab_switching_and_closing_fallback() {
        let mut store = SessionTabsStore::new();
        let id1 = store
            .open_session("a.har".into(), sample_session("a.test", "/a", 200, b"a"))
            .unwrap();
        let id2 = store
            .open_session("b.har".into(), sample_session("b.test", "/b", 200, b"b"))
            .unwrap();

        assert_eq!(store.active_tab_id(), id2);
        assert!(!store.switch_to(999));
        assert_eq!(store.active_tab_id(), id2);

        assert!(store.switch_to(LIVE_TAB_ID));
        assert!(!store.is_active_read_only());
        assert!(store.active_session().is_none());

        assert!(store.switch_to(id2));
        assert!(store.is_active_read_only());
        assert!(store.active_session().is_some());

        // Cannot close Live tab.
        assert!(!store.close_tab(LIVE_TAB_ID));
        // Closing active tab 2 falls back to tab 1.
        assert!(store.close_tab(id2));
        assert_eq!(store.active_tab_id(), id1);
        // Closing active tab 1 falls back to Live.
        assert!(store.close_tab(id1));
        assert_eq!(store.active_tab_id(), LIVE_TAB_ID);
        assert!(!store.is_active_read_only());
    }

    #[test]
    fn read_only_guard_and_cross_tab_baseline_diff_resolution() {
        let dir =
            std::env::temp_dir().join(format!("zniff-session-tabs-test-{}", std::process::id()));
        let ca = Arc::new(Ca::open(&dir).expect("ca"));
        let hub = Arc::new(Hub::new(ca).expect("hub"));
        let _ = std::fs::remove_dir_all(&dir);

        // Add one flow to Live session (flow_id = 1).
        let live_id = hub.with_session(|s| {
            let id = s.open(session::FlowDraft {
                protocol: session::Protocol::Http11,
                method: "GET".into(),
                host: "api.example.com".into(),
                path: "/v1/items".into(),
                url: "https://api.example.com/v1/items".into(),
                request_headers: vec![],
            });
            s.set_response(
                id,
                200,
                vec![("Content-Type".into(), "application/json".into())],
                Some(session::Protocol::Http11),
            );
            s.append_body(id, Direction::Response, br#"{"source":"live"}"#);
            s.mark_finished(id);
            id
        });

        let controls = Arc::new(Controls {
            hub,
            capture: Mutex::new(None),
            status: Mutex::new(StatusMsg::Idle),
            running: Mutex::new(false),
            listen: Mutex::new(String::new()),
            listen_port: Mutex::new(8899),
            ca_status: Mutex::new(ca::CaStatus::NotInstalled),
            active_external_ca: Mutex::new(None),
            external_cas: Mutex::new(Vec::new()),
            use_system_proxy: Mutex::new(false),
            proxy_applied: Mutex::new(false),
            ops: Mutex::new(()),
            listen_all: Mutex::new(false),
            update_state: Mutex::new(UpdateState::default()),
            redact_export: Mutex::new(false),
            redact_strict: Mutex::new(false),
            redact_fields: Mutex::new(String::new()),
            relay_url: Mutex::new(String::new()),
            relay_token: Mutex::new(String::new()),
            #[cfg(not(target_arch = "wasm32"))]
            relay_push: Mutex::new(None),
            toast_seq: std::sync::atomic::AtomicU64::new(0),
            toast_queue: Mutex::new(Vec::new()),
            config_error: Mutex::new(None),
            config_path: Mutex::new(String::new()),
            pending_replay_diff: Mutex::new(None),
            session_tabs: Mutex::new(SessionTabsStore::new()),
        });

        // Live tab is not read-only.
        assert!(!guard_read_only_tab(&controls));

        // Open an imported tab with a flow that also has flow_id = 1.
        let imported = sample_session(
            "api.example.com",
            "/v1/items",
            200,
            br#"{"source":"imported"}"#,
        );
        let tab_id = {
            let mut store = controls.session_tabs.lock().unwrap();
            store.open_session("prod.har".into(), imported).unwrap()
        };

        // Imported tab triggers read-only guard.
        assert!(guard_read_only_tab(&controls));
        assert_eq!(lock_status(&controls), StatusMsg::ReadOnlyTab);

        // Set imported flow #1 as baseline.
        let imported_flow = with_active_session(&controls, |s| s.hydrated(1)).unwrap();
        {
            let mut store = controls.session_tabs.lock().unwrap();
            store.set_baseline_ref(tab_id, imported_flow);
        }

        // On the imported tab, diffing flow #1 against itself returns None.
        assert!(resolve_baseline_flow(&controls, Some(1), Some(1)).is_none());
        assert_eq!(baseline_id_for_active_tab(&controls), Some(1));

        // Switch back to Live tab: baseline_id_for_active_tab is None (no false row mark on Live #1),
        // yet resolve_baseline_flow resolves the imported tab's flow #1 for cross-tab diff!
        {
            let mut store = controls.session_tabs.lock().unwrap();
            assert!(store.switch_to(LIVE_TAB_ID));
        }
        assert_eq!(baseline_id_for_active_tab(&controls), None);
        let resolved = resolve_baseline_flow(&controls, Some(live_id), None)
            .expect("cross-tab baseline should resolve");
        assert_eq!(resolved.response_body, br#"{"source":"imported"}"#);

        // Closing the imported tab clears its baseline reference.
        {
            let mut store = controls.session_tabs.lock().unwrap();
            assert!(store.close_tab(tab_id));
        }
        assert!(resolve_baseline_flow(&controls, Some(live_id), None).is_none());
    }
}
