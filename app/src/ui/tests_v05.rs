//! Headless UI functional tests for v0.4.x & v0.5.x features in both Zh and En.

use super::*;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

use nsh_core::{
    ApproxTextEngine, Core, EventKind, EventPayload, MountItem, NodeId, Op, Point, PressEvent,
    Prop, ROOT_ID, Tag, ViewKind,
};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static TEST_LANG: Cell<Lang> = const { Cell::new(Lang::En) };
    static TEST_LOCKED: Cell<bool> = const { Cell::new(false) };
    static TEST_CA_TRUSTED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
struct HeadlessTree {
    kinds: HashMap<NodeId, ViewKind>,
    parent: HashMap<NodeId, NodeId>,
    children: HashMap<NodeId, Vec<NodeId>>,
    texts: HashMap<NodeId, String>,
    placeholders: HashMap<NodeId, String>,
    pressed: HashMap<NodeId, Option<nsh_core::Checked>>,
    roles: HashMap<NodeId, AccessibilityRole>,
    labels: HashMap<NodeId, Option<String>>,
    elements_hidden: HashMap<NodeId, bool>,
    focusable: HashMap<NodeId, Option<bool>>,
    keymap: Option<Arc<nsh_core::keymap::Keymap>>,
}

impl HeadlessTree {
    fn pump(&mut self, core: &mut Core<ApproxTextEngine>) -> bool {
        let mut any_commit = false;
        let mut evs = Vec::new();
        let mut scroll_echoes = Vec::new();
        for _ in 0..16 {
            scroll_echoes.clear();
            let batch = core.commit();
            if let Some(b) = &batch {
                any_commit = true;
                for item in &b.items {
                    match item {
                        MountItem::Create { id, kind } => {
                            self.kinds.insert(*id, *kind);
                        }
                        MountItem::Insert {
                            parent,
                            child,
                            before,
                        } => {
                            if let Some(old) = self.parent.insert(*child, *parent)
                                && let Some(list) = self.children.get_mut(&old)
                            {
                                list.retain(|c| c != child);
                            }
                            let list = self.children.entry(*parent).or_default();
                            list.retain(|c| c != child);
                            match before.and_then(|b| list.iter().position(|c| *c == b)) {
                                Some(pos) => list.insert(pos, *child),
                                None => list.push(*child),
                            }
                        }
                        MountItem::Remove { id } => {
                            if let Some(old) = self.parent.remove(id)
                                && let Some(list) = self.children.get_mut(&old)
                            {
                                list.retain(|c| c != id);
                            }
                        }
                        MountItem::Delete { id } => {
                            if let Some(old) = self.parent.remove(id)
                                && let Some(list) = self.children.get_mut(&old)
                            {
                                list.retain(|c| c != id);
                            }
                            self.children.remove(id);
                            self.kinds.remove(id);
                            self.texts.remove(id);
                            self.placeholders.remove(id);
                            self.pressed.remove(id);
                            self.roles.remove(id);
                            self.labels.remove(id);
                            self.elements_hidden.remove(id);
                            self.focusable.remove(id);
                        }
                        MountItem::Text { id, text, .. } => {
                            self.texts.insert(*id, text.text.clone());
                        }
                        MountItem::Input { id, config } => {
                            self.placeholders.insert(*id, config.placeholder.clone());
                        }
                        MountItem::ViewProps { id, props } => {
                            self.pressed.insert(*id, props.accessibility.state.pressed);
                            self.roles.insert(*id, props.accessibility.role);
                            self.labels.insert(
                                *id,
                                props.accessibility.label.as_deref().map(str::to_string),
                            );
                            self.elements_hidden.insert(*id, props.accessibility.hidden);
                            self.focusable.insert(*id, props.focusable);
                        }
                        MountItem::Command {
                            id,
                            command: nsh_core::Command::ScrollTo { x, y, .. },
                        } => {
                            scroll_echoes.push((*id, Point { x: *x, y: *y }));
                        }
                        MountItem::Keymap(km) => {
                            self.keymap = Some(km.clone());
                        }
                        _ => {}
                    }
                }
            }
            evs.clear();
            core.take_events(&mut evs);
            let mut any = false;
            for (id, offset) in scroll_echoes.drain(..) {
                any |= core.component_event(id, EventKind::Scroll, &EventPayload::Scroll(offset));
            }
            for ev in evs.drain(..) {
                if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                    any |= core.component_event(id, kind, &payload);
                }
            }
            if batch.is_none() && !any {
                break;
            }
        }
        any_commit
    }

    /// Collects all live text nodes reachable from `ROOT_ID` (including overlays/popovers).
    fn all_live_texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![ROOT_ID];
        while let Some(id) = stack.pop() {
            if let Some(s) = self.texts.get(&id) {
                out.push(s.clone());
            }
            if let Some(kids) = self.children.get(&id) {
                for &child in kids.iter().rev() {
                    stack.push(child);
                }
            }
        }
        out
    }

    fn has_text(&self, needle: &str) -> bool {
        self.all_live_texts().iter().any(|t| t == needle)
    }

    fn has_text_containing(&self, needle: &str) -> bool {
        self.all_live_texts().iter().any(|t| t.contains(needle))
    }

    /// Finds a live text node matching `predicate` and dispatches `Press` on its parent
    /// view (or up to 2 levels higher if the immediate parent doesn't trigger a state update).
    fn press_matching(
        &mut self,
        core: &mut Core<ApproxTextEngine>,
        predicate: impl Fn(&str) -> bool,
    ) -> bool {
        let target = self
            .live_node_ids()
            .into_iter()
            .find(|id| self.texts.get(id).is_some_and(|s| predicate(s)));
        let Some(text_id) = target else {
            return false;
        };
        let mut cur = match self.parent.get(&text_id).copied() {
            Some(p) => p,
            None => return false,
        };
        for _ in 0..3 {
            core.component_event(
                cur,
                EventKind::Press,
                &EventPayload::Press(PressEvent::default()),
            );
            if self.pump(core) {
                return true;
            }
            let Some(&next) = self.parent.get(&cur) else {
                break;
            };
            cur = next;
        }
        true
    }

    fn press_text(&mut self, core: &mut Core<ApproxTextEngine>, exact: &str) -> bool {
        self.press_matching(core, |s| s == exact)
    }

    fn press_label(&mut self, core: &mut Core<ApproxTextEngine>, exact: &str) -> bool {
        let Some(target) = self
            .live_node_ids()
            .into_iter()
            .find(|id| self.labels.get(id).and_then(|l| l.as_deref()) == Some(exact))
        else {
            return false;
        };
        core.component_event(
            target,
            EventKind::Press,
            &EventPayload::Press(PressEvent::default()),
        );
        self.pump(core);
        true
    }

    fn live_node_ids(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![ROOT_ID];
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(kids) = self.children.get(&id) {
                for &child in kids.iter().rev() {
                    stack.push(child);
                }
            }
        }
        out
    }

    fn press_escape(&mut self, core: &mut Core<ApproxTextEngine>, editing: bool) -> bool {
        let Some(km) = self.keymap.as_ref() else {
            return false;
        };
        let stroke = nsh_core::keymap::Keystroke::parse_with("escape", true).unwrap();
        let mut focus = nsh_core::keymap::KeyFocus::default();
        focus.editing = editing;
        let Some(id) = km.find_with(ROOT_ID, stroke, focus, false).fired() else {
            return false;
        };
        core.component_event(
            ROOT_ID,
            EventKind::KeyCommand,
            &EventPayload::Value(nsh_core::PayloadValue::Number(id as f64)),
        );
        self.pump(core);
        true
    }
}

fn mount<C: Component + 'static>(size: Size<Points>) -> (Core<ApproxTextEngine>, HeadlessTree) {
    let mut core = Core::new(ApproxTextEngine, size);
    let mut registry = Registry::new();
    registry.register::<C>();
    core.set_components(registry);
    for op in [
        Op::Create {
            id: 2,
            tag: Tag::View,
        },
        Op::SetProp {
            id: 2,
            prop: Prop::Component(std::any::type_name::<C>().into()),
        },
        Op::Insert {
            parent: ROOT_ID,
            child: 2,
            before: None,
        },
    ] {
        core.apply(op).expect("apply op");
    }
    let mut tree = HeadlessTree::default();
    tree.pump(&mut core);
    (core, tree)
}

fn provide_base_contexts(lang: Lang) -> (AppearancePrefs, Signal<FilterSuggestionsCatalog>) {
    provide_context(signal(Some(lang)));
    provide_mono_font(false);
    let appearance = provide_appearance_prefs(config::ThemeMode::System, 100);
    let catalog = signal(FilterSuggestionsCatalog::default());
    provide_context(catalog);
    (appearance, catalog)
}

fn make_controls(tag: &str) -> Arc<Controls> {
    reset_test_config();
    let n = TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("zniff-v05-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(crate::ca::Ca::open(&dir).expect("ca"));
    let hub = Arc::new(Hub::new(ca).expect("hub"));
    let _ = std::fs::remove_dir_all(&dir);
    Arc::new(Controls {
        hub,
        capture: Mutex::new(None),
        status: Mutex::new(StatusMsg::Idle),
        running: Mutex::new(false),
        listen: Mutex::new("127.0.0.1:8877".into()),
        listen_port: Mutex::new(8877),
        ca_status: Mutex::new(crate::ca::CaStatus::NotInstalled),
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
        relay_push: Mutex::new(None),
        toast_seq: std::sync::atomic::AtomicU64::new(0),
        toast_queue: Mutex::new(Vec::new()),
        config_error: Mutex::new(None),
        config_path: Mutex::new(String::new()),
        pending_replay_diff: Mutex::new(None),
        pending_hook_run: Mutex::new(None),
        session_tabs: Mutex::new(SessionTabsStore::new()),
    })
}

fn make_profiles() -> ProfileSignals {
    ProfileSignals {
        saved: signal(Vec::new()),
        history: signal(Vec::new()),
        name: signal(String::new()),
        filter: signal(String::new()),
        filter_enabled: signal(true),
        bypass: signal(String::new()),
        rules: signal(String::new()),
        rule_groups: signal(Vec::new()),
        editing_group: signal(None),
        focused_rule_line: signal(None),
        upstream: signal(String::new()),
        rules_on: signal(true),
        breaks_on: signal(true),
        show_tunnels: signal(false),
        proxy_auth: signal(String::new()),
        allow_hosts: signal(String::new()),
        allow_hosts_strict: signal(false),
        allow_hosts_locked: signal(false),
        allow_denied: signal(allow_hosts::AllowDeniedSummary::default()),
        relay_url: signal(String::new()),
        relay_accept: signal(false),
        list_width: signal(660.0),
        list_height: signal(320.0),
        sidebar_width: signal(SIDEBAR_WIDTH_DEFAULT),
        sidebar_collapsed: signal(false),
        layout_mode: signal(config::LayoutMode::Split),
        panel_open: signal(false),
        rules_panel_tab: signal(0),
        filter_panel_open: signal(false),
        preset_panel_open: signal(false),
        import_open: signal(false),
        access_open: signal(false),
        quick_rule_prefill: signal(None),
        machine_settings_open: signal(false),
        settings_open: signal(false),
        settings_tab: signal(SettingsTab::General),
        shortcuts_open: signal(false),
        marker_modal_open: signal(false),
        watch_open: signal(false),
        about_open: signal(false),
        default_json_tree: signal(false),
        show_tray: signal(true),
        minimize_to_tray: signal(false),
        onboarding_strip_dismissed: signal(false),
        deselect_rev: signal(0),
    }
}

fn make_compose() -> Compose {
    Compose {
        composing: signal(false),
        response: signal(false),
        paused_id: signal(None),
        method: signal(String::new()),
        url: signal(String::new()),
        headers: signal(String::new()),
        body: signal(String::new()),
        status: signal(String::new()),
        draft_saved: signal((String::new(), String::new(), String::new(), String::new())),
        recent: signal(Vec::new()),
        inline_mock: InlineMockSignals::new(),
    }
}

fn make_seq_builder() -> SeqBuilder {
    let seq_diagram = signal(present::SequenceDiagramData::default());
    let seq_viewport_w = signal(0.0_f32);
    SeqBuilder {
        open: signal(false),
        items: signal(Vec::new()),
        resume_ids: signal(Vec::new()),
        resume_view_mode: signal(0),
        include_response: signal(true),
        include_query: signal(false),
        output: signal(String::new()),
        diagram: seq_diagram,
        view_mode: signal(0),
        source_flowchart: signal(false),
        hover_bar: signal(None),
        redact_open: signal(false),
        redact_export: signal(false),
        redact_strict: signal(false),
        redact_fields: signal(String::new()),
        redact_flow_id: signal(None),
        viewport_w: seq_viewport_w,
        track_w: signal(0.0_f32),
        layout: computed(move || seq_diagram.with(|d| swimlane_layout(d, seq_viewport_w.get()))),
        flat_rows: computed(move || seq_diagram.with(build_dep_flat_rows)),
    }
}

fn make_ca_signals(trusted: bool) -> CaSignals {
    CaSignals {
        status: signal(if trusted {
            ca::CaStatus::Trusted
        } else {
            ca::CaStatus::NotInstalled
        }),
        active_external: signal(None),
        detected_external: signal(Vec::new()),
    }
}

fn make_rule_draft() -> QuickRuleDraftSignals {
    QuickRuleDraftSignals {
        kind: signal(0_u8),
        host: signal(String::new()),
        path: signal(String::new()),
        op: signal(String::new()),
        from: signal(String::new()),
        to: signal(String::new()),
        modes: QuickRuleModeSignals::new(),
        editing_line: signal(None),
    }
}

fn make_shown(url: &str, status: Option<u16>, error: &str) -> Shown {
    Shown {
        phase: signal(Phase::Ready),
        method: signal("GET".into()),
        url: signal(url.into()),
        protocol: signal("HTTP/1.1".into()),
        rules: signal(String::new()),
        rule_hits: signal(Vec::new()),
        gql_badge: signal(String::new()),
        status: signal(status),
        error: signal(error.into()),
        request_headers: signal(Vec::new()),
        response_headers: signal(Vec::new()),
        request_body: signal(String::new()),
        response_body: signal(r#"{"ok":true}"#.into()),
        request_body_bytes: signal(0),
        response_body_bytes: signal(11),
        request_hex: signal(Vec::new()),
        response_hex: signal(Vec::new()),
        pane_width: signal(520.0),
        request_language: signal(String::new()),
        response_language: signal("json".into()),
        request_doc: signal(None),
        response_doc: signal(None),
        req_doc_preview: signal(false),
        res_doc_preview: signal(false),
        request_json: signal(None),
        response_json: signal(None),
        req_json_tree: signal(false),
        res_json_tree: signal(false),
        request_form: signal(Vec::new()),
        request_note: signal(String::new()),
        response_note: signal(String::new()),
        hook_cards: signal(Vec::new()),
        req_hook_cards: signal(Vec::new()),
        res_hook_cards: signal(Vec::new()),
        metrics: signal(String::new()),
        timing_bar: signal(present::TimingBreakdown::default()),
        query: signal(Vec::new()),
        preview: signal(String::new()),
        preview_w: signal(0.0),
        preview_h: signal(0.0),
        image_caption: signal(String::new()),
        pretty: signal(true),
        default_json_tree: signal(false),
        messages: signal(Vec::new()),
        llm_stream: signal(None),
        msg_llm_view: signal(true),
        diff: signal(String::new()),
        diff_old: signal(String::new()),
        diff_new: signal(String::new()),
        diff_baseline_label: signal(String::new()),
        diff_scope: signal(0_u8),
        pinned: signal(false),
        is_marker: signal(false),
        comment: signal(String::new()),
        editing_comment: signal(false),
        repeating: signal(false),
        repeat_count: signal("1".into()),
        repeat_interval: signal("0".into()),
        repeat_concurrency: signal("1".into()),
        req_search: signal(String::new()),
        res_search: signal(String::new()),
        msg_search: signal(String::new()),
        msg_dir: signal(0),
        msg_kind: signal(0),
        relay: signal(String::new()),
        request_offset: signal(0),
        response_offset: signal(0),
        request_window: signal(TextWindow::default()),
        response_window: signal(TextWindow::default()),
        req_headers_loading: signal(false),
        res_headers_loading: signal(false),
        req_body_loading: signal(false),
        res_body_loading: signal(false),
        messages_loading: signal(false),
        diff_loading: signal(false),
    }
}

fn seed_http_flow(controls: &Controls, host: &str, path: &str, status: u16, body: &[u8]) -> u64 {
    let doc = serde_json::json!({
        "log": {
            "version": "1.2",
            "entries": [{
                "startedDateTime": "2026-11-21T10:00:00.000Z",
                "time": 42,
                "request": {
                    "method": "GET",
                    "url": format!("https://{host}{path}"),
                    "httpVersion": "HTTP/1.1",
                    "headers": [{"name": "Host", "value": host}],
                    "queryString": [],
                    "headersSize": -1,
                    "bodySize": 0
                },
                "response": {
                    "status": status,
                    "statusText": "OK",
                    "httpVersion": "HTTP/1.1",
                    "headers": [{"name": "Content-Type", "value": "application/json"}],
                    "content": {
                        "size": body.len(),
                        "mimeType": "application/json",
                        "text": String::from_utf8_lossy(body)
                    },
                    "headersSize": -1,
                    "bodySize": body.len()
                },
                "cache": {},
                "timings": {"send": 1, "wait": 20, "receive": 21}
            }]
        }
    });
    let imported = controls.hub.import_har(&doc);
    assert_eq!(imported, 1);
    controls
        .hub
        .with_session(|s| s.iter().last().map(|f| f.id).expect("seeded flow"))
}

// ---------------------------------------------------------------------------
// 1. A1: Filter popover, Rules popover, and Preset pill mount cleanly & independently
// ---------------------------------------------------------------------------

#[test]
fn a1_filter_rules_and_preset_popovers_mount_independently_zh_en() {
    #[component]
    fn A1Harness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("a1");
        let profiles = make_profiles();
        let draft = make_rule_draft();
        let step = signal(0_u8);

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                view()
                    .w_full()
                    .h(px(42.0))
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(saved_filter_menu(profiles, controls.clone(), draft))
                    .child(rules_menu_popover(profiles, controls.clone(), draft))
                    .child(preset_pill_popover(profiles, controls, draft)),
            )
            .child(
                view()
                    .on_press(move || {
                        let next = step.get() + 1;
                        step.set(next);
                        match next {
                            1 => {
                                profiles.panel_open.set(false);
                                profiles.preset_panel_open.set(false);
                                profiles.filter_panel_open.set(true);
                            }
                            2 => {
                                profiles.filter_panel_open.set(false);
                                profiles.preset_panel_open.set(false);
                                profiles.panel_open.set(true);
                            }
                            3 => {
                                profiles.filter_panel_open.set(false);
                                profiles.panel_open.set(false);
                                profiles.preset_panel_open.set(true);
                            }
                            _ => {
                                profiles.filter_panel_open.set(false);
                                profiles.panel_open.set(false);
                                profiles.preset_panel_open.set(false);
                            }
                        }
                    })
                    .child(text("STEP_A1")),
            )
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<A1Harness>(Size::new(px(1200.0), px(760.0)));

        // All 3 independent triggers render with localized labels.
        assert!(tree.has_text(t(lang, "过滤", "Filter")));
        assert!(tree.has_text(t(lang, "规则", "Rules")));
        assert!(tree.has_text(t(lang, "预设", "Presets")));

        // Step 1: Open Filter popover only.
        assert!(tree.press_text(&mut core, "STEP_A1"));
        assert!(
            tree.has_text_containing(t(lang, "域名", "Host")),
            "Filter panel should render in {lang:?}"
        );

        // Step 2: Switch to Rules popover only.
        assert!(tree.press_text(&mut core, "STEP_A1"));
        assert!(
            tree.has_text(t(lang, "直通与上游", "Bypass & Proxy")),
            "Rules panel should render in {lang:?}"
        );

        // Step 3: Switch to Preset pill popover only.
        assert!(tree.press_text(&mut core, "STEP_A1"));
        assert!(
            tree.has_text(t(lang, "配置预设", "Presets")) || tree.has_text(t(lang, "保存", "Save")),
            "Preset panel should render in {lang:?}"
        );

        // Step 4: Close all cleanly without disposed-signal panics.
        assert!(tree.press_text(&mut core, "STEP_A1"));
    }
}

// ---------------------------------------------------------------------------
// 2. A2 + A3: All 6 Settings tabs & slimmed More menu (Export / Tools / Help <= 6 items)
// ---------------------------------------------------------------------------

#[test]
fn a2_settings_all_six_tabs_render_and_switch_cleanly_zh_en() {
    #[component]
    fn SettingsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("a2-settings");
        let profiles = make_profiles();
        let props = SettingsWindowProps {
            profiles,
            controls,
            ca_ui: make_ca_signals(false),
            running: signal(false),
            listen: signal(String::new()),
            proxy_on: signal(false),
            port_text: signal("8877".into()),
            listen_all: signal(false),
            pretty: signal(true),
            seq_builder: make_seq_builder(),
            rows: signal(Vec::new()),
            compose: make_compose(),
            selected: signal(None),
            shown_phase: signal(Phase::Empty),
            update_state: signal(UpdateState::default()),
            install_ca: Rc::new(|| {}),
            remove_ca: Rc::new(|| {}),
            toggle_proxy: Rc::new(|| {}),
        };
        view()
            .w(px(740.0))
            .h(px(540.0))
            .child(settings_shell(props, false))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SettingsHarness>(Size::new(px(740.0), px(540.0)));

        // Every tab label must appear in the sidebar and mount cleanly when selected.
        let expected_tab_content: &[(SettingsTab, &str, &str)] = &[
            (SettingsTab::General, "外观与语言", "Appearance & Language"),
            (
                SettingsTab::Certificate,
                "HTTPS 根证书",
                "HTTPS Root Certificate",
            ),
            (SettingsTab::Network, "监听地址", "Capture Listener"),
            (SettingsTab::Security, "代理认证", "Proxy auth"),
            (SettingsTab::Relay, "中继", "Relay"),
            (SettingsTab::Updates, "版本", "Version"),
        ];

        for &(tab, zh_marker, en_marker) in expected_tab_content {
            let (zh_lbl, en_lbl) = tab.label();
            let lbl = t(lang, zh_lbl, en_lbl);
            assert!(
                tree.press_text(&mut core, lbl),
                "should press settings tab {lbl} in {lang:?}"
            );
            let marker = t(lang, zh_marker, en_marker);
            assert!(
                tree.has_text_containing(marker),
                "settings tab {tab:?} should render '{marker}' in {lang:?}; got {:?}",
                tree.all_live_texts()
            );
        }
    }
}

#[test]
fn a3_more_menu_has_three_groups_each_at_most_six_items_zh_en() {
    #[component]
    fn MoreMenuHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("a3-more");
        let profiles = make_profiles();
        let open = signal(true);
        view().w(px(360.0)).h(px(720.0)).child(more_menu(
            open,
            || {},
            || {},
            || {},
            controls,
            signal(Vec::new()),
            profiles.filter,
            signal(Phase::Empty),
            signal(String::new()),
            profiles,
            make_compose(),
            make_seq_builder(),
            signal(None),
            signal(UpdateState::default()),
        ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (_core, tree) = mount::<MoreMenuHarness>(Size::new(px(360.0), px(720.0)));

        let g1 = t(lang, "导出", "Export");
        let g2 = t(lang, "工具", "Tools");
        let g3 = t(lang, "帮助", "Help");
        assert!(tree.has_text(g1), "missing Export group in {lang:?}");
        assert!(tree.has_text(g2), "missing Tools group in {lang:?}");
        assert!(tree.has_text(g3), "missing Help group in {lang:?}");

        // Locate the items container (parent of the group label's wrapper view) and count rows per group.
        let g1_text_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.texts.get(id).is_some_and(|s| s == g1))
            .expect("group 1 label");
        let g1_wrapper = tree.parent[&g1_text_id];
        let items_container = tree.parent[&g1_wrapper];
        let children = &tree.children[&items_container];

        let mut counts = Vec::new();
        let mut current: Option<usize> = None;
        for &child in children {
            let child_texts = {
                let mut out = Vec::new();
                let mut stack = vec![child];
                while let Some(id) = stack.pop() {
                    if let Some(s) = tree.texts.get(&id) {
                        out.push(s.as_str());
                    }
                    if let Some(kids) = tree.children.get(&id) {
                        stack.extend(kids.iter().copied());
                    }
                }
                out
            };
            if child_texts.len() == 1
                && matches!(child_texts[0], s if s == g1 || s == g2 || s == g3)
            {
                if let Some(c) = current.replace(0) {
                    counts.push(c);
                }
            } else if !child_texts.is_empty()
                && let Some(c) = current.as_mut()
            {
                *c += 1;
            }
        }
        if let Some(c) = current {
            counts.push(c);
        }
        assert_eq!(counts.len(), 3, "expected 3 menu groups, got {counts:?}");
        for (idx, count) in counts.iter().enumerate() {
            assert!(
                *count <= 6,
                "More menu group {} has {} items (> 6) in {lang:?}",
                idx + 1,
                count
            );
        }
    }
}

#[test]
fn more_menu_clear_unstarred_requires_two_step_confirm_and_disables_when_empty_zh_en() {
    fn sample_unpinned_row(id: u64) -> Arc<RowLine> {
        Arc::new(RowLine {
            id,
            method: "GET".into(),
            status: Some(200),
            time: "00:00:00".into(),
            gap_nanos: 0,
            clock: String::new(),
            elapsed: "1ms".into(),
            elapsed_nanos: Some(1_000_000),
            elapsed_slow: false,
            host: "api.example.com".into(),
            client_ip: "127.0.0.1".into(),
            path: "/v1/ping".into(),
            host_path: "api.example.com/v1/ping".into(),
            url: "https://api.example.com/v1/ping".into(),
            comment: String::new(),
            protocol: "HTTP/1.1".into(),
            size: "12B".into(),
            size_bytes: 12,
            failed: false,
            pinned: false,
            is_marker: false,
            rule_marker: String::new(),
            rule_summary: String::new(),
            relay_short: String::new(),
        })
    }

    #[component]
    fn ClearGuardHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("more-clear-guard");
        let profiles = make_profiles();
        let open = signal(true);
        let rows = signal(Vec::<Arc<RowLine>>::new());
        let cleared = signal(0_usize);
        let add_row = rows;
        let check_cleared = cleared;

        view()
            .w(px(360.0))
            .h(px(760.0))
            .flex_col()
            .child(
                view()
                    .on_press(move || add_row.set(vec![sample_unpinned_row(1)]))
                    .child(text("SEED_UNPINNED")),
            )
            .child(text(move || {
                format!("CLEARED:{}:OPEN:{}", check_cleared.get(), open.get())
            }))
            .child(more_menu(
                open,
                || {},
                || {},
                move || cleared.update(|n| *n += 1),
                controls,
                rows,
                profiles.filter,
                signal(Phase::Empty),
                signal(String::new()),
                profiles,
                make_compose(),
                make_seq_builder(),
                signal(None),
                signal(UpdateState::default()),
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<ClearGuardHarness>(Size::new(px(360.0), px(760.0)));
        let clear_label = t(lang, "清空未星标", "Clear unstarred");
        let confirm_label = t(lang, "再次点击确认清空", "Click again to clear");

        // Disabled when no unpinned flows exist: pressing does not arm confirm or clear.
        assert!(tree.press_text(&mut core, clear_label));
        assert!(!tree.has_text(confirm_label));
        assert!(tree.has_text("CLEARED:0:OPEN:true"));

        // Seed 1 unpinned row -> first click arms 2-step confirmation and keeps menu open.
        assert!(tree.press_text(&mut core, "SEED_UNPINNED"));
        assert!(tree.press_text(&mut core, clear_label));
        assert!(tree.has_text(confirm_label));
        assert!(tree.has_text("CLEARED:0:OPEN:true"));

        // Second click executes clear and dismisses menu.
        assert!(tree.press_text(&mut core, confirm_label));
        assert!(tree.has_text("CLEARED:1:OPEN:false"));
    }
}

// ---------------------------------------------------------------------------
// 3. B1 + B2 + B3: Detail rule hit badges, rule line hit counts, and dry-run preview
// ---------------------------------------------------------------------------

#[test]
fn b1_b2_b3_rule_badges_line_hits_and_dry_run_zh_en() {
    #[component]
    fn RuleInsightsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        let (_, catalog) = provide_base_contexts(lang);
        let controls = make_controls("b1b2b3");
        seed_http_flow(
            &controls,
            "api.example.com",
            "/v1/orders",
            200,
            br#"{"id":1}"#,
        );
        let profiles = make_profiles();
        profiles
            .rules
            .set("map-local api.example.com/v1 status=201\ndelay nomatch.test 100ms".into());

        // Populate hit counts: line 1 has 2 hits, line 2 has 0 hits.
        let mut cat = FilterSuggestionsCatalog::default();
        cat.rule_line_hits.insert((None, 1), 2);
        catalog.set(cat);

        let shown = make_shown("https://api.example.com/v1/orders", Some(201), "");
        shown.rule_hits.set(vec![session::RuleHit {
            verb: "map-local".into(),
            group_name: None,
            line_no: 1,
        }]);

        let target = RuleEditTarget::from_profiles(profiles);
        let draft = make_rule_draft();
        let dry_draft = signal(Option::<String>::None);
        let step = signal(0_u8);

        view()
            .w(px(680.0))
            .h(px(480.0))
            .flex_col()
            .gap_2()
            .child(detail_rule_badges(shown, profiles, controls.clone()))
            .child(rule_line_strip(
                target,
                controls.clone(),
                draft,
                signal(false),
            ))
            .child(rule_dry_run_strip(target, controls, move || {
                dry_draft.get()
            }))
            .child(text(move || {
                format!(
                    "OPEN:{} FOCUS:{:?}",
                    profiles.panel_open.get(),
                    profiles.focused_rule_line.get()
                )
            }))
            .child(
                view()
                    .on_press(move || {
                        let next = step.get() + 1;
                        step.set(next);
                        if next == 1 {
                            // Switch dry-run draft to a 0-match rule.
                            dry_draft.set(Some("delay never-matches.invalid 50ms".into()));
                        } else {
                            dry_draft.set(None);
                        }
                    })
                    .child(text("TOGGLE_DRY_RUN")),
            )
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<RuleInsightsHarness>(Size::new(px(680.0), px(480.0)));

        // B1: Rule hit badge renders and clicking it opens rules panel focused on line 1.
        assert!(
            tree.has_text_containing("map-local"),
            "B1 badge missing in {lang:?}"
        );
        assert!(tree.press_matching(&mut core, |s| s.contains("map-local") && s.contains("#1")));
        assert!(
            tree.has_text_containing("OPEN:true FOCUS:Some((None, 1))"),
            "clicking B1 badge should focus line 1; texts={:?}",
            tree.all_live_texts()
        );

        // B2: Rule line strip renders line previews and hit count badge (×2 vs 0).
        assert!(tree.has_text_containing("#1 map-local api.example.com/v1"));
        assert!(tree.has_text_containing("#2 delay nomatch.test"));
        assert!(tree.has_text("×2"), "line 1 hit count '×2' should render");
        assert!(tree.has_text("0"), "line 2 hit count '0' should render");

        // B3: Dry-run matching case (while line 1 is active).
        assert!(
            tree.has_text_containing(t(lang, "1 / 1", "1 of 1")),
            "B3 matching dry-run should show 1/1 in {lang:?}; texts={:?}",
            tree.all_live_texts()
        );

        // Toggle comment on a rule line cleanly.
        assert!(tree.press_text(&mut core, "✓"));
        assert!(tree.has_text("#"), "comment toggle should flip ✓ to #");

        // B3: Dry-run 0-match warning case.
        assert!(tree.press_text(&mut core, "TOGGLE_DRY_RUN"));
        assert!(
            tree.has_text_containing(t(lang, "0 条命中", "0 matches")),
            "B3 zero-match dry-run warning should render in {lang:?}; texts={:?}",
            tree.all_live_texts()
        );
    }
}

// ---------------------------------------------------------------------------
// 4. C1 + C2 + C3: ERR repair actions, batch TLS failure banner, 4-step checklist
// ---------------------------------------------------------------------------

#[test]
fn c1_c2_c3_error_actions_tls_banner_and_onboarding_checklist_zh_en() {
    #[component]
    fn RecoveryAndChecklistHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        let ca_trusted = TEST_CA_TRUSTED.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("c1c2c3");
        let profiles = make_profiles();

        let shown = make_shown(
            "https://pinning.example.com/api",
            None,
            "client TLS handshake failed: unknown ca",
        );
        let burst = signal(TlsFailureBurst {
            failure_count: 4,
            distinct_hosts: 2,
            latest_flow_id: 9,
        });
        let dismissed = signal(false);
        let ca_ui = make_ca_signals(ca_trusted);
        let selected = signal(None);

        provide_context(TlsFailureBannerSignals {
            burst,
            dismissed,
            ca_status: ca_ui.status,
            controls: controls.clone(),
            selected,
            install_ca: Rc::new(|| {}),
        });

        let running = signal(true);
        let traffic = signal(OnboardingTraffic {
            total_flows: 2,
            has_decrypted_https: ca_trusted,
            has_tls_error: !ca_trusted,
        });
        let onboarding_props = OnboardingProps {
            controls: controls.clone(),
            running,
            listen: signal("127.0.0.1:8877".into()),
            ca_ui,
            use_system_proxy: signal(false),
            adb_count: signal(0),
            flow_total: signal(2),
            onboarding_traffic: traffic,
            compose: make_compose(),
            selected,
            profiles,
            install_ca: Rc::new(|| {}),
            toggle_system_proxy: Rc::new(|| {}),
            qr_open: signal(false),
            pending_qr: signal(false),
        };

        let mode = signal(0_u8);
        view()
            .w(px(640.0))
            .h(px(640.0))
            .flex_col()
            .gap_2()
            .child(tls_failure_banner_from_context())
            .child(error_summary_banner(shown, controls, profiles))
            .child(onboarding_checklist(&onboarding_props))
            .child(text(move || format!("BYPASS:{}", profiles.bypass.get())))
            .child(
                view()
                    .on_press(move || {
                        let next = mode.get() + 1;
                        mode.set(next);
                        if next == 1 {
                            shown.status.set(None);
                            shown.error.set("connection refused".into());
                        } else {
                            shown.status.set(Some(407));
                            shown.error.set(String::new());
                        }
                    })
                    .child(text("NEXT_ERR_KIND")),
            )
    }

    for lang in [Lang::Zh, Lang::En] {
        for ca_trusted in [false, true] {
            TEST_LANG.set(lang);
            TEST_CA_TRUSTED.set(ca_trusted);
            let (mut core, mut tree) =
                mount::<RecoveryAndChecklistHarness>(Size::new(px(640.0), px(640.0)));

            // C2: Batch TLS failure banner renders.
            assert!(
                tree.has_text_containing(t(
                    lang,
                    "检测到 4 次 TLS 握手失败",
                    "Detected 4 TLS handshake failures"
                )),
                "C2 banner should render in {lang:?} (ca_trusted={ca_trusted})"
            );

            // C1: TLS trust error actions render; pressing Bypass TLS appends host.
            assert!(tree.has_text(t(lang, "加入直通", "Bypass TLS")));
            assert!(tree.press_text(&mut core, t(lang, "加入直通", "Bypass TLS")));
            assert!(
                tree.has_text_containing("BYPASS:pinning.example.com"),
                "Bypass TLS should update bypass list"
            );
            assert!(tree.has_text(t(lang, "已在直通名单", "In bypass list")));

            // Switch to NetworkUpstream error -> Check Upstream & Add resolve.
            assert!(tree.press_text(&mut core, "NEXT_ERR_KIND"));
            assert!(tree.has_text(t(lang, "检查上游", "Check Upstream")));
            assert!(tree.has_text(t(lang, "添加 resolve", "Add resolve")));

            // Switch to HTTP 407 ProxyAuth -> Open Security.
            assert!(tree.press_text(&mut core, "NEXT_ERR_KIND"));
            assert!(tree.has_text(t(lang, "打开安全设置", "Open Security")));

            // C3: Verifiable checklist progress renders (2/3 when only HTTP + proxy running,
            // 3/3 when decrypted HTTPS arrives), while Step 2 stays informational (Mobile QR +
            // explanation why desktop cannot verify remote trust).
            assert!(
                tree.has_text(t(lang, "手机扫码", "Mobile QR")),
                "Mobile QR chip should render in Step 2 (ca_trusted={ca_trusted})"
            );
            assert!(
                tree.has_text_containing(t(
                    lang,
                    if ca_trusted {
                        "无法由本机检测是否已信任"
                    } else {
                        "客户端信任状态将在首条 HTTPS 时验证"
                    },
                    if ca_trusted {
                        "cannot be detected from host"
                    } else {
                        "verified on first HTTPS"
                    },
                )),
                "Step 2 explanation hint missing in {lang:?} (ca_trusted={ca_trusted}); texts={:?}",
                tree.all_live_texts()
            );
            let expected_progress = if ca_trusted { "3/3" } else { "2/3" };
            assert!(
                tree.has_text_containing(expected_progress),
                "C3 checklist progress ({expected_progress}) missing in {lang:?}; texts={:?}",
                tree.all_live_texts()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5. D1 + D2 + D4 + D5 + D6–D8: Empty diag, autocomplete/syntax, shortcuts, toast, appearance
// ---------------------------------------------------------------------------

#[test]
fn d1_d2_d4_d5_d6_d8_ux_polish_features_zh_en() {
    #[component]
    fn UxPolishHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        let (appearance, _) = provide_base_contexts(lang);
        let controls = make_controls("d-polish");

        // D1: Empty filter diagnosis.
        let eff_filter = signal("status:404".to_string());
        let main_filter = signal("status:404".to_string());
        let filter_enabled = signal(true);
        let list_filter = signal(String::new());
        let diagnosis = signal(EmptyFilterDiagnosis {
            total_flows: 5,
            blocking_tokens: vec!["status:404".into()],
            combined_mismatch: false,
            both_filters_active: false,
            primary_empty: false,
            secondary_empty: false,
        });

        // D2: Filter autocomplete + syntax error highlight.
        let filter_val = signal("status:bogus".to_string());
        let focus_ref = element_ref();

        // D4: Shortcuts modal.
        let shortcuts_open = signal(true);

        // D5: 5s Undo Toast with real undoable cleared flows.
        seed_http_flow(&controls, "undo.example.com", "/v1/a", 200, b"{}");
        let cleared = controls.hub.clear_session_with_undo();
        assert_eq!(cleared, 1);
        let toast_manager = stored(ToastManager::<u64, StatusMsg>::new(ToastMotion::sonner()));
        let initial_toasts = toast_manager.with(|m| {
            let now = nsh_core::time::now();
            m.push(
                1_u64,
                StatusMsg::ClearedFlows { count: cleared },
                ToastOptions::timeout(Some(Duration::from_millis(5000))),
                now,
            );
            m.visible(3)
                .map(|(id, data, status)| (*id, data.clone(), status))
                .collect::<Vec<_>>()
        });
        let toast_shown = signal(initial_toasts);
        let toast_gen = signal(1_u64);
        let toast_state = ToastStackState::new();
        let hub_for_check = controls.hub.clone();

        view()
            .w(px(900.0))
            .h(px(720.0))
            .flex_col()
            .gap_2()
            .child(list_empty(EmptyFilterState {
                effective_filter: eff_filter,
                main_filter,
                filter_enabled,
                list_filter,
                diagnosis,
                running: signal(true),
                on_clear_primary: None,
            }))
            .child(filter_assist_input(
                filter_val,
                tr("过滤流量", "Filter traffic"),
                28.0,
                24.0,
                focus_ref,
            ))
            .child(filter_syntax_error_strip(filter_val))
            .child(menu_theme_row(controls.clone()))
            .child(menu_zoom_row(controls.clone()))
            .child(text(move || {
                let toast_ending = toast_shown.with(|items| {
                    items
                        .iter()
                        .all(|(_, _, st)| *st == ToastTransitionStatus::Ending)
                });
                let flow_count = hub_for_check.with_session(|s| s.len());
                format!(
                    "THEME:{:?} ZOOM:{} SHORTCUTS:{} UNDO_OK:{}:{}",
                    appearance.theme_mode.get(),
                    appearance.ui_zoom.get(),
                    shortcuts_open.get(),
                    toast_ending,
                    flow_count
                )
            }))
            .child(shortcuts_modal(shortcuts_open))
            .child(toast_overlay(
                toast_shown,
                toast_gen,
                toast_state,
                toast_manager,
                controls,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<UxPolishHarness>(Size::new(px(900.0), px(720.0)));

        // D1: Diagnosis text renders.
        assert!(
            tree.has_text_containing("status:404"),
            "D1 diagnosis should mention status:404 in {lang:?}"
        );

        // D2: Syntax error strip highlights invalid status code.
        assert!(
            tree.has_text_containing("status:bogus")
                || tree.has_text_containing(t(lang, "状态码", "status")),
            "D2 syntax error strip should render in {lang:?}; texts={:?}",
            tree.all_live_texts()
        );

        // D4: Shortcuts modal renders & closes cleanly.
        assert!(tree.has_text(t(lang, "快捷键速查", "Keyboard Shortcuts")));
        assert!(tree.press_text(&mut core, t(lang, "关闭", "Close")));
        assert!(tree.has_text_containing("SHORTCUTS:false"));

        // D5: Undo toast renders & pressing Undo restores cleared flows and starts exit transition.
        assert!(tree.has_text(t(lang, "撤销", "Undo")));
        assert!(tree.press_text(&mut core, t(lang, "撤销", "Undo")));
        assert!(
            tree.has_text_containing("UNDO_OK:true:1"),
            "D5 pressing Undo should restore cleared flow and dismiss toast; texts={:?}",
            tree.all_live_texts()
        );

        // D6–D8: Theme mode & zoom controls update appearance signals cleanly.
        assert!(tree.press_text(&mut core, t(lang, "深色", "Dark")));
        assert!(tree.has_text_containing("THEME:Dark"));
        assert!(tree.press_text(&mut core, "+"));
        assert!(tree.has_text_containing("ZOOM:110"));
        assert!(tree.has_text("110%"));
        assert!(tree.press_text(&mut core, "110%"));
        assert!(tree.has_text_containing("ZOOM:100"));
    }
}

// ---------------------------------------------------------------------------
// 6. E1 + E3 + E4 + E5 + E6 + E10 + I3: Core v0.5 workflow UI components
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e1_e3_e4_e5_e6_v05_interactive_components_zh_en() {
    #[component]
    fn V05ComponentsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("e-v05");
        let flow_id = seed_http_flow(
            &controls,
            "api.example.com",
            "/v1/items",
            200,
            br#"{"items":[1,2]}"#,
        );
        let profiles = make_profiles();
        let compose = make_compose();
        let selected = signal(Some(flow_id));
        let shown = make_shown("https://api.example.com/v1/items", Some(200), "");

        // E4 rule edit target.
        let target = RuleEditTarget::from_profiles(profiles);

        // E5 marker modal open.
        profiles.marker_modal_open.set(true);

        // E6 watch popover open with 1 rule and 2 hits.
        profiles.watch_open.set(true);
        let watch_rules = signal(vec![config::WatchRule {
            name: "Errors".into(),
            filter: "status:5xx".into(),
            enabled: true,
        }]);
        let watch_eval = signal(crate::watch::WatchEvaluation {
            total_hits: 2,
            active_rules: 1,
            rules: vec![crate::watch::WatchRuleState {
                name: "Errors".into(),
                filter: "status:5xx".into(),
                enabled: true,
                hit_count: 2,
                recent_ids: vec![flow_id],
            }],
            new_alerts: Vec::new(),
        });

        let net_open = signal(true);
        let marker_count = signal(0_usize);
        let draft = make_rule_draft();
        let quick_open = signal(false);
        let count_controls = controls.clone();

        view()
            .w(px(960.0))
            .h(px(760.0))
            .flex_col()
            .gap_2()
            .child(inline_mock_trigger_button(
                shown,
                controls.clone(),
                selected,
                compose,
            ))
            .child(when(
                move || compose.inline_mock.open.get(),
                move || inline_mock_box(compose, controls.clone(), profiles),
            ))
            .child(toolbar_network_preset_popover(
                profiles,
                count_controls.clone(),
            ))
            .child(rule_templates_menu(
                target,
                count_controls.clone(),
                draft,
                quick_open,
            ))
            .child(watch_popover(WatchPopoverProps {
                open: profiles.watch_open,
                rules: watch_rules,
                eval: watch_eval,
                effective_filter: profiles.filter,
                list_filter: signal(String::new()),
                selected,
                active_tab: signal(LIVE_TAB_ID),
                baseline: signal(None),
                primary_filter: profiles.filter,
                filter_enabled: profiles.filter_enabled,
                controls: count_controls.clone(),
            }))
            .child(marker_modal(
                profiles.marker_modal_open,
                count_controls.clone(),
            ))
            .child(text(move || {
                let markers = count_controls
                    .hub
                    .with_session(|s| s.iter().filter(|f| f.is_marker()).count());
                let _ = marker_count.get();
                let _ = profiles.marker_modal_open.get();
                let _ = net_open.get();
                format!(
                    "RULES:{} MARKERS:{markers} MOCK_OPEN:{}",
                    profiles.rules.get(),
                    compose.inline_mock.open.get()
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<V05ComponentsHarness>(Size::new(px(960.0), px(760.0)));

        // E5: Marker modal renders and preset chip inserts a marker & closes modal.
        let preset_chip = t(lang, "开始测试", "Start test");
        assert!(
            tree.has_text(preset_chip),
            "E5 preset chip missing in {lang:?}"
        );
        assert!(tree.press_text(&mut core, preset_chip));
        assert!(
            tree.has_text_containing("MARKERS:1"),
            "E5 clicking preset should add marker flow; texts={:?}",
            tree.all_live_texts()
        );

        // E1: Press Mock trigger -> opens inline mock editor.
        assert!(tree.press_text(&mut core, "Mock"));
        assert!(tree.has_text_containing("MOCK_OPEN:true"));
        assert!(tree.has_text(t(lang, "内联 Mock 响应", "Inline Mock Response")));

        // E4: Open "+ Add rule" dropdown and click Dev CORS -> appends `cors *` to rules.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        let cors_label = t(lang, "开发期 CORS", "Dev CORS");
        assert!(
            tree.has_text(cors_label),
            "E4 template item missing after opening menu in {lang:?}"
        );
        assert!(tree.press_text(&mut core, cors_label));
        assert!(
            tree.has_text_containing("RULES:cors *"),
            "E4 template should append 'cors *'"
        );

        // E6: Watch popover renders rule name and hit count.
        assert!(
            tree.has_text_containing("Errors"),
            "E6 watch rule name should render in {lang:?}"
        );
    }
}

#[test]
fn e3_network_presets_apply_and_clear_managed_group_zh_en() {
    #[component]
    fn NetworkPresetHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("e3-net");
        let profiles = make_profiles();
        let step = signal(0_u8);
        let step_controls = controls.clone();

        view()
            .w(px(500.0))
            .h(px(300.0))
            .flex_col()
            .gap_2()
            .child(toolbar_network_preset_popover(profiles, controls))
            .child(
                view()
                    .on_press(move || {
                        let next = step.get() + 1;
                        step.set(next);
                        let preset = match next {
                            1 => NetworkPreset::Offline,
                            2 => NetworkPreset::Slow3g,
                            _ => NetworkPreset::None,
                        };
                        apply_network_preset(profiles, &step_controls, preset);
                    })
                    .child(text("STEP_NET")),
            )
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<NetworkPresetHarness>(Size::new(px(500.0), px(300.0)));

        // Step 1: Offline preset renders badge.
        assert!(tree.press_text(&mut core, "STEP_NET"));
        assert!(tree.has_text(t(lang, "离线", "Offline")));

        // Step 2: Slow 3G preset renders badge.
        assert!(tree.press_text(&mut core, "STEP_NET"));
        assert!(tree.has_text(t(lang, "弱网", "Slow 3G")));

        // Step 3: Reset to None hides badge cleanly.
        assert!(tree.press_text(&mut core, "STEP_NET"));
        assert!(!tree.has_text(t(lang, "弱网", "Slow 3G")));
    }
}

#[test]
fn e10_session_tabs_switch_close_and_cross_tab_baseline_diff_zh_en() {
    #[component]
    fn SessionTabsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("e10-tabs");
        let live_id = seed_http_flow(
            &controls,
            "live.example.com",
            "/v1/a",
            200,
            br#"{"env":"live"}"#,
        );

        // Open a read-only imported session tab.
        let mut imported_sess = session::Session::default();
        let imp_id = imported_sess.open(session::FlowDraft {
            protocol: session::Protocol::Http11,
            method: "GET".into(),
            host: "prod.example.com".into(),
            path: "/v1/a".into(),
            url: "https://prod.example.com/v1/a".into(),
            request_headers: vec![("Host".into(), "prod.example.com".into())],
        });
        imported_sess.set_response(
            imp_id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            Some(session::Protocol::Http11),
        );
        imported_sess.append_body(imp_id, Direction::Response, br#"{"env":"prod"}"#);
        imported_sess.mark_finished(imp_id);
        let tab_id = controls
            .session_tabs
            .lock()
            .unwrap()
            .open_session("prod.har".into(), imported_sess)
            .expect("open imported tab");

        // Set cross-tab baseline on Live flow while viewing imported tab.
        {
            let live_snap = controls
                .hub
                .with_session(|s| s.get(live_id).cloned().expect("live flow"));
            let mut store = controls.session_tabs.lock().unwrap();
            store.set_baseline_ref(LIVE_TAB_ID, live_snap);
        }

        let tabs = signal(session_tab_summaries(&controls, 1));
        let active_tab = signal(tab_id);
        let selected = signal(None);
        let baseline = signal(baseline_id_for_active_tab(&controls));
        let props = SessionTabBarProps {
            controls: controls.clone(),
            tabs,
            active_tab,
            selected,
            baseline,
            compose: make_compose(),
            seq_builder: make_seq_builder(),
        };
        provide_context(props);

        view()
            .w(px(700.0))
            .h(px(120.0))
            .flex_col()
            .child(session_tab_bar())
            .child(
                button()
                    .on_press({
                        let controls = controls.clone();
                        move |_: &()| tabs.set(session_tab_summaries(&controls, 1))
                    })
                    .child(text("btn_sync_tabs")),
            )
            .child(text(move || {
                let active = active_tab.get();
                let count = tabs.with(Vec::len);
                let base_host = resolve_baseline_flow(&controls, None, None)
                    .map(|f| f.host)
                    .unwrap_or_default();
                format!("ACTIVE:{active} TABS:{count} BASE:{base_host}")
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SessionTabsHarness>(Size::new(px(700.0), px(120.0)));

        // Both Live and imported read-only tab chips render, and cross-tab baseline resolves to live.example.com.
        assert!(tree.has_text_containing(t(lang, "实时会话 (1)", "Live (1)")));
        assert!(tree.has_text_containing("prod.har (1)"));
        assert!(tree.has_text(t(lang, "只读", "RO")));
        assert!(tree.has_text_containing("ACTIVE:1 TABS:2 BASE:live.example.com"));

        // Click Live tab chip -> switches back to tab 0 while keeping baseline intact.
        assert!(tree.press_matching(&mut core, |s| s.contains(t(lang, "实时会话", "Live"))));
        assert!(tree.has_text_containing("ACTIVE:0 TABS:2 BASE:live.example.com"));

        // Click the sibling close button next to the imported Tab: closes prod.har without switching into it.
        let ro_label_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| {
                tree.texts
                    .get(id)
                    .is_some_and(|s| s == t(lang, "只读", "RO"))
            })
            .expect("RO badge");
        let mut pill_id = ro_label_id;
        for _ in 0..3 {
            pill_id = tree.parent[&pill_id];
        }
        let close_wrapper_id = *tree.children[&pill_id]
            .last()
            .expect("close wrapper sibling next to tab");
        let close_btn_id = tree.children[&close_wrapper_id][0];
        core.component_event(
            close_btn_id,
            EventKind::Press,
            &EventPayload::Press(PressEvent::default()),
        );
        tree.pump(&mut core);
        assert!(tree.press_text(&mut core, "btn_sync_tabs"));
        assert!(
            tree.has_text_containing("ACTIVE:0 TABS:1 BASE:live.example.com"),
            "closing imported tab via sibling button must keep active tab 0 in {lang:?}; texts={:?}",
            tree.all_live_texts()
        );
        assert!(!tree.has_text_containing("prod.har (1)"));
    }
}

#[test]
fn i3_onboarding_card_loads_sample_har_traffic_zh_en() {
    #[component]
    fn SampleHarOnboardingHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("i3-sample");
        let profiles = make_profiles();
        let props = OnboardingProps {
            controls: controls.clone(),
            running: signal(true),
            listen: signal("127.0.0.1:8877".into()),
            ca_ui: make_ca_signals(false),
            use_system_proxy: signal(false),
            adb_count: signal(0),
            flow_total: signal(0),
            onboarding_traffic: signal(OnboardingTraffic::default()),
            compose: make_compose(),
            selected: signal(None),
            profiles,
            install_ca: Rc::new(|| {}),
            toggle_system_proxy: Rc::new(|| {}),
            qr_open: signal(false),
            pending_qr: signal(false),
        };
        let pane_w = signal(560.0_f32);
        let rev = signal(0_u64);
        let count_controls = controls;

        view()
            .w(px(640.0))
            .h(px(760.0))
            .flex_col()
            .child(onboarding_card(props, pane_w))
            .child(
                view()
                    .on_press(move || rev.set(rev.get() + 1))
                    .child(text(move || {
                        let _ = rev.get();
                        let count = count_controls.hub.with_session(|s| s.len());
                        format!("SAMPLE_FLOWS:{count}")
                    })),
            )
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) =
            mount::<SampleHarOnboardingHarness>(Size::new(px(640.0), px(760.0)));

        let tile_label = t(lang, "加载示例流量", "Load Sample Traffic");
        assert!(
            tree.has_text(tile_label),
            "I3 sample HAR tile missing in {lang:?}"
        );
        assert!(tree.has_text("SAMPLE_FLOWS:0"));

        assert!(tree.press_text(&mut core, tile_label));
        // Refresh counter text node.
        assert!(tree.press_matching(&mut core, |s| s.starts_with("SAMPLE_FLOWS:")));
        assert!(
            !tree.has_text("SAMPLE_FLOWS:0"),
            "I3 clicking Load Sample Traffic should import demo flows in {lang:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. allow_hosts denied summary: unlocked (+Allow visible) vs CLI-locked (read-only)
// ---------------------------------------------------------------------------

#[test]
fn allow_hosts_denied_summary_unlocked_and_locked_zh_en() {
    #[component]
    fn AllowDeniedHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        let locked = TEST_LOCKED.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("allow-denied");
        let denied = signal(allow_hosts::AllowDeniedSummary {
            total: 3,
            hosts: vec![
                ("blocked.example.com".into(), 2),
                ("other.example.com".into(), 1),
            ],
        });
        let allow_hosts_sig = signal("api.example.com".to_string());
        let locked_sig = signal(locked);

        view()
            .w(px(420.0))
            .h(px(260.0))
            .flex_col()
            .gap_2()
            .child(allow_denied_summary_section(
                denied,
                allow_hosts_sig,
                locked_sig,
                controls,
            ))
            .child(text(move || format!("ALLOW:{}", allow_hosts_sig.get())))
    }

    for lang in [Lang::Zh, Lang::En] {
        // Unlocked mode: +Allow visible, clicking it appends host, Reset clears summary.
        TEST_LANG.set(lang);
        TEST_LOCKED.set(false);
        let (mut core, mut tree) = mount::<AllowDeniedHarness>(Size::new(px(420.0), px(260.0)));

        assert!(tree.has_text(t(lang, "已拦截 3 次请求", "3 requests denied")));
        assert!(tree.has_text("blocked.example.com"));
        assert!(tree.has_text("×2"));
        assert!(tree.has_text(t(lang, "+ 允许", "+ Allow")));

        assert!(tree.press_text(&mut core, t(lang, "+ 允许", "+ Allow")));
        assert!(
            tree.has_text_containing("blocked.example.com")
                && tree.has_text_containing("ALLOW:api.example.com"),
            "unlocked +Allow should append host to allow_hosts"
        );
        assert_eq!(
            saved_config().allow_hosts,
            "api.example.com\nblocked.example.com"
        );
        assert!(!ca::data_dir().join("config.toml").exists());
        assert!(ca::data_dir().starts_with(std::env::temp_dir()));

        assert!(tree.press_text(&mut core, t(lang, "清零", "Reset")));
        assert!(!tree.has_text(t(lang, "已拦截 3 次请求", "3 requests denied")));

        // CLI-locked mode: read-only, +Allow must NOT render.
        TEST_LOCKED.set(true);
        let (_core_locked, tree_locked) =
            mount::<AllowDeniedHarness>(Size::new(px(420.0), px(260.0)));
        assert!(tree_locked.has_text(t(lang, "已拦截 3 次请求", "3 requests denied")));
        assert!(tree_locked.has_text("blocked.example.com"));
        assert!(
            !tree_locked.has_text(t(lang, "+ 允许", "+ Allow")),
            "CLI-locked mode must hide +Allow button in {lang:?}"
        );
    }
}

#[test]
fn allow_denied_summary_formats_thousands() {
    #[component]
    fn LargeAllowDeniedHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("allow-denied-large");
        let denied = signal(allow_hosts::AllowDeniedSummary {
            total: 1545,
            hosts: vec![("localhost".into(), 1435), ("www.google.com".into(), 44)],
        });
        view()
            .w(px(420.0))
            .h(px(220.0))
            .child(allow_denied_summary_section(
                denied,
                signal("api.example.com".to_string()),
                signal(false),
                controls,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (_core, tree) = mount::<LargeAllowDeniedHarness>(Size::new(px(420.0), px(220.0)));
        assert!(tree.has_text(t(lang, "已拦截 1,545 次请求", "1,545 requests denied")));
        assert!(tree.has_text("×1,435"));
        assert!(tree.has_text("×44"));
    }
}

#[test]
fn preset_sync_bar_surfaces_unsaved_state_and_opens_presets_zh_en() {
    #[component]
    fn SyncHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("preset-sync-bar");
        let profiles = make_profiles();
        let draft = make_rule_draft();
        profiles.saved.set(vec![config::SavedProfile {
            name: "test".into(),
            filter: "host:api.example.com".into(),
            bypass_hosts: String::new(),
            rules: String::new(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }]);
        profiles.name.set("test".into());
        profiles.filter.set("host:api.example.com".into());
        profiles.filter_panel_open.set(true);
        let step = signal(0_u8);

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                view()
                    .w_full()
                    .h(px(42.0))
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(saved_filter_menu(profiles, controls.clone(), draft))
                    .child(rules_menu_popover(profiles, controls.clone(), draft))
                    .child(preset_pill_popover(profiles, controls, draft)),
            )
            .child(
                view()
                    .on_press(move || {
                        let next = step.get() + 1;
                        step.set(next);
                        match next {
                            1 => {
                                profiles.filter.set("host:api.example.com path:/v2".into());
                            }
                            2 => {
                                profiles.filter_panel_open.set(false);
                                profiles.panel_open.set(true);
                            }
                            _ => {
                                profiles.panel_open.set(false);
                                profiles.filter_panel_open.set(true);
                                profiles.name.set(String::new());
                            }
                        }
                    })
                    .child(text("SYNC_STEP")),
            )
            .child(text(move || {
                format!(
                    "OPEN:f={},r={},p={}",
                    profiles.filter_panel_open.get(),
                    profiles.panel_open.get(),
                    profiles.preset_panel_open.get()
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SyncHarness>(Size::new(px(1200.0), px(760.0)));

        // Step 0: Active preset "test" is clean; sync footer stays hidden.
        assert!(!tree.has_text(t(lang, "更新预设", "Update preset")));
        assert!(!tree.has_text(t(lang, "存为预设…", "Save as preset…")));

        // Step 1: Modify filter -> Filter popover surfaces unsaved changed field, Discard, and Update button.
        assert!(tree.press_text(&mut core, "SYNC_STEP"));
        assert!(tree.has_text(t(
            lang,
            "预设「test」未保存：过滤",
            "Preset \"test\" unsaved: Filter",
        )));
        assert!(tree.has_text(t(lang, "丢弃", "Discard")));
        assert!(tree.has_text(t(lang, "更新预设", "Update preset")));

        // Step 2: Open Rules popover -> same quick-update bar & Discard button appear inside Rules popover.
        assert!(tree.press_text(&mut core, "SYNC_STEP"));
        assert!(tree.has_text(t(lang, "直通与上游", "Bypass & Proxy")));
        assert!(tree.has_text(t(lang, "丢弃", "Discard")));
        assert!(tree.has_text(t(lang, "更新预设", "Update preset")));

        // Step 3: Clear preset name with active filter -> shows "Save as preset…" without Discard; clicking opens Presets.
        assert!(tree.press_text(&mut core, "SYNC_STEP"));
        assert!(tree.has_text(t(lang, "存为预设…", "Save as preset…")));
        assert!(!tree.has_text(t(lang, "丢弃", "Discard")));
        assert!(tree.press_text(&mut core, t(lang, "存为预设…", "Save as preset…")));
        assert!(tree.has_text("OPEN:f=false,r=false,p=true"));
    }
}

#[test]
fn title_bar_sidebar_toggle_only_renders_in_split_mode_zh_en() {
    #[component]
    fn SidebarToggleHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("sidebar-toggle");
        let profiles = make_profiles();
        let step = signal(0_u8);

        view()
            .w(px(320.0))
            .h(px(80.0))
            .flex_row()
            .items_center()
            .child(toolbar_sidebar_toggle(
                profiles.layout_mode,
                profiles.sidebar_collapsed,
                controls,
            ))
            .child(
                view()
                    .on_press(move || {
                        let next = step.get() + 1;
                        step.set(next);
                        match next {
                            1 => profiles.sidebar_collapsed.set(true),
                            2 => profiles.layout_mode.set(config::LayoutMode::TopBottom),
                            _ => {
                                profiles.layout_mode.set(config::LayoutMode::Split);
                                profiles.sidebar_collapsed.set(false);
                            }
                        }
                    })
                    .child(text("SIDEBAR_STEP")),
            )
            .child(text(move || {
                format!(
                    "MODE:{:?}:{}",
                    profiles.layout_mode.get(),
                    profiles.sidebar_collapsed.get()
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SidebarToggleHarness>(Size::new(px(320.0), px(80.0)));
        let canvas_count = |t: &HeadlessTree| {
            t.kinds
                .values()
                .filter(|k| matches!(k, ViewKind::Canvas))
                .count()
        };

        assert!(tree.has_text("MODE:Split:false"));
        assert_eq!(canvas_count(&tree), 1);

        assert!(tree.press_text(&mut core, "SIDEBAR_STEP"));
        assert!(tree.has_text("MODE:Split:true"));
        assert_eq!(canvas_count(&tree), 1);

        assert!(tree.press_text(&mut core, "SIDEBAR_STEP"));
        assert!(tree.has_text("MODE:TopBottom:true"));
        assert_eq!(canvas_count(&tree), 0);

        assert!(tree.press_text(&mut core, "SIDEBAR_STEP"));
        assert!(tree.has_text("MODE:Split:false"));
        assert_eq!(canvas_count(&tree), 1);
    }
}

#[test]
fn persisted_folded_cards_cover_content_cards_and_skip_diff() {
    let restored = folded_bits_from_config(&[
        " QUERY ".to_string(),
        "Req_Headers".to_string(),
        "res_headers".to_string(),
        "REQ_BODY".to_string(),
        "res_body".to_string(),
        "messages".to_string(),
        "diff".to_string(),
        "unknown".to_string(),
    ]);
    assert_eq!(
        restored,
        FOLD_REQ_HEADERS
            | FOLD_RES_HEADERS
            | FOLD_QUERY
            | FOLD_REQ_BODY
            | FOLD_RES_BODY
            | FOLD_MESSAGES
    );

    let canonical = folded_cards_from_bits(
        FOLD_REQ_HEADERS
            | FOLD_RES_HEADERS
            | FOLD_QUERY
            | FOLD_REQ_BODY
            | FOLD_RES_BODY
            | FOLD_MESSAGES
            | FOLD_DIFF,
    );
    assert_eq!(
        canonical,
        vec![
            "req_headers",
            "res_headers",
            "query",
            "req_body",
            "res_body",
            "messages",
        ]
    );

    let (next, persisted) = toggle_card_bits(FOLD_DIFF, FOLD_RES_BODY);
    assert_eq!(next, FOLD_DIFF | FOLD_RES_BODY);
    assert_eq!(persisted, Some(vec!["res_body".to_string()]));

    let (next, persisted) = toggle_card_bits(next, FOLD_MESSAGES);
    assert_eq!(next, FOLD_DIFF | FOLD_RES_BODY | FOLD_MESSAGES);
    assert_eq!(
        persisted,
        Some(vec!["res_body".to_string(), "messages".to_string()])
    );

    let (next, persisted) = toggle_card_bits(next, FOLD_REQ_HEADERS);
    assert_eq!(
        next,
        FOLD_DIFF | FOLD_RES_BODY | FOLD_MESSAGES | FOLD_REQ_HEADERS
    );
    assert_eq!(
        persisted,
        Some(vec![
            "req_headers".to_string(),
            "res_body".to_string(),
            "messages".to_string(),
        ])
    );

    let (next, persisted) = toggle_card_bits(next, FOLD_RES_BODY);
    assert_eq!(next, FOLD_DIFF | FOLD_MESSAGES | FOLD_REQ_HEADERS);
    assert_eq!(
        persisted,
        Some(vec!["req_headers".to_string(), "messages".to_string()])
    );

    let (toggled_diff, persisted) = toggle_card_bits(next, FOLD_DIFF);
    assert_eq!(toggled_diff, next ^ FOLD_DIFF);
    assert_eq!(persisted, None);

    assert_eq!(effective_body_bytes(256, 256, false), 256);
    assert_eq!(effective_body_bytes(12_698, 2048, false), 12_698);
    assert_eq!(effective_body_bytes(80, 100, false), 80);
    assert_eq!(effective_body_bytes(1_048_576, 5_242_880, true), 5_242_880);
    assert_eq!(effective_body_bytes(0, 3_145_728, false), 3_145_728);
}

#[test]
fn header_and_message_rows_omit_per_row_copy_buttons_and_keep_copy_all() {
    #[component]
    fn HoverRowHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("hover-row-copy");
        let headers = signal(vec![
            HeaderLine {
                key: 0,
                name: "content-type".into(),
                value: "application/json".into(),
                hint: None,
            },
            HeaderLine {
                key: 1,
                name: "x-trace-id".into(),
                value: "trace-xyz".into(),
                hint: None,
            },
            HeaderLine {
                key: 2,
                name: "authorization".into(),
                value: "Bearer secret".into(),
                hint: None,
            },
        ]);
        let shown = make_shown("http://127.0.0.1:8877/ws", Some(101), "");
        shown.messages.set(vec![
            HeaderLine {
                key: 0,
                name: "↑ text".into(),
                value: "msg-alpha".into(),
                hint: None,
            },
            HeaderLine {
                key: 1,
                name: "↓ text".into(),
                value: "msg-beta".into(),
                hint: None,
            },
        ]);
        view()
            .w(px(520.0))
            .h(px(640.0))
            .gap_3()
            .child(header_section(HeaderSectionProps {
                zh: "响应头",
                en: "Response headers",
                lines: headers,
                loading: signal(false),
                pane_width: shown.pane_width,
                folded: signal(0_u8),
                bit: FOLD_RES_HEADERS,
                controls: controls.clone(),
            }))
            .child(messages_section(shown, controls, signal(0_u8)))
    }

    let live_canvas_count = |tree: &HeadlessTree| -> usize {
        tree.live_node_ids()
            .into_iter()
            .filter(|id| tree.kinds.get(id) == Some(&ViewKind::Canvas))
            .count()
    };

    let enter_ancestors =
        |tree: &mut HeadlessTree, core: &mut Core<ApproxTextEngine>, exact: &str| {
            let text_id = tree
                .live_node_ids()
                .into_iter()
                .find(|id| tree.texts.get(id).is_some_and(|s| s == exact))
                .expect("target row text");
            let mut cur = tree.parent.get(&text_id).copied().expect("row text parent");
            for _ in 0..3 {
                core.component_event(
                    cur,
                    EventKind::MouseEnter,
                    &EventPayload::Mouse(MouseEvent::default()),
                );
                tree.pump(core);
                cur = tree.parent.get(&cur).copied().expect("ancestor view");
            }
        };

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<HoverRowHarness>(Size::new(px(520.0), px(640.0)));
        assert!(tree.has_text("x-trace-id"));
        assert!(tree.has_text("msg-alpha"));
        assert!(tree.has_text(t(lang, "复制全部", "Copy all")));
        assert_eq!(
            live_canvas_count(&tree),
            2,
            "at rest, only the 2 card chevrons should render canvases in {lang:?}"
        );

        enter_ancestors(&mut tree, &mut core, "x-trace-id");
        assert_eq!(
            live_canvas_count(&tree),
            2,
            "hovering a header row must not add a per-row copy icon in {lang:?}"
        );

        enter_ancestors(&mut tree, &mut core, "msg-alpha");
        assert_eq!(
            live_canvas_count(&tree),
            2,
            "hovering a message row must not add a per-row copy icon in {lang:?}"
        );
    }
}

#[test]
fn facet_tab_has_filter_tracks_domain_mute_and_app_tokens() {
    assert!(!facet_tab_has_filter("", 0));
    assert!(!facet_tab_has_filter("", 1));

    assert!(facet_tab_has_filter("host:api.example.com", 0));
    assert!(!facet_tab_has_filter("host:api.example.com", 1));

    assert!(facet_tab_has_filter("mute:ads.example.com", 0));
    assert!(!facet_tab_has_filter("mute:ads.example.com", 1));

    assert!(!facet_tab_has_filter("app:curl", 0));
    assert!(facet_tab_has_filter("app:curl", 1));

    assert!(facet_tab_has_filter("host:api.example.com app:curl", 0));
    assert!(facet_tab_has_filter("host:api.example.com app:curl", 1));

    assert!(!facet_tab_has_filter("type:json slow:", 0));
    assert!(!facet_tab_has_filter("type:json slow:", 1));
}

#[test]
fn facet_drawer_overlay_dismisses_on_press_outside_and_toggles_tabs() {
    use crate::session::{FacetEntry, FacetKind, SessionFacets};

    #[component]
    fn FacetOverlayHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let facets = signal(SessionFacets {
            domains: vec![FacetEntry {
                label: "api.vicanso.dev".into(),
                token: "api.vicanso.dev".into(),
                kind: FacetKind::Domain,
                count: 1,
                error_count: 0,
            }],
            apps: vec![FacetEntry {
                label: "curl".into(),
                token: "curl".into(),
                kind: FacetKind::App,
                count: 1,
                error_count: 0,
            }],
        });
        let facets_open = signal(false);
        let facet_tab = signal(0_u8);
        let filter = signal(String::new());
        view()
            .w(px(520.0))
            .h(px(640.0))
            .child(facet_quick_bar(facets, facets_open, facet_tab, filter))
            .child(facet_drawer_overlay(
                facets,
                facets_open,
                facet_tab,
                filter,
                move || facets_open.get(),
                || 360.0,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<FacetOverlayHarness>(Size::new(px(520.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        let domains_btn = t(lang, "域名 (1)", "Domains (1)");
        let sheet_apps_tab = t(lang, "应用", "Apps");

        assert!(!tree.has_text("api.vicanso.dev"));
        assert!(tree.press_text(&mut core, domains_btn));
        assert!(tree.has_text("api.vicanso.dev"));

        // Pressing the sheet backdrop closes the left drawer.
        let modal_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.kinds.get(id) == Some(&ViewKind::Modal))
            .expect("sheet modal");
        let backdrop_id = tree
            .children
            .get(&modal_id)
            .and_then(|kids| kids.first().copied())
            .expect("sheet backdrop");
        core.component_event(
            backdrop_id,
            EventKind::Press,
            &EventPayload::Press(PressEvent::default()),
        );
        tree.pump(&mut core);
        assert!(
            !tree.has_text("api.vicanso.dev"),
            "backdrop press must dismiss facet sheet in {lang:?}"
        );

        // Re-open Domains, switch to Apps tab inside the sheet, then dismiss with Escape.
        assert!(tree.press_text(&mut core, domains_btn));
        assert!(tree.has_text("api.vicanso.dev"));

        assert!(tree.press_text(&mut core, sheet_apps_tab));
        assert!(!tree.has_text("api.vicanso.dev"));
        assert!(tree.has_text("curl"));

        let modal_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.kinds.get(id) == Some(&ViewKind::Modal))
            .expect("sheet modal after tab switch");
        core.component_event(modal_id, EventKind::RequestClose, &EventPayload::None);
        tree.pump(&mut core);
        assert!(
            !tree.has_text("curl"),
            "Escape/RequestClose must dismiss facet sheet in {lang:?}"
        );
    }
}

#[test]
fn resize_handles_update_and_clamp_pane_dimensions() {
    use nsh_core::{PointerType, TouchEvent, TouchPhase, TouchPoint};

    #[component]
    fn ResizeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("resize-handles");
        let list_width = signal(380.0_f32);
        let list_height = signal(320.0_f32);
        let sidebar_width = signal(SIDEBAR_WIDTH_DEFAULT);
        let layout_mode = signal(config::LayoutMode::Split);
        let sidebar_collapsed = signal(false);

        view()
            .w(px(1200.0))
            .h(px(800.0))
            .flex_col()
            .child(row_splitter(list_height, controls.clone()))
            .child(column_splitter(
                list_width,
                layout_mode,
                sidebar_width,
                sidebar_collapsed,
                controls,
            ))
            .child(text(move || {
                format!(
                    "DIMS|{}|{}",
                    list_width.get().round() as i32,
                    list_height.get().round() as i32
                )
            }))
    }

    fn touch_drag(phase: TouchPhase, path: Vec<NodeId>, x: f32, y: f32, ts: f64) -> EventPayload {
        EventPayload::Touch(Box::new(TouchEvent {
            phase,
            timestamp: ts,
            points: vec![TouchPoint {
                id: 1,
                target: path[0],
                path: if phase == TouchPhase::Start {
                    path
                } else {
                    Vec::new()
                },
                page: Point::new(px(x), px(y)),
                location: Point::new(px(x), px(y)),
                changed: true,
                pointer_type: PointerType::Mouse,
                pressure: 0.5,
                tilt_x: 0.0,
                tilt_y: 0.0,
            }],
            activated: None,
            modifiers: Default::default(),
        }))
    }

    TEST_LANG.set(Lang::En);
    let (mut core, mut tree) = mount::<ResizeHarness>(Size::new(px(1200.0), px(800.0)));
    assert!(tree.has_text("DIMS|380|320"));

    // Locate handles by accessible label (z-index stacking places splitters after sibling text).
    let harness_root = tree.children[&ROOT_ID][0];
    let col_handle = tree
        .live_node_ids()
        .into_iter()
        .find(|id| {
            tree.labels.get(id).and_then(|l| l.as_deref()) == Some("Resize list and detail width")
        })
        .expect("column resize handle");
    let col_wrapper = tree.parent[&col_handle];
    let col_path = vec![col_handle, col_wrapper, harness_root, ROOT_ID];
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::Start, col_path, 380.0, 200.0, 0.0),
    );
    tree.pump(&mut core);
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::Move, vec![col_handle], 428.0, 200.0, 16.0),
    );
    tree.pump(&mut core);
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::End, vec![col_handle], 428.0, 200.0, 32.0),
    );
    tree.pump(&mut core);
    assert!(
        tree.has_text("DIMS|428|320"),
        "expected column resize to apply offset; texts={:?}",
        tree.all_live_texts()
    );

    let row_handle = tree
        .live_node_ids()
        .into_iter()
        .find(|id| {
            tree.labels.get(id).and_then(|l| l.as_deref()) == Some("Resize list and detail height")
        })
        .expect("row resize handle");
    let row_wrapper = tree.parent[&row_handle];
    let row_path = vec![row_handle, row_wrapper, harness_root, ROOT_ID];
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::Start, row_path, 200.0, 320.0, 48.0),
    );
    tree.pump(&mut core);
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::Move, vec![row_handle], 200.0, 280.0, 64.0),
    );
    tree.pump(&mut core);
    core.component_event(
        ROOT_ID,
        EventKind::Touch,
        &touch_drag(TouchPhase::End, vec![row_handle], 200.0, 280.0, 80.0),
    );
    tree.pump(&mut core);
    assert!(
        tree.has_text("DIMS|428|280"),
        "expected row resize to apply offset; texts={:?}",
        tree.all_live_texts()
    );
}

#[test]
fn rules_panel_redesign_tabs_grouped_kind_menu_and_collapsible_quick_add() {
    use std::collections::BTreeSet;

    assert_eq!(QUICK_KIND_GROUPS.len(), 4);
    let seen: BTreeSet<u8> = QUICK_KIND_GROUPS
        .iter()
        .flat_map(|g| g.items.iter().map(|it| it.idx))
        .collect();
    assert_eq!(seen.len(), 16);
    assert_eq!(*seen.first().unwrap(), 0);
    assert_eq!(*seen.last().unwrap(), 15);

    #[component]
    fn RulesPanelHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("rules-panel-redesign");
        let profiles = make_profiles();
        profiles.rules.set("cors *\ndelay * 2000ms".into());
        let draft = make_rule_draft();
        view()
            .w(px(720.0))
            .h(px(800.0))
            .child(rules_panel(profiles, controls, draft))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<RulesPanelHarness>(Size::new(px(720.0), px(800.0)));

        // Default Tab 0: underline tabs + mini toggles (`Enabled` / `Breaks`),
        // single unified `+ Add rule` menu, no idle bottom form bar, and collapsed line chips.
        assert!(tree.has_text(t(lang, "规则", "Rules")));
        assert!(tree.has_text(t(lang, "直通与上游", "Bypass & Proxy")));
        assert!(tree.has_text(t(lang, "启用", "Enabled")));
        assert!(tree.has_text(t(lang, "断点", "Breaks")));
        assert!(tree.has_text(t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.has_text(t(lang, "管理单条", "Manage")));
        assert!(!tree.has_text(t(lang, "+ 表单添加规则", "+ Add rule via form")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(!tree.has_text("#1 cors *"));
        assert!(!tree.has_text(t(lang, "直通名单（不解密）", "TLS Bypass (No MITM)")));
        assert!(!tree.has_text(t(lang, "上游代理", "Upstream Proxy")));
        assert!(!tree.has_text(t(lang, "开发期 CORS", "Dev CORS")));

        // Open `+ Add rule` -> click 1-click preset `Throttle (3G)` -> inserts directly without opening form.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.has_text(t(lang, "开发期 CORS", "Dev CORS")));
        assert!(tree.has_text(t(lang, "弱网限速（3G）", "Throttle (3G)")));
        assert!(tree.press_text(&mut core, t(lang, "弱网限速（3G）", "Throttle (3G)")));
        assert!(tree.has_text_containing(t(lang, "3 条规则", "3 rules")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));

        // Expand per-line manage chips on demand.
        assert!(tree.press_text(&mut core, t(lang, "管理单条", "Manage")));
        assert!(tree.has_text("#1 cors *"));
        assert!(tree.has_text("#3 throttle * 3g"));
        assert!(tree.press_text(&mut core, t(lang, "收起", "Hide")));
        assert!(!tree.has_text("#1 cors *"));

        // Open `+ Add rule` -> choose parameterized common example `Force 500` -> pre-fills form.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.has_text(t(lang, "常用示例", "Common Examples")));
        assert!(tree.press_text(&mut core, t(lang, "强制 500", "Force 500")));
        assert!(tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(tree.has_text(t(lang, "填入示例", "Use example")));
        assert!(tree.has_text("map-local api.example.com/v1 status=500"));
        assert!(tree.press_text(&mut core, t(lang, "确认添加", "Add")));
        assert!(tree.has_text_containing(t(lang, "4 条规则", "4 rules")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));

        // Open `+ Add rule` -> choose typed builder `Replace` -> opens form with contextual example.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        let replace_label = t(lang, "替换", "Replace");
        let resolve_label = t(lang, "解析", "Resolve");
        assert!(tree.has_text(replace_label));
        assert!(tree.press_text(&mut core, replace_label));
        assert!(tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(tree.has_text(r#"replace api.example.com/v1 res "old" -> "new""#));
        assert!(tree.press_text(&mut core, t(lang, "填入示例", "Use example")));
        assert!(tree.press_text(&mut core, t(lang, "确认添加", "Add")));
        assert!(tree.has_text_containing(t(lang, "5 条规则", "5 rules")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));

        // Open `+ Add rule` -> `Replace` -> test `GQL`, switch kind dropdown to `Resolve`, and close.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.press_text(&mut core, replace_label));
        assert!(tree.has_text("GQL"));
        assert!(tree.press_text(&mut core, "GQL"));

        // Switch kind via form dropdown and confirm the contextual example updates, then close.
        assert!(tree.press_text(&mut core, replace_label));
        assert!(
            tree.has_text(resolve_label),
            "grouped rule kind dropdown should reveal '{resolve_label}' in {lang:?}"
        );
        assert!(tree.press_text(&mut core, resolve_label));
        assert!(tree.has_text(resolve_label));
        assert!(tree.has_text("resolve api.example.com 127.0.0.1 8080"));
        assert!(tree.press_text(&mut core, t(lang, "关闭", "Close")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));

        // Switch to Tab 1 ("直通与上游" / "Bypass & Proxy").
        assert!(tree.press_text(&mut core, t(lang, "直通与上游", "Bypass & Proxy")));
        assert!(tree.has_text(t(lang, "直通名单（不解密）", "TLS Bypass (No MITM)")));
        assert!(tree.has_text(t(lang, "上游代理", "Upstream Proxy")));
        assert!(tree.has_text(t(lang, "显示隧道失败", "Show tunnel failures")));
    }
}

#[test]
fn bug1_break_conditions_preserved_on_add_dry_run_and_save() {
    assert_eq!(
        effective_quick_inputs(4, false, "X-Debug:1", "error"),
        ("X-Debug:1", "error")
    );
    assert_eq!(
        effective_quick_inputs(5, false, "X-Trace:1", "timeout"),
        ("X-Trace:1", "timeout")
    );
    assert_eq!(
        draft_rules_with_op(
            "",
            "api.example.com",
            "/v1",
            "GetUser",
            "X-Debug:1",
            "error",
            false,
            4
        ),
        "break api.example.com/v1 header=X-Debug:1 body=error op=GetUser\n"
    );
    assert_eq!(
        draft_rules_with_op("", "api.example.com", "/v1", "", "", "error", false, 5),
        "break-res api.example.com/v1 body=error\n"
    );

    #[component]
    fn BreakAddHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("bug1-break-conds");
        let profiles = make_profiles();
        let draft = make_rule_draft();
        view()
            .w(px(720.0))
            .h(px(800.0))
            .flex_col()
            .child(rules_panel(profiles, controls, draft))
            .child(text(move || format!("RULES_NOW:{}", profiles.rules.get())))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<BreakAddHarness>(Size::new(px(720.0), px(800.0)));

        // Kind 4 (`break`) example carries `header=X-Debug:1`.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.press_text(&mut core, "break"));
        assert!(tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(tree.press_text(&mut core, t(lang, "填入示例", "Use example")));
        assert!(tree.press_text(&mut core, t(lang, "确认添加", "Add")));
        assert!(
            tree.has_text_containing("break api.example.com/v1 header=X-Debug:1"),
            "break header condition must survive Add in {lang:?}: {:?}",
            tree.all_live_texts()
        );

        // Kind 5 (`break-res`) example carries `body=error`.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.press_text(&mut core, "break-res"));
        assert!(tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(tree.press_text(&mut core, t(lang, "填入示例", "Use example")));
        assert!(tree.press_text(&mut core, t(lang, "确认添加", "Add")));
        assert!(
            tree.has_text_containing("break-res api.example.com/v1 body=error"),
            "break-res body condition must survive Add in {lang:?}: {:?}",
            tree.all_live_texts()
        );
    }
}

#[test]
fn bug2_closing_quick_add_clears_draft_and_preset_dirty_state() {
    #[component]
    fn CloseDraftHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("bug2-close-draft");
        let profiles = make_profiles();
        let saved = config::SavedProfile {
            name: "dev".into(),
            filter: String::new(),
            bypass_hosts: String::new(),
            rules: "cors *".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        };
        profiles.saved.set(vec![saved]);
        profiles.name.set("dev".into());
        profiles.rules.set("cors *".into());
        let draft = make_rule_draft();
        view()
            .w(px(720.0))
            .h(px(800.0))
            .flex_col()
            .child(rules_panel(profiles, controls, draft))
            .child(text(move || {
                format!("STATE:{:?}", live_preset_save_state(profiles, draft))
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<CloseDraftHarness>(Size::new(px(720.0), px(800.0)));
        assert!(tree.has_text("STATE:Clean"));

        // Pre-fill parameterized example `Force 500` -> marks preset dirty.
        assert!(tree.press_text(&mut core, t(lang, "+ 添加规则", "+ Add rule")));
        assert!(tree.press_text(&mut core, t(lang, "强制 500", "Force 500")));
        assert!(tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(
            tree.has_text("STATE:UpdateExisting(\"dev\")"),
            "expected dirty preset state while draft is open in {lang:?}"
        );

        // Clicking Close clears draft and restores Clean preset state.
        assert!(tree.press_text(&mut core, t(lang, "关闭", "Close")));
        assert!(!tree.has_text(t(lang, "按类型配置规则", "Configure Rule")));
        assert!(
            tree.has_text("STATE:Clean"),
            "closing quick-add form must clear draft and restore Clean state in {lang:?}"
        );
    }
}

#[test]
fn bug3_external_jumps_switch_rules_panel_active_tab() {
    #[component]
    fn TabJumpHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("bug3-tab-jump");
        let profiles = make_profiles();
        profiles
            .rules
            .set("map-local api.example.com/v1 status=201".into());
        let draft = make_rule_draft();
        let shown = make_shown(
            "https://api.example.com/v1/orders",
            None,
            "connection refused",
        );
        shown.rule_hits.set(vec![session::RuleHit {
            verb: "map-local".into(),
            group_name: None,
            line_no: 1,
        }]);
        view()
            .w(px(760.0))
            .h(px(860.0))
            .flex_col()
            .gap_2()
            .child(detail_rule_badges(shown, profiles, controls.clone()))
            .child(error_summary_banner(shown, controls.clone(), profiles))
            .child(rules_panel(profiles, controls, draft))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<TabJumpHarness>(Size::new(px(760.0), px(860.0)));

        // Start on Tab 0, switch manually to Tab 1.
        assert!(tree.press_text(&mut core, t(lang, "直通与上游", "Bypass & Proxy")));
        assert!(tree.has_text(t(lang, "上游代理", "Upstream Proxy")));

        // Clicking a detail rule badge jumps back to Tab 0 and highlights the rule chip.
        assert!(tree.press_text(&mut core, "map-local #1"));
        assert!(
            tree.has_text("#1 map-local api.example.com/v1 status=201"),
            "focus_rule_hit must switch to Tab 0 and reveal highlighted chip in {lang:?}"
        );
        assert!(!tree.has_text(t(lang, "上游代理", "Upstream Proxy")));

        // Clicking `Check Upstream` jumps to Tab 1 where Upstream Proxy lives.
        assert!(tree.press_text(&mut core, t(lang, "检查上游", "Check Upstream")));
        assert!(
            tree.has_text(t(lang, "上游代理", "Upstream Proxy")),
            "ErrorAction::CheckUpstream must open Tab 1 in {lang:?}"
        );

        // Clicking `Add resolve` jumps back to Tab 0 and opens the pre-filled form.
        assert!(tree.press_text(&mut core, t(lang, "添加 resolve", "Add resolve")));
        assert!(
            tree.has_text(t(lang, "按类型配置规则", "Configure Rule")),
            "ErrorAction::AddResolve must switch back to Tab 0 and open quick-add in {lang:?}"
        );
        assert!(!tree.has_text(t(lang, "上游代理", "Upstream Proxy")));
    }
}

#[test]
fn bug4_proto_and_invalid_chips_do_not_open_stale_form() {
    #[component]
    fn ChipGuardHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("bug4-chip-guard");
        let profiles = make_profiles();
        profiles
            .rules
            .set("cors *\nproto api.example.com/grpc /tmp/schema.proto\nbad syntax line".into());
        let draft = make_rule_draft();
        view()
            .w(px(720.0))
            .h(px(800.0))
            .child(rules_panel(profiles, controls, draft))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<ChipGuardHarness>(Size::new(px(720.0), px(800.0)));

        // Expand line chips.
        assert!(tree.press_text(&mut core, t(lang, "管理单条", "Manage")));
        assert!(tree.has_text("#2 proto api.example.com/grpc /tmp/schema.proto"));
        assert!(tree.has_text("#3 bad syntax line"));

        // Clicking proto chip must NOT open the quick-add form.
        assert!(tree.press_text(&mut core, "#2 proto api.example.com/grpc /tmp/schema.proto"));
        assert!(
            !tree.has_text(t(lang, "按类型配置规则", "Configure Rule")),
            "proto chip must not open quick-add form in {lang:?}"
        );

        // Clicking invalid chip must NOT open the quick-add form.
        assert!(tree.press_text(&mut core, "#3 bad syntax line"));
        assert!(
            !tree.has_text(t(lang, "按类型配置规则", "Configure Rule")),
            "invalid chip must not open quick-add form in {lang:?}"
        );

        // Clicking a supported rule chip (`cors *`) still opens the form.
        assert!(tree.press_text(&mut core, "#1 cors *"));
        assert!(
            tree.has_text(t(lang, "按类型配置规则", "Configure Rule")),
            "supported chip should open quick-add form in {lang:?}"
        );
    }
}

#[test]
fn popover_closes_when_focus_moves_into_outside_workspace_input() {
    #[component]
    fn OutsideInputDismissHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let dismiss = provide_popover_dismiss();
        let workspace_ref = element_ref();
        workspace_ref.on_focus_in(move || dismiss.dismiss());
        let open = signal(true);
        bind_popover_dismiss(open);
        let inside_text = signal(String::new());
        let outside_text = signal(String::new());

        view()
            .w(px(640.0))
            .h(px(480.0))
            .flex_col()
            .child(
                popover()
                    .open(move || Some(open.get()))
                    .on_open_change(move |event: &nsh_base::PopoverEvent| {
                        if let nsh_base::PopoverEvent::OpenChange(next, _) = event {
                            open.set(*next);
                        }
                    })
                    .trigger(view().child(text("PRESET_TRIGGER")))
                    .child(
                        view().p_2().child(text("INSIDE_HOST")).child(
                            input()
                                .value(inside_text)
                                .on_change(move |s: &str| inside_text.set(s.to_string())),
                        ),
                    ),
            )
            .child(
                view()
                    .element_ref(&workspace_ref)
                    .flex_1()
                    .child(text("OUTSIDE_HOST"))
                    .child(
                        input()
                            .value(outside_text)
                            .on_change(move |s: &str| outside_text.set(s.to_string())),
                    ),
            )
            .child(text(move || format!("POPOVER_OPEN:{}", open.get())))
    }

    TEST_LANG.set(Lang::En);
    let (mut core, mut tree) = mount::<OutsideInputDismissHarness>(Size::new(px(640.0), px(480.0)));
    assert!(tree.has_text("POPOVER_OPEN:true"));

    let find_sibling_input = |marker: &str| -> NodeId {
        let marker_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.texts.get(id).is_some_and(|s| s == marker))
            .unwrap_or_else(|| panic!("missing marker {marker}"));
        let parent_id = tree.parent[&marker_id];
        tree.children[&parent_id]
            .iter()
            .copied()
            .find(|id| tree.kinds.get(id) == Some(&ViewKind::Input))
            .unwrap_or_else(|| panic!("missing Input under {marker}"))
    };
    let inside_input = find_sibling_input("INSIDE_HOST");
    let outside_input = find_sibling_input("OUTSIDE_HOST");

    // Focusing an input inside the popover keeps the popover open.
    core.set_states(inside_input, nsh_core::tailwind::state::FOCUSED);
    tree.pump(&mut core);
    assert!(tree.has_text("POPOVER_OPEN:true"));

    // Moving focus into the outside workspace input closes the popover.
    core.set_states(outside_input, nsh_core::tailwind::state::FOCUSED);
    tree.pump(&mut core);
    assert!(tree.has_text("POPOVER_OPEN:false"));
}

#[test]
fn switching_quick_rule_kind_clears_incompatible_action_inputs_and_preserves_target() {
    assert!(quick_kind_shares_action_inputs(6, 6));
    assert!(quick_kind_shares_action_inputs(0, 0));
    assert!(quick_kind_shares_action_inputs(4, 5));
    assert!(quick_kind_shares_action_inputs(5, 4));
    assert!(!quick_kind_shares_action_inputs(6, 4));
    assert!(!quick_kind_shares_action_inputs(0, 3));

    #[component]
    fn QuickKindSwitchHarness() -> impl IntoElement {
        provide_base_contexts(Lang::En);
        let controls = make_controls("quick-kind-switch");
        let profiles = make_profiles();
        let draft = make_rule_draft();
        draft.kind.set(6);
        draft.host.set("api.vicanso.dev".into());
        draft.path.set("/v1".into());
        draft.op.set("GetUser".into());
        draft.from.set("X-Signature123".into());
        draft
            .to
            .set(r#"{{hmac_sha256("dev-secret", req.body)}}"#.into());
        let open = signal(true);

        view()
            .w(px(720.0))
            .h(px(640.0))
            .flex_col()
            .child(quick_add_section(
                RuleEditTarget::from_profiles(profiles),
                controls,
                draft,
                open,
            ))
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || switch_quick_kind(draft, 6))
                    .child(text("btn_same")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || switch_quick_kind(draft, 4))
                    .child(text("btn_to_break")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || {
                        draft.from.set("X-Debug:1".into());
                        draft.to.set("error".into());
                        switch_quick_kind(draft, 5);
                    })
                    .child(text("btn_to_break_res")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || switch_quick_kind(draft, 3))
                    .child(text("btn_to_delay")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || draft.from.set("2000ms".into()))
                    .child(text("btn_seed_delay")),
            )
            .child(text(move || {
                format!(
                    "DRAFT|{}|{}|{}|{}|{}|{}",
                    draft.kind.get(),
                    draft.host.get(),
                    draft.path.get(),
                    draft.op.get(),
                    draft.from.get(),
                    draft.to.get(),
                )
            }))
    }

    let input_with_placeholder = |tree: &HeadlessTree, placeholder: &str| -> Option<NodeId> {
        tree.live_node_ids().into_iter().find(|id| {
            tree.kinds.get(id) == Some(&ViewKind::Input)
                && tree.placeholders.get(id).is_some_and(|p| p == placeholder)
        })
    };

    assert_eq!(quick_action_input_group(4), quick_action_input_group(5));
    assert_ne!(quick_action_input_group(6), quick_action_input_group(4));

    let (mut core, mut tree) = mount::<QuickKindSwitchHarness>(Size::new(px(720.0), px(640.0)));
    assert!(tree.has_text(
        r#"DRAFT|6|api.vicanso.dev|/v1|GetUser|X-Signature123|{{hmac_sha256("dev-secret", req.body)}}"#
    ));
    let header_from_id =
        input_with_placeholder(&tree, "Header name: X-Debug").expect("Header from input mounted");
    let header_to_id =
        input_with_placeholder(&tree, "Value (empty to delete)").expect("Header to input mounted");

    // Selecting the same kind preserves both action inputs and node identities.
    assert!(tree.press_text(&mut core, "btn_same"));
    assert!(tree.has_text(
        r#"DRAFT|6|api.vicanso.dev|/v1|GetUser|X-Signature123|{{hmac_sha256("dev-secret", req.body)}}"#
    ));
    assert_eq!(
        input_with_placeholder(&tree, "Value (empty to delete)"),
        Some(header_to_id)
    );

    // Switching Header (6) -> Break (4) preserves host/path/op, clears from/to, and
    // remounts action inputs with new NodeIds and updated placeholders.
    assert!(tree.press_text(&mut core, "btn_to_break"));
    assert!(tree.has_text("DRAFT|4|api.vicanso.dev|/v1|GetUser||"));
    let live_ids = tree.live_node_ids();
    assert!(
        !live_ids.contains(&header_from_id) && !live_ids.contains(&header_to_id),
        "Header from/to inputs must unmount on incompatible kind switch"
    );
    let break_from_id = input_with_placeholder(&tree, "Header cond (optional): X-Debug:1")
        .expect("Break from input remounted with updated placeholder");
    let break_to_id = input_with_placeholder(&tree, "Body substr (optional): error")
        .expect("Break to input remounted with updated placeholder");

    // Switching Break (4) -> Break-Res (5) preserves shared condition inputs in place.
    assert!(tree.press_text(&mut core, "btn_to_break_res"));
    assert!(tree.has_text("DRAFT|5|api.vicanso.dev|/v1|GetUser|X-Debug:1|error"));
    assert_eq!(
        input_with_placeholder(&tree, "Header cond (optional): X-Debug:1"),
        Some(break_from_id)
    );
    assert_eq!(
        input_with_placeholder(&tree, "Body substr (optional): error"),
        Some(break_to_id)
    );

    // Switching Break-Res (5) -> Delay (3) preserves target, clears from/to, and unmounts `to`.
    assert!(tree.press_text(&mut core, "btn_to_delay"));
    assert!(tree.has_text("DRAFT|3|api.vicanso.dev|/v1|GetUser||"));
    assert!(input_with_placeholder(&tree, "Body substr (optional): error").is_none());
    assert!(input_with_placeholder(&tree, "Delay duration: 2000ms or 2s").is_some());

    // Switching via the live quick_kind_dropdown menu clears Delay input too and refreshes `to`.
    assert!(tree.press_text(&mut core, "btn_seed_delay"));
    assert!(tree.has_text("DRAFT|3|api.vicanso.dev|/v1|GetUser|2000ms|"));
    assert!(tree.press_text(&mut core, "Delay"));
    assert!(tree.press_text(&mut core, "Break"));
    assert!(tree.has_text("DRAFT|4|api.vicanso.dev|/v1|GetUser||"));
    assert!(input_with_placeholder(&tree, "Body substr (optional): error").is_some());
}

#[tokio::test]
async fn preset_sync_bar_shows_diff_and_discards_with_two_step_confirm_zh_en() {
    #[component]
    fn SyncDiscardHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("preset-sync-discard");
        let profiles = make_profiles();
        let draft = make_rule_draft();
        profiles.saved.set(vec![config::SavedProfile {
            name: "test".into(),
            filter: "host:api.vicanso.dev".into(),
            bypass_hosts: String::new(),
            rules: "cors *".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }]);
        profiles.name.set("test".into());
        profiles.filter.set("host:api.vicanso.dev".into());
        profiles.rules.set("cors *".into());
        profiles.rules_on.set(true);
        profiles.breaks_on.set(true);

        view()
            .w(px(720.0))
            .h(px(360.0))
            .flex_col()
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || {
                        profiles.filter.set("host:api.vicanso.dev path:/v2".into());
                        profiles.breaks_on.set(false);
                        draft.kind.set(4);
                        draft.host.set("api.vicanso.dev".into());
                        draft.path.set("/v1".into());
                    })
                    .child(text("DIRTY_ALL")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || {
                        profiles.filter.set("host:cdn.vicanso.dev".into());
                    })
                    .child(text("DIRTY_CDN")),
            )
            .child(
                view()
                    .role(AccessibilityRole::Button)
                    .on_press(move || {
                        profiles.filter.set("host:api.vicanso.dev".into());
                    })
                    .child(text("DIRTY_API")),
            )
            .child(text(move || {
                format!(
                    "SNAP|{}|{}|{}|{}",
                    profiles.filter.get(),
                    profiles.rules.get(),
                    profiles.breaks_on.get(),
                    draft.host.get(),
                )
            }))
            .child(preset_sync_bar(profiles, draft, controls))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SyncDiscardHarness>(Size::new(px(720.0), px(360.0)));
        assert!(!tree.has_text(t(lang, "更新预设", "Update preset")));
        assert!(!tree.has_text(t(lang, "丢弃", "Discard")));

        // Dirty filter + toggle + quick-rule draft -> status lists every changed part.
        assert!(tree.press_text(&mut core, "DIRTY_ALL"));
        assert!(tree.has_text(t(
            lang,
            "预设「test」未保存：过滤、规则（含草稿）、断点开关",
            "Preset \"test\" unsaved: Filter, Rules (incl. draft), Breaks toggle",
        )));
        assert!(tree.has_text(t(lang, "丢弃", "Discard")));

        // First click arms confirm state without reverting workspace changes.
        assert!(tree.press_text(&mut core, t(lang, "丢弃", "Discard")));
        assert!(tree.has_text(t(lang, "确认丢弃", "Confirm")));
        assert!(tree.has_text("SNAP|host:api.vicanso.dev path:/v2|cors *|false|api.vicanso.dev"));

        // Second click discards all edits + draft and returns preset to Clean.
        assert!(tree.press_text(&mut core, t(lang, "确认丢弃", "Confirm")));
        assert!(tree.has_text("SNAP|host:api.vicanso.dev|cors *|true|"));
        assert!(!tree.has_text(t(lang, "更新预设", "Update preset")));
        assert!(!tree.has_text(t(lang, "丢弃", "Discard")));

        // Arming Discard then clicking Update preset clears pending confirm state cleanly.
        assert!(tree.press_text(&mut core, "DIRTY_CDN"));
        assert!(tree.press_text(&mut core, t(lang, "丢弃", "Discard")));
        assert!(tree.has_text(t(lang, "确认丢弃", "Confirm")));
        assert!(tree.press_text(&mut core, t(lang, "更新预设", "Update preset")));
        assert!(!tree.has_text(t(lang, "更新预设", "Update preset")));

        assert!(tree.press_text(&mut core, "DIRTY_API"));
        assert!(tree.has_text(t(lang, "丢弃", "Discard")));
        assert!(!tree.has_text(t(lang, "确认丢弃", "Confirm")));
    }
}

#[tokio::test]
async fn popover_trigger_tooltips_stay_suppressed_while_open_and_single_escape_closes_zh_en() {
    #[component]
    fn PopoverEscapeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        provide_popover_dismiss();
        let controls = make_controls("popover-esc");
        let profiles = make_profiles();
        let draft = make_rule_draft();

        view()
            .w(px(960.0))
            .h(px(640.0))
            .flex_row()
            .gap_2()
            .child(saved_filter_menu(profiles, controls.clone(), draft))
            .child(rules_menu_popover(profiles, controls.clone(), draft))
            .child(preset_pill_popover(profiles, controls, draft))
            .child(text(move || {
                format!(
                    "OPEN|{}|{}|{}",
                    profiles.filter_panel_open.get(),
                    profiles.panel_open.get(),
                    profiles.preset_panel_open.get(),
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<PopoverEscapeHarness>(Size::new(px(960.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        let cases = [
            (
                t(lang, "过滤", "Filter"),
                t(lang, "流量过滤", "Traffic Filter"),
                t(lang, "视图流量过滤", "Traffic view filter"),
            ),
            (
                t(lang, "规则", "Rules"),
                t(lang, "直通与上游", "Bypass & Proxy"),
                t(
                    lang,
                    "规则、分组、直通与上游代理",
                    "Rules, groups, TLS bypass & upstream",
                ),
            ),
            (
                t(lang, "预设", "Presets"),
                t(lang, "新建预设", "New preset"),
                t(
                    lang,
                    "切换或保存配置预设（包含过滤与规则）",
                    "Switch or save presets (filter + rules)",
                ),
            ),
        ];

        for (trigger_label, open_marker, tip_substr) in cases {
            let trigger_text_id = tree
                .live_node_ids()
                .into_iter()
                .find(|id| tree.texts.get(id).is_some_and(|s| s == trigger_label))
                .unwrap_or_else(|| panic!("missing trigger {trigger_label} in {lang:?}"));
            let trigger_id = tree.parent[&trigger_text_id];

            // Keyboard focus while closed shows the trigger tooltip hint.
            core.set_input_modality(nsh_core::platform::InputModality::Keyboard);
            core.set_states(trigger_id, nsh_core::tailwind::state::FOCUSED);
            tree.pump(&mut core);
            assert!(
                tree.has_text_containing(tip_substr),
                "closed trigger {trigger_label} tooltip should render on keyboard focus in {lang:?}"
            );

            // Opening the popover immediately hides/disables the trigger tooltip.
            assert!(tree.press_text(&mut core, trigger_label));
            assert!(tree.has_text(open_marker));
            assert!(!tree.has_text("OPEN|false|false|false"));
            assert!(
                !tree.has_text_containing(tip_substr),
                "trigger {trigger_label} tooltip must stay suppressed while open in {lang:?}"
            );

            // Re-focusing the trigger via keyboard while open must NOT surface the tooltip.
            core.set_input_modality(nsh_core::platform::InputModality::Keyboard);
            core.set_states(trigger_id, nsh_core::tailwind::state::FOCUSED);
            tree.pump(&mut core);
            assert!(
                !tree.has_text_containing(tip_substr),
                "re-focused trigger {trigger_label} tooltip must remain disabled while open in {lang:?}"
            );

            // A single Escape keystroke flips the open signal and unmounts the popover immediately.
            core.component_event(
                ROOT_ID,
                EventKind::KeyCommand,
                &EventPayload::Value(nsh_core::PayloadValue::Number(u32::MAX as f64)),
            );
            tree.pump(&mut core);
            assert!(
                tree.has_text("OPEN|false|false|false"),
                "single Escape must close open signal for {trigger_label} in {lang:?}; texts={:?}",
                tree.all_live_texts()
            );
            assert!(
                !tree.has_text(open_marker),
                "single Escape must unmount open {trigger_label} popover in {lang:?}"
            );
        }
    }
}

#[test]
fn batch3_collapsible_headers_and_onboarding_disclosure_toggle_zh_en() {
    #[component]
    fn CollapsibleHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("b3-coll");
        let profiles = make_profiles();
        let folded = signal(0_u8);
        let headers = signal(vec![HeaderLine {
            key: 1,
            name: "x-trace-id".into(),
            value: "val-alpha".into(),
            hint: None,
        }]);
        let props = OnboardingProps {
            controls: controls.clone(),
            running: signal(true),
            listen: signal("127.0.0.1:8877".into()),
            ca_ui: make_ca_signals(false),
            use_system_proxy: signal(false),
            adb_count: signal(0),
            flow_total: signal(0),
            onboarding_traffic: signal(OnboardingTraffic::default()),
            compose: make_compose(),
            selected: signal(None),
            profiles,
            install_ca: Rc::new(|| {}),
            toggle_system_proxy: Rc::new(|| {}),
            qr_open: signal(false),
            pending_qr: signal(false),
        };
        let root = view()
            .w(px(640.0))
            .h(px(720.0))
            .flex_col()
            .gap_2()
            .child(header_section(HeaderSectionProps {
                zh: "响应头",
                en: "Response headers",
                lines: headers,
                loading: signal(false),
                pane_width: signal(520.0),
                folded,
                bit: FOLD_RES_HEADERS,
                controls,
            }))
            .child(text(move || format!("FOLDED:{}", folded.get())));
        #[cfg(not(target_arch = "wasm32"))]
        let root = root.child(onboarding_more_connection_disclosure(&props));
        root
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<CollapsibleHarness>(Size::new(px(640.0), px(720.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);

        assert!(tree.has_text("FOLDED:0"));
        assert!(tree.has_text("val-alpha"));
        assert!(!tree.has_text(t(lang, "复制代理环境", "Copy Proxy Env")));

        // Toggle response headers collapsible closed & re-open.
        assert!(tree.press_text(&mut core, t(lang, "响应头", "Response headers")));
        assert!(tree.has_text(&format!("FOLDED:{FOLD_RES_HEADERS}")));
        assert!(
            !tree.has_text("val-alpha"),
            "collapsing header_section must unmount header lines in {lang:?}"
        );
        assert!(tree.press_text(&mut core, t(lang, "响应头", "Response headers")));
        assert!(tree.has_text("FOLDED:0"));
        assert!(tree.has_text("val-alpha"));

        // Toggle onboarding disclosure open & closed.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let toggle = t(lang, "更多连接方式", "More connection options");
            let utility = t(lang, "复制代理环境", "Copy Proxy Env");
            assert!(tree.press_text(&mut core, toggle));
            assert!(
                tree.has_text(utility),
                "opening onboarding disclosure must show utility strip in {lang:?}"
            );
            assert!(tree.press_text(&mut core, toggle));
            assert!(
                !tree.has_text(utility),
                "closing onboarding disclosure must hide utility strip in {lang:?}"
            );
        }
    }
}

#[test]
fn batch3_checkbox_redact_toggles_cleanly_zh_en() {
    #[component]
    fn CheckboxHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("b3-chk");
        let menu_open = signal(true);
        let rows = signal(Vec::new());
        let seq_builder = make_seq_builder();
        let compose = make_compose();
        let selected = signal(None);
        let shown_phase = signal(Phase::Empty);
        view()
            .w(px(360.0))
            .h(px(120.0))
            .flex_col()
            .child(menu_redact_row(
                menu_open,
                controls,
                rows,
                seq_builder,
                compose,
                selected,
                shown_phase,
            ))
            .child(text(move || {
                format!("REDACT:{}", seq_builder.redact_export.get())
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<CheckboxHarness>(Size::new(px(360.0), px(120.0)));
        tree.pump(&mut core);
        let label = t(lang, "导出/分享脱敏", "Redact exports");
        assert!(tree.has_text("REDACT:false"));
        assert!(tree.press_text(&mut core, label));
        assert!(
            tree.has_text("REDACT:true"),
            "pressing redact checkbox must check in {lang:?}"
        );
        assert!(tree.press_text(&mut core, label));
        assert!(
            tree.has_text("REDACT:false"),
            "pressing redact checkbox again must uncheck in {lang:?}"
        );
    }
}

#[test]
fn batch3_radio_groups_switch_layout_theme_and_language_cleanly() {
    #[component]
    fn RadioGroupHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        let (appearance, _) = provide_base_contexts(lang);
        let controls = make_controls("b3-radio");
        let profiles = make_profiles();
        let langs = ui_langs();
        view()
            .w(px(480.0))
            .h(px(240.0))
            .flex_col()
            .gap_1()
            .child(settings_layout_row(profiles.layout_mode, controls.clone()))
            .child(menu_theme_row(controls.clone()))
            .child(menu_lang_row(controls))
            .child(text(move || {
                format!(
                    "RADIO|{:?}|{:?}|{:?}",
                    profiles.layout_mode.get(),
                    appearance.theme_mode.get(),
                    langs.get()
                )
            }))
    }

    TEST_LANG.set(Lang::Zh);
    let (mut core, mut tree) = mount::<RadioGroupHarness>(Size::new(px(480.0), px(240.0)));
    tree.pump(&mut core);
    assert!(tree.has_text("RADIO|Split|System|Some(Zh)"));

    // Pressing already-selected radio stays idempotent; pressing sibling selects it exclusively.
    let _ = tree.press_text(&mut core, "左右分栏");
    assert!(tree.has_text("RADIO|Split|System|Some(Zh)"));
    assert!(tree.press_text(&mut core, "上下分栏"));
    assert!(tree.has_text("RADIO|TopBottom|System|Some(Zh)"));

    assert!(tree.press_text(&mut core, "深色"));
    assert!(tree.has_text("RADIO|TopBottom|Dark|Some(Zh)"));
    assert!(tree.press_text(&mut core, "浅色"));
    assert!(tree.has_text("RADIO|TopBottom|Light|Some(Zh)"));

    assert!(tree.press_text(&mut core, "EN"));
    assert!(tree.has_text("RADIO|TopBottom|Light|Some(En)"));
    assert!(tree.has_text("Top / Bottom"));
    assert!(tree.press_text(&mut core, "中文"));
    assert!(tree.has_text("RADIO|TopBottom|Light|Some(Zh)"));
}

#[test]
fn batch3_link_chip_handles_open_in_both_languages() {
    #[component]
    fn LinkHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("b3-link");
        let opened = signal(String::new());
        view()
            .w(px(360.0))
            .h(px(120.0))
            .flex_row()
            .gap_2()
            .child(about_link_chip(
                Glyph::DeviceDesktop,
                tr("官网", "Website"),
                "https://vicanso.dev",
                controls,
            ))
            .child(
                link()
                    .href("https://vicanso.dev")
                    .open_with(LinkOpenWith::App)
                    .on_open(move |href: &SharedString| opened.set(href.to_string()))
                    .child(text("LINK_OPEN_PROBE")),
            )
            .child(text(move || format!("OPENED:{}", opened.get())))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<LinkHarness>(Size::new(px(360.0), px(120.0)));
        tree.pump(&mut core);
        assert!(tree.has_text(t(lang, "官网", "Website")));
        assert!(tree.has_text("OPENED:"));
        assert!(tree.press_text(&mut core, "LINK_OPEN_PROBE"));
        assert!(
            tree.has_text("OPENED:https://vicanso.dev"),
            "nsh_base::link with LinkOpenWith::App must fire on_open with href in {lang:?}"
        );
    }
}

#[test]
fn facet_drawer_sheet_escape_closes_even_while_editing_zh_en() {
    use crate::session::{FacetEntry, FacetKind, SessionFacets};

    #[component]
    fn FacetSheetEscapeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let facets = signal(SessionFacets {
            domains: vec![FacetEntry {
                label: "api.vicanso.dev".into(),
                token: "api.vicanso.dev".into(),
                kind: FacetKind::Domain,
                count: 1,
                error_count: 0,
            }],
            apps: vec![],
        });
        let facets_open = signal(false);
        let facet_tab = signal(0_u8);
        let filter = signal(String::new());
        on_key("escape", move |_| {
            if facets_open.get() {
                facets_open.set(false);
            }
        });
        view()
            .w(px(520.0))
            .h(px(640.0))
            .child(facet_quick_bar(facets, facets_open, facet_tab, filter))
            .child(facet_drawer_overlay(
                facets,
                facets_open,
                facet_tab,
                filter,
                move || facets_open.get(),
                || 360.0,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) =
            mount::<FacetSheetEscapeHarness>(Size::new(px(520.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        let domains_btn = t(lang, "域名 (1)", "Domains (1)");

        assert!(!tree.has_text("api.vicanso.dev"));
        assert!(tree.press_text(&mut core, domains_btn));
        assert!(tree.has_text("api.vicanso.dev"));

        assert!(
            tree.press_escape(&mut core, true),
            "Escape while editing must still fire Sheet binding in {lang:?}"
        );
        assert!(
            !tree.has_text("api.vicanso.dev"),
            "Sheet must close on Escape even when editing in {lang:?}"
        );

        assert!(tree.press_text(&mut core, domains_btn));
        assert!(tree.has_text("api.vicanso.dev"));
        assert!(tree.press_escape(&mut core, false));
        assert!(
            !tree.has_text("api.vicanso.dev"),
            "Sheet must close on normal Escape in {lang:?}"
        );
    }
}

#[test]
fn rules_select_escape_closes_dropdown_before_parent_modal_zh_en() {
    #[component]
    fn SelectEscapeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let panel_open = signal(true);
        let draft = make_rule_draft();
        draft.kind.set(6);
        on_key("escape", move |_| {
            if panel_open.get() {
                panel_open.set(false);
            }
        });
        view()
            .w(px(640.0))
            .h(px(640.0))
            .child(when(
                move || panel_open.get(),
                move || quick_kind_dropdown(draft),
            ))
            .child(text(move || format!("PANEL|{}", panel_open.get())))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<SelectEscapeHarness>(Size::new(px(640.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        let (kind_zh, kind_en) = quick_kind(6).label();
        let trigger_label = match lang {
            Lang::Zh => kind_zh,
            Lang::En => kind_en,
        };
        let group_label = t(lang, "内容改写", "Content Rewrite");

        assert!(tree.has_text("PANEL|true"));
        assert!(!tree.has_text(group_label));
        assert!(tree.press_text(&mut core, trigger_label));
        assert!(
            tree.has_text(group_label),
            "Select dropdown must open on trigger press in {lang:?}"
        );

        // First Escape closes only the Select dropdown, leaving parent panel open.
        assert!(tree.press_escape(&mut core, false));
        assert!(
            !tree.has_text(group_label),
            "first Escape must close Select dropdown in {lang:?}"
        );
        assert!(
            tree.has_text("PANEL|true"),
            "first Escape must not close parent panel in {lang:?}"
        );

        // Second Escape falls through to the parent panel's Escape handler.
        assert!(tree.press_escape(&mut core, false));
        assert!(
            tree.has_text("PANEL|false"),
            "second Escape must close parent panel in {lang:?}"
        );
    }
}

#[test]
fn nested_overlay_escape_closes_innermost_first_zh_en() {
    #[component]
    fn NestedEscapeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        provide_popover_dismiss();
        let root_count = signal(0_u32);
        let outer = signal(true);
        let inner = signal(true);
        on_key("escape", move |_| root_count.update(|n| *n += 1));
        bind_popover_dismiss(outer);
        bind_escape_while(
            move || inner.get(),
            nsh_core::keymap::When::Always,
            move || inner.set(false),
        );
        view()
            .w(px(640.0))
            .h(px(480.0))
            .child(
                button()
                    .on_press(move |_: &()| inner.set(true))
                    .child(text("REOPEN_INNER")),
            )
            .child(text(move || format!("ROOT|{}", root_count.get())))
            .child(text(move || {
                format!("OVERLAY|{}|{}", outer.get(), inner.get())
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<NestedEscapeHarness>(Size::new(px(640.0), px(480.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        assert!(tree.has_text("ROOT|0"));
        assert!(tree.has_text("OVERLAY|true|true"));

        // (a) Editing: inner (When::Always) closes; outer (When::Auto) stays open.
        assert!(tree.press_escape(&mut core, true));
        assert!(tree.has_text("OVERLAY|true|false"));
        assert!(tree.has_text("ROOT|0"));

        // (b) Reopen inner, then non-editing Esc closes inner -> outer -> (c) root.
        assert!(tree.press_text(&mut core, "REOPEN_INNER"));
        assert!(tree.has_text("OVERLAY|true|true"));

        assert!(tree.press_escape(&mut core, false));
        assert!(tree.has_text("OVERLAY|true|false"));
        assert!(tree.has_text("ROOT|0"));

        assert!(tree.press_escape(&mut core, false));
        assert!(tree.has_text("OVERLAY|false|false"));
        assert!(tree.has_text("ROOT|0"));

        assert!(tree.press_escape(&mut core, false));
        assert!(tree.has_text("ROOT|1"));
    }
}

#[test]
fn shortcuts_modal_escape_closes_via_own_binding_zh_en() {
    #[component]
    fn ShortcutsEscapeHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let open = signal(true);
        view()
            .w(px(800.0))
            .h(px(640.0))
            .child(shortcuts_modal(open))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<ShortcutsEscapeHarness>(Size::new(px(800.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);
        let title = t(lang, "导航与视图", "Navigation & View");
        assert!(tree.has_text(title));

        assert!(tree.press_escape(&mut core, false));
        assert!(
            !tree.has_text(title),
            "shortcuts_modal must close on its own Escape binding in {lang:?}"
        );
    }
}

#[test]
fn quick_filter_chips_are_toggles_and_flip_filter_zh_en() {
    #[component]
    fn ToggleChipHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let filter = signal(String::new());
        view()
            .w(px(360.0))
            .h(px(120.0))
            .child(toggle_group().flex_row().gap_1p5().child(filter_chip(
                filter,
                "慢请求",
                "Slow",
                "slow:",
                SLOW_ALIASES,
            )))
            .child(text(move || format!("FILTER|{}", filter.get())))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<ToggleChipHarness>(Size::new(px(360.0), px(120.0)));
        tree.pump(&mut core);
        let label = t(lang, "慢请求", "Slow");
        let text_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.texts.get(id).is_some_and(|s| s == label))
            .expect("chip label");
        let chip_id = tree.parent[&text_id];

        assert!(tree.has_text("FILTER|"));
        assert_eq!(
            tree.pressed.get(&chip_id),
            Some(&Some(nsh_core::Checked::False)),
            "initial toggle chip pressed state must be False in {lang:?}"
        );

        assert!(tree.press_text(&mut core, label));
        assert!(tree.has_text("FILTER|slow:"));
        assert_eq!(
            tree.pressed.get(&chip_id),
            Some(&Some(nsh_core::Checked::True)),
            "active toggle chip pressed state must be True in {lang:?}"
        );

        assert!(tree.press_text(&mut core, label));
        assert!(tree.has_text("FILTER|"));
        assert_eq!(
            tree.pressed.get(&chip_id),
            Some(&Some(nsh_core::Checked::False)),
            "deactivated toggle chip pressed state must return to False in {lang:?}"
        );
    }
}

#[test]
fn folded_header_and_message_sections_stay_hidden_after_async_hydration() {
    #[component]
    fn FoldedHydrationHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("folded-hydrate");
        let shown = make_shown("https://api.vicanso.dev/v1/orders", Some(200), "");
        let folded = signal(FOLD_RES_HEADERS | FOLD_MESSAGES);
        let hydrate_headers = shown.response_headers;
        let hydrate_messages = shown.messages;
        view()
            .w(px(640.0))
            .h(px(600.0))
            .flex_col()
            .gap_2()
            .child(
                button()
                    .on_press(move |_: &()| {
                        hydrate_headers.set(vec![HeaderLine {
                            key: 1,
                            name: "X-Hydrated-Header".into(),
                            value: "hydrated-secret-value".into(),
                            hint: None,
                        }]);
                        hydrate_messages.set(vec![HeaderLine {
                            key: 2,
                            name: "↑ 文本".into(),
                            value: "ws-hydrated-frame".into(),
                            hint: None,
                        }]);
                    })
                    .child(text("HYDRATE_ASYNC")),
            )
            .child(header_section(HeaderSectionProps {
                zh: "响应头",
                en: "Response headers",
                lines: shown.response_headers,
                loading: shown.res_headers_loading,
                pane_width: shown.pane_width,
                folded,
                bit: FOLD_RES_HEADERS,
                controls: controls.clone(),
            }))
            .child(messages_section(shown, controls, folded))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<FoldedHydrationHarness>(Size::new(px(640.0), px(600.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);

        let hdr_title = t(lang, "响应头", "Response headers");
        let msg_title = t(lang, "消息", "Messages");
        assert!(tree.has_text(hdr_title));
        assert!(tree.has_text(msg_title));
        assert!(!tree.has_text("hydrated-secret-value"));
        assert!(!tree.has_text("ws-hydrated-frame"));

        // Simulate async `/api/flows/{id}` payload arriving while cards are pre-folded.
        assert!(tree.press_text(&mut core, "HYDRATE_ASYNC"));
        assert!(
            !tree.has_text("hydrated-secret-value"),
            "pre-folded header_section leaked content on async hydration in {lang:?}"
        );
        assert!(
            !tree.has_text("ws-hydrated-frame"),
            "pre-folded messages_section leaked content on async hydration in {lang:?}"
        );

        // Expanding each section reveals the hydrated rows; collapsing hides them again.
        assert!(tree.press_text(&mut core, hdr_title));
        assert!(tree.has_text("hydrated-secret-value"));
        assert!(tree.press_text(&mut core, hdr_title));
        assert!(!tree.has_text("hydrated-secret-value"));

        assert!(tree.press_text(&mut core, msg_title));
        assert!(tree.has_text("ws-hydrated-frame"));
        assert!(tree.press_text(&mut core, msg_title));
        assert!(!tree.has_text("ws-hydrated-frame"));
    }
}

#[test]
fn escape_closes_marker_command_palette_and_watch_while_editing_and_toast_visible() {
    #[component]
    fn EscapeWhileEditingAndToastHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let _dismiss = provide_popover_dismiss();
        let controls = make_controls("esc-editing-toast");
        let profiles = make_profiles();
        let compose = make_compose();
        let seq_builder = make_seq_builder();
        let shown = make_shown("https://api.vicanso.dev/v1/health", Some(200), "");
        let rows = signal(Vec::new());
        let selected = signal(None::<u64>);
        let baseline = signal(None::<u64>);
        let active_tab = signal(0_u64);
        let filter = signal(String::new());
        let list_filter = signal(String::new());
        let primary_filter = signal(String::new());
        let filter_enabled = signal(true);
        let pretty = signal(true);
        let running = signal(false);
        let proxy_on = signal(false);
        let filter_focus = element_ref();

        let marker_open = signal(true);
        let palette_open = signal(false);
        let shortcuts_open = signal(false);
        let watch_open = signal(false);
        let watch_rules = signal(Vec::new());
        let watch_eval = signal(crate::watch::WatchEvaluation::default());

        let toast_manager = stored(ToastManager::<u64, StatusMsg>::new(ToastMotion::sonner()));
        let initial_toasts = toast_manager.with(|m| {
            let now = nsh_core::time::now();
            m.push(
                1_u64,
                StatusMsg::ClearedFlows { count: 1 },
                ToastOptions::timeout(Some(Duration::from_millis(5000))),
                now,
            );
            m.visible(3)
                .map(|(id, data, status)| (*id, data.clone(), status))
                .collect::<Vec<_>>()
        });
        let toast_shown = signal(initial_toasts);
        let toast_gen = signal(1_u64);
        let toast_state = ToastStackState::new();

        view()
            .w(px(900.0))
            .h(px(640.0))
            .flex_col()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(
                        button()
                            .on_press(move |_: &()| watch_open.set(true))
                            .child(text("OPEN_WATCH")),
                    )
                    .child(
                        button()
                            .on_press(move |_: &()| palette_open.set(true))
                            .child(text("OPEN_PALETTE")),
                    ),
            )
            .child(text(move || {
                format!(
                    "STATE|m={}|p={}|w={}",
                    marker_open.get(),
                    palette_open.get(),
                    watch_open.get()
                )
            }))
            .child(watch_popover(WatchPopoverProps {
                open: watch_open,
                rules: watch_rules,
                eval: watch_eval,
                effective_filter: filter,
                list_filter,
                selected,
                active_tab,
                baseline,
                primary_filter,
                filter_enabled,
                controls: controls.clone(),
            }))
            .child(marker_modal(marker_open, controls.clone()))
            .child(command_palette_modal(
                palette_open,
                shortcuts_open,
                filter_focus,
                controls.clone(),
                running,
                proxy_on,
                rows,
                filter,
                pretty,
                profiles,
                compose,
                seq_builder,
                selected,
                baseline,
                shown,
            ))
            .child(toast_overlay(
                toast_shown,
                toast_gen,
                toast_state,
                toast_manager,
                controls,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) =
            mount::<EscapeWhileEditingAndToastHarness>(Size::new(px(900.0), px(640.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);

        // 1. Session Marker closes on Escape even while editing an input AND toast is visible.
        assert!(tree.has_text("STATE|m=true|p=false|w=false"));
        assert!(tree.press_escape(&mut core, true));
        assert!(
            tree.has_text("STATE|m=false|p=false|w=false"),
            "marker_modal must close on Escape while editing with toast visible in {lang:?}"
        );

        // 2. Watch popover opens, closes on Escape while editing AND toast is visible.
        assert!(tree.press_text(&mut core, "OPEN_WATCH"));
        assert!(tree.has_text("STATE|m=false|p=false|w=true"));
        assert!(tree.press_escape(&mut core, true));
        assert!(
            tree.has_text("STATE|m=false|p=false|w=false"),
            "watch_popover must close on Escape while editing with toast visible in {lang:?}"
        );

        // 3. Opening Command Palette while Watch popover is open dismisses Watch,
        // and Escape while editing closes Command Palette even with toast visible.
        assert!(tree.press_text(&mut core, "OPEN_WATCH"));
        assert!(tree.has_text("STATE|m=false|p=false|w=true"));
        assert!(tree.press_text(&mut core, "OPEN_PALETTE"));
        assert!(
            tree.has_text("STATE|m=false|p=true|w=false"),
            "opening command_palette must dismiss background watch_popover in {lang:?}"
        );
        assert!(tree.press_escape(&mut core, true));
        assert!(
            tree.has_text("STATE|m=false|p=false|w=false"),
            "command_palette must close on Escape while editing with toast visible in {lang:?}"
        );
    }
}

#[test]
fn marker_modal_submit_and_detached_spawn_survive_immediate_modal_close() {
    #[component]
    fn MarkerSubmitHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("marker-submit");
        let open = signal(true);
        let attached_ran = signal(false);
        let detached_ran = signal(false);
        let hub_for_count = controls.hub.clone();
        let refresh = signal(0_u64);
        view()
            .w(px(720.0))
            .h(px(520.0))
            .flex_col()
            .child(marker_modal(open, controls))
            .child(when(
                move || !open.get(),
                move || {
                    view().child(
                        button()
                            .on_press(move |_: &()| {
                                refresh.update(|n| *n += 1);
                            })
                            .child(text("SYNC_COUNT")),
                    )
                },
            ))
            .child(when(
                move || !detached_ran.get(),
                move || {
                    let close_flag = signal(true);
                    when(
                        move || close_flag.get(),
                        move || {
                            button()
                                .on_press(move |_: &()| {
                                    // Attached spawn is cancelled when `close_flag.set(false)`
                                    // disposes this `when(...)` owner.
                                    nsh_core::prelude::spawn(async move {
                                        attached_ran.set(true);
                                    });
                                    // Detached spawn (`hub_web::spawn_detached`) survives disposal.
                                    nsh_core::prelude::Owner::detached().run(|| {
                                        nsh_core::prelude::spawn(async move {
                                            detached_ran.set(true);
                                        });
                                    });
                                    close_flag.set(false);
                                })
                                .child(text("RUN_DETACHED"))
                        },
                    )
                },
            ))
            .child(text(move || {
                let _ = refresh.get();
                let (count, last) = hub_for_count.with_session(|s| {
                    (
                        s.len(),
                        s.iter().last().map(|f| f.path.clone()).unwrap_or_default(),
                    )
                });
                format!(
                    "MARKERS|open={}|count={}|last={}|attached={}|detached={}",
                    open.get(),
                    count,
                    last,
                    attached_ran.get(),
                    detached_ran.get()
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<MarkerSubmitHarness>(Size::new(px(720.0), px(520.0)));
        core.set_environment(nsh_core::element::Environment {
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);

        assert!(tree.has_text("MARKERS|open=true|count=0|last=|attached=false|detached=false"));
        let submit_label = t(lang, "插入标记", "Add marker");
        assert!(tree.press_text(&mut core, submit_label));
        assert!(tree.press_text(&mut core, "SYNC_COUNT"));
        let expected_default = t(lang, "时间标记", "Marker");
        assert!(
            tree.has_text(&format!(
                "MARKERS|open=false|count=1|last={expected_default}|attached=false|detached=false"
            )),
            "clicking {submit_label} must add marker and close modal in {lang:?}"
        );

        assert!(tree.press_text(&mut core, "RUN_DETACHED"));
        core.run_component_tasks();
        tree.pump(&mut core);
        assert!(
            tree.has_text(&format!(
                "MARKERS|open=false|count=1|last={expected_default}|attached=false|detached=true"
            )),
            "attached spawn should cancel while detached spawn completes after when-owner disposal"
        );
    }
}

#[test]
fn narrow_split_layout_keeps_status_column_visible_without_overriding_user_widths() {
    let shown = ListColumns::from_config(&config::Config::default());

    // 780px Split layout (sidebar auto-collapses < 880px): list pane clamps to 440px.
    let list_w_780 = clamp_list_px_for_layout(
        config::LIST_WIDTH_DEFAULT as f32,
        780.0,
        config::LayoutMode::Split,
        0.0,
    );
    assert_eq!(list_w_780, 440.0);

    let cols = flow_table_columns_for_layout(shown, &HashMap::new(), false, list_w_780);
    let width_of = |key: &str| {
        cols.iter()
            .find(|c| c.key.as_ref() == key)
            .and_then(|c| c.width)
            .map(|p| p.0)
            .unwrap_or(0.0)
    };
    let core_total = width_of("id") + width_of("method") + width_of("path") + width_of("status");
    assert_eq!(width_of("status"), 52.0);
    assert!(
        core_total <= list_w_780 - 8.0,
        "core columns ({core_total}px) must fit within 780px split pane ({list_w_780}px)"
    );
    assert!(width_of("path") >= 120.0 && width_of("path") < 300.0);

    // Wide 1280px Split pane keeps default path width (300px) intact.
    let wide_cols = flow_table_columns_for_layout(shown, &HashMap::new(), false, 660.0);
    let wide_path = wide_cols
        .iter()
        .find(|c| c.key.as_ref() == "path")
        .and_then(|c| c.width)
        .map(|p| p.0);
    assert_eq!(wide_path, Some(300.0));

    // User-resized path width is respected and never auto-shrunk.
    let mut custom = HashMap::new();
    custom.insert("path".to_string(), 320_u32);
    let custom_cols = flow_table_columns_for_layout(shown, &custom, false, list_w_780);
    let custom_path = custom_cols
        .iter()
        .find(|c| c.key.as_ref() == "path")
        .and_then(|c| c.width)
        .map(|p| p.0);
    assert_eq!(custom_path, Some(320.0));
}

#[test]
fn a11y_backdrop_hidden_inputs_labeled_and_native_layout_and_kind_controls() {
    #[component]
    fn A11yControlsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("a11y-controls");
        let layout_mode = signal(config::LayoutMode::Split);
        let draft = make_rule_draft();
        let backdrop_closed = signal(false);
        let sample_val = signal(String::new());
        view()
            .w(px(900.0))
            .h(px(400.0))
            .flex_col()
            .gap_2()
            .child(
                ui_modal_backdrop(move || backdrop_closed.set(true))
                    .child(text("BACKDROP_INNER_MARKER")),
            )
            .child(
                ui_input_a11y(input(), tr("测试域名输入", "Test host input"))
                    .value(move || sample_val.get())
                    .placeholder(tr("输入 api.vicanso.dev", "Enter api.vicanso.dev")),
            )
            .child(toolbar_layout_switcher(layout_mode, controls))
            .child(quick_kind_dropdown(draft))
            .child(text(move || {
                format!("BACKDROP_CLOSED|{}", backdrop_closed.get())
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<A11yControlsHarness>(Size::new(px(900.0), px(400.0)));
        tree.pump(&mut core);

        // 1. Backdrop is hidden from accessibility and non-focusable, yet still dismisses on press.
        let marker_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| {
                tree.texts
                    .get(id)
                    .is_some_and(|s| s == "BACKDROP_INNER_MARKER")
            })
            .expect("backdrop marker");
        let backdrop_id = tree.parent[&marker_id];
        assert_eq!(tree.elements_hidden.get(&backdrop_id), Some(&true));
        assert_eq!(tree.focusable.get(&backdrop_id), Some(&Some(false)));
        assert!(tree.press_text(&mut core, "BACKDROP_INNER_MARKER"));
        assert!(tree.has_text("BACKDROP_CLOSED|true"));

        // 2. ui_input_a11y sets explicit label and native Textbox role.
        let expected_input_label = t(lang, "测试域名输入", "Test host input");
        let input_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.kinds.get(id) == Some(&ViewKind::Input))
            .expect("input node");
        assert_eq!(
            tree.labels.get(&input_id).and_then(|l| l.as_deref()),
            Some(expected_input_label)
        );
        assert_eq!(tree.roles.get(&input_id), Some(&AccessibilityRole::Textbox));

        // 3. quick_kind_dropdown exposes native Combobox role with localized label.
        let kind_label = t(lang, "选择规则类型", "Select rule type");
        let combobox_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.labels.get(id).and_then(|l| l.as_deref()) == Some(kind_label))
            .expect("quick_kind_dropdown combobox");
        assert_eq!(
            tree.roles.get(&combobox_id),
            Some(&AccessibilityRole::Combobox)
        );

        // 4. Layout switcher opens native Button menu items with labels.
        let switcher_label = t(lang, "分栏", "Layout");
        assert!(tree.press_text(&mut core, switcher_label));
        let tb_label = t(lang, "上下分栏", "Top / Bottom split");
        let tb_btn = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.labels.get(id).and_then(|l| l.as_deref()) == Some(tb_label))
            .expect("top/bottom layout button");
        assert_eq!(tree.roles.get(&tb_btn), Some(&AccessibilityRole::Button));
    }
}

#[test]
fn pane_splitter_stacks_above_detail_pane_in_split_and_top_bottom_modes() {
    #[component]
    fn SplitterStackingHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("splitter-z");
        let list_width = signal(config::LIST_WIDTH_DEFAULT as f32);
        let list_height = signal(320.0_f32);
        let layout_mode = signal(config::LayoutMode::Split);
        let sidebar_width = signal(config::SIDEBAR_WIDTH_DEFAULT as f32);
        let sidebar_collapsed = signal(false);

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        layout_mode.set(config::LayoutMode::TopBottom);
                    })
                    .child(text("TO_TOP_BOTTOM")),
            )
            .child(
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex_row()
                    .child(view().w(px(500.0)).h_full().child(text("FLOW_LIST_STUB")))
                    .child(pane_splitter(
                        list_width,
                        list_height,
                        layout_mode,
                        sidebar_width,
                        sidebar_collapsed,
                        controls,
                    ))
                    .child(view().flex_1().h_full().child(text("DETAIL_PANE_STUB"))),
            )
    }

    TEST_LANG.set(Lang::En);
    let (mut core, mut tree) = mount::<SplitterStackingHarness>(Size::new(px(1200.0), px(760.0)));
    tree.pump(&mut core);

    let assert_splitter_after_detail = |tree: &HeadlessTree, handle_label: &str| {
        let detail_text = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.texts.get(id).is_some_and(|s| s == "DETAIL_PANE_STUB"))
            .expect("detail stub text");
        let detail_pane = tree.parent[&detail_text];
        let container = tree.parent[&detail_pane];
        let handle_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| tree.labels.get(id).and_then(|l| l.as_deref()) == Some(handle_label))
            .expect("resize handle");
        let mut splitter_top = handle_id;
        while let Some(&p) = tree.parent.get(&splitter_top) {
            if p == container {
                break;
            }
            splitter_top = p;
        }
        assert_eq!(tree.parent.get(&splitter_top), Some(&container));
        let siblings = &tree.children[&container];
        let detail_pos = siblings
            .iter()
            .position(|&id| id == detail_pane)
            .expect("detail pane in container");
        let splitter_pos = siblings
            .iter()
            .position(|&id| id == splitter_top)
            .expect("splitter in container");
        assert!(
            splitter_pos > detail_pos,
            "{handle_label} (pos {splitter_pos}) must stack after detail_pane (pos {detail_pos}) so center/trailing hit area is unoccluded"
        );
    };

    assert_splitter_after_detail(&tree, "Resize list and detail width");
    assert!(tree.press_text(&mut core, "TO_TOP_BOTTOM"));
    assert_splitter_after_detail(&tree, "Resize list and detail height");
}

#[test]
fn llm_stream_panel_renders_individual_wrapping_metric_badges_and_zh_punctuation() {
    #[component]
    fn LlmBadgesHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("llm-badges");
        let summary = signal(Some(llm_stream::LlmStreamSummary {
            provider: llm_stream::LlmProvider::OpenAiChat,
            model: Some("gpt-4o-mini".into()),
            finish_reason: Some("tool_calls".into()),
            thinking: "Step 1: inspect api.vicanso.dev".into(),
            content: "All checks green.".into(),
            tool_calls: vec![llm_stream::LlmToolCall {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: r#"{"city":"Shanghai"}"#.into(),
            }],
            usage: llm_stream::LlmUsage {
                input_tokens: Some(42),
                output_tokens: Some(18),
                reasoning_tokens: Some(10),
                total_tokens: Some(60),
            },
            chunk_count: 4,
            ttft_ms: Some(12),
            total_ms: Some(250),
        }));
        view()
            .w(px(360.0))
            .h(px(520.0))
            .child(llm_stream_panel(summary, controls))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<LlmBadgesHarness>(Size::new(px(360.0), px(520.0)));
        tree.pump(&mut core);

        assert!(tree.has_text("OpenAI Chat"));
        assert!(tree.has_text("gpt-4o-mini"));
        assert!(tree.has_text("TTFT 12ms"));
        assert!(tree.has_text("75.6 tok/s"));
        // Metrics must render as separate wrapping badges, never one long concatenated line.
        assert!(!tree.has_text_containing("TTFT 12ms · "));
        match lang {
            Lang::Zh => {
                assert!(tree.has_text("总耗时 250ms"));
                assert!(tree.has_text("输入：42 输出：18 思考：10（60）"));
                assert!(tree.has_text("4 帧"));
                assert!(tree.has_text("结束原因：tool_calls"));
            }
            Lang::En => {
                assert!(tree.has_text("Total 250ms"));
                assert!(tree.has_text("in:42 out:18 reason:10 (60)"));
                assert!(tree.has_text("4 chunks"));
                assert!(tree.has_text("finish: tool_calls"));
            }
        }
    }
}

#[test]
fn config_broken_bar_dismiss_reopen_and_backup_reset_clears_error_zh_en() {
    #[component]
    fn ConfigBrokenBarHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("cfg-broken-bar");
        let cfg_dir = ca::data_dir();
        let dev = config::is_dev_mode();
        let cfg_path = config::config_path_for(&cfg_dir, dev);
        let _ = std::fs::create_dir_all(&cfg_dir);
        std::fs::write(&cfg_path, "listen_port = [broken toml\n").expect("write broken config");
        *controls.config_path.lock().unwrap() = cfg_path.display().to_string();
        *controls.config_error.lock().unwrap() = Some("invalid array syntax".to_string());

        let config_broken = signal(Some("invalid array syntax".to_string()));
        let config_broken_dismissed = signal(false);
        let profiles = make_profiles();
        let port_text = signal("8878".to_string());
        let listen_all = signal(false);
        let bar_controls = controls.clone();
        let check_controls = controls;

        view()
            .w(px(900.0))
            .h(px(240.0))
            .flex_col()
            .child(when(
                move || config_broken.get().is_some() && !config_broken_dismissed.get(),
                move || {
                    config_broken_bar(
                        config_broken,
                        config_broken_dismissed,
                        profiles,
                        port_text,
                        listen_all,
                        bar_controls.clone(),
                    )
                },
            ))
            .child(footer_config_error_button(
                config_broken,
                config_broken_dismissed,
            ))
            .child(text(move || {
                let ctrl_err = check_controls
                    .config_error
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_default();
                format!(
                    "CFG_STATE|broken={}|dismissed={}|ctrl_err={}",
                    config_broken.get().is_some(),
                    config_broken_dismissed.get(),
                    ctrl_err
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<ConfigBrokenBarHarness>(Size::new(px(900.0), px(240.0)));
        tree.pump(&mut core);

        let reset_label = t(lang, "备份并重置", "Backup & reset");
        let footer_label = t(lang, "配置错误", "Config error");
        let dismiss_label = t(lang, "隐藏横幅", "Dismiss banner");

        // 1. Initial broken state shows both banner and footer indicator.
        assert!(tree.has_text(reset_label));
        assert!(tree.has_text(footer_label));
        assert!(
            tree.has_text("CFG_STATE|broken=true|dismissed=false|ctrl_err=invalid array syntax")
        );

        // 2. Dismiss hides banner while keeping footer reopen pill visible.
        assert!(tree.press_label(&mut core, dismiss_label));
        assert!(!tree.has_text(reset_label));
        assert!(tree.has_text(footer_label));
        assert!(
            tree.has_text("CFG_STATE|broken=true|dismissed=true|ctrl_err=invalid array syntax")
        );

        // 3. Clicking footer `Config error` / `配置错误` pill reopens banner.
        assert!(tree.press_text(&mut core, footer_label));
        assert!(tree.has_text(reset_label));
        assert!(
            tree.has_text("CFG_STATE|broken=true|dismissed=false|ctrl_err=invalid array syntax")
        );

        // 4. Clicking `Backup & reset` / `备份并重置` clears banner, footer pill, and error state.
        assert!(tree.press_text(&mut core, reset_label));
        assert!(!tree.has_text(reset_label));
        assert!(!tree.has_text(footer_label));
        assert!(tree.has_text("CFG_STATE|broken=false|dismissed=false|ctrl_err="));

        // Verify valid replacement config was written inside the isolated test data dir.
        let cfg_dir = ca::data_dir();
        let (_cfg, load_err) = config::load_mode_ext(&cfg_dir, config::is_dev_mode());
        assert!(
            load_err.is_none(),
            "reset config must parse cleanly in {lang:?}: {:?}",
            load_err
        );
    }
}

#[test]
fn font_settings_preset_override_partial_preservation_blur_and_escape_zh_en() {
    #[component]
    fn FontSettingsHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let font = mono_font();
        let controls = make_controls("font-settings");
        view()
            .w(px(760.0))
            .h(px(720.0))
            .flex_col()
            .child(settings_font_section(controls))
            .child(text(move || {
                let current = theme();
                format!(
                    "FONT_STATE|preset={}|ui={}|mono={}|sans_tok={}|mono_tok={}",
                    font.preset.get().as_str(),
                    font.ui_font.get(),
                    font.mono_font.get(),
                    current.font_sans.as_deref().unwrap_or("<none>"),
                    current.font_mono.as_deref().unwrap_or("<none>"),
                )
            }))
    }

    let find_input_by_placeholder = |tree: &HeadlessTree, needle: &str| -> NodeId {
        tree.kinds
            .iter()
            .find_map(|(id, kind)| {
                (*kind == nsh_core::ViewKind::Input
                    && tree
                        .placeholders
                        .get(id)
                        .is_some_and(|p| p.contains(needle)))
                .then_some(*id)
            })
            .unwrap_or_else(|| panic!("missing input with placeholder `{needle}`"))
    };

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<FontSettingsHarness>(Size::new(px(760.0), px(720.0)));
        core.set_environment(nsh_core::element::Environment {
            window: Size::new(px(760.0), px(720.0)),
            os: nsh_core::platform::Os::MacOs,
            idiom: nsh_core::platform::Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        });
        tree.pump(&mut core);

        let customized_badge = t(lang, "已自定义", "Customized");
        let dev_pill = t(lang, "开发者", "Developer");
        let select_ui_label = t(lang, "选择界面字体", "Select UI font");
        let reset_label = t(lang, "恢复默认", "Reset");
        let ui_input_id = find_input_by_placeholder(&tree, "Inter");
        let mono_input_id = find_input_by_placeholder(&tree, "Fira Code");

        assert!(tree.has_text_containing("FONT_STATE|preset=default|ui=|mono=|sans_tok=<none>|"));
        assert!(!tree.has_text(customized_badge));

        // 1. Enter custom UI and Mono stacks and submit both.
        let custom_ui = "Inter, PingFang SC, sans-serif";
        let custom_mono = "Fira Code, Menlo, monospace";
        core.component_event(
            ui_input_id,
            EventKind::SubmitEditing,
            &EventPayload::Text {
                text: custom_ui.into(),
                count: 1,
            },
        );
        core.component_event(
            mono_input_id,
            EventKind::SubmitEditing,
            &EventPayload::Text {
                text: custom_mono.into(),
                count: 1,
            },
        );
        tree.pump(&mut core);
        assert!(tree.has_text(customized_badge));
        assert!(tree.has_text_containing(&format!(
            "ui={custom_ui}|mono={custom_mono}|sans_tok={custom_ui}|mono_tok={custom_mono}"
        )));

        // 2. Dirty A -> B + late blur guard: type uncommitted text into mono input,
        // click Developer preset, then emit a late Blur on mono input.
        core.component_event(
            mono_input_id,
            EventKind::ChangeText,
            &EventPayload::Text {
                text: "Cascadia Mono".into(),
                count: 2,
            },
        );
        tree.pump(&mut core);
        assert!(tree.press_text(&mut core, dev_pill));
        core.component_event(mono_input_id, EventKind::Blur, &EventPayload::None);
        tree.pump(&mut core);

        assert!(!tree.has_text(customized_badge));
        assert!(tree.has_text_containing("FONT_STATE|preset=developer|ui=|mono=|"));
        assert!(tree.has_text_containing("sans_tok=Inter, SF Pro Text,"));
        assert!(tree.has_text_containing("mono_tok=Fira Code, JetBrains Mono NL,"));
        let saved_after_dev = saved_config();
        assert_eq!(saved_after_dev.font_preset, config::FontPreset::Developer);
        assert!(saved_after_dev.ui_font.is_empty());
        assert!(saved_after_dev.mono_font.is_empty());

        // 3. Partial override A -> A' via Blur: set UI font only, keep Developer mono stack intact.
        core.component_event(
            ui_input_id,
            EventKind::ChangeText,
            &EventPayload::Text {
                text: "Georgia".into(),
                count: 3,
            },
        );
        core.component_event(ui_input_id, EventKind::Blur, &EventPayload::None);
        tree.pump(&mut core);
        assert!(tree.has_text(customized_badge));
        assert!(tree.has_text_containing("FONT_STATE|preset=developer|ui=Georgia|mono=|"));
        assert!(tree.has_text_containing("sans_tok=Georgia, "));
        assert!(tree.has_text_containing("mono_tok=Fira Code, JetBrains Mono NL,"));

        // 4. Open UI font select dropdown and close it with Escape even while editing=true.
        assert!(tree.press_label(&mut core, select_ui_label));
        let dropdown_item = t(lang, "微软雅黑 UI", "Microsoft YaHei UI");
        assert!(tree.has_text(dropdown_item));
        assert!(tree.press_escape(&mut core, true));
        assert!(!tree.has_text(dropdown_item));

        // 5. Reset restores Default preset and clears overrides.
        assert!(tree.press_text(&mut core, reset_label));
        assert!(!tree.has_text(customized_badge));
        assert!(tree.has_text_containing("FONT_STATE|preset=default|ui=|mono=|sans_tok=<none>|"));
        let saved_reset = saved_config();
        assert_eq!(saved_reset.font_preset, config::FontPreset::Default);
        assert!(saved_reset.ui_font.is_empty());
        assert!(saved_reset.mono_font.is_empty());
    }
}

#[test]
fn tray_close_destroys_workspace_and_reopen_restores_session_and_live_state() {
    use std::cell::RefCell;

    type CounterPair = (Rc<Cell<u32>>, Rc<Cell<u32>>);

    thread_local! {
        static TEST_CONTROLS: RefCell<Option<Arc<Controls>>> = const { RefCell::new(None) };
        static TEST_COUNTERS: RefCell<CounterPair> =
            RefCell::new((Rc::new(Cell::new(0)), Rc::new(Cell::new(0))));
    }

    fn record_flow(controls: &Controls, path: &str) {
        controls.hub.with_session(|session| {
            session.open(crate::session::FlowDraft {
                protocol: crate::session::Protocol::Http2,
                method: "GET".into(),
                host: "api.vicanso.dev".into(),
                path: path.into(),
                url: format!("https://api.vicanso.dev{path}"),
                request_headers: Vec::new(),
            });
        });
        controls.hub.touch();
    }

    fn workspace_body(
        rt: AppRuntime,
        mounts: Rc<Cell<u32>>,
        disposals: Rc<Cell<u32>>,
    ) -> impl IntoElement {
        mounts.set(mounts.get() + 1);
        cleanup(move || disposals.set(disposals.get() + 1));

        let local_cleared = signal(0_u32);
        let seen_clear = stored(rt.tray_clear_rev.get_untracked());
        let clear_sig = rt.tray_clear_rev;
        effect(move || {
            let rev = clear_sig.get();
            if seen_clear.replace(rev) != rev {
                local_cleared.set(local_cleared.get_untracked() + 1);
            }
        });

        let open_modals_rt = rt.clone();
        let controls = rt.controls.clone();
        let running = rt.running;
        let listen = rt.listen;
        view()
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        open_modals_rt.settings_open.set(true);
                        open_modals_rt.about_open.set(true);
                    })
                    .child(text("OPEN_MODALS")),
            )
            .child(text(move || {
                let _ = listen.get();
                let _ = running.get();
                let paths: Vec<String> = controls
                    .hub
                    .with_session(|session| session.iter().map(|flow| flow.path.clone()).collect());
                format!(
                    "WORKSPACE_LIVE|flows={}|paths={}|running={}|listen={}|cleared={}",
                    paths.len(),
                    paths.join(","),
                    running.get(),
                    listen.get(),
                    local_cleared.get(),
                )
            }))
    }

    #[component]
    fn TrayLifecycleHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("tray-lifecycle");
        *controls.running.lock().unwrap() = true;
        *controls.listen.lock().unwrap() = "127.0.0.1:8877".into();
        record_flow(&controls, "/before-close");

        let (mounts, disposals) = TEST_COUNTERS.with(|c| c.borrow().clone());
        let now = clock::now();
        let rt = AppRuntime {
            controls: controls.clone(),
            langs: signal(Some(lang)),
            show_tray: signal(true),
            minimize_to_tray: signal(true),
            main_open: signal(true),
            filter: signal(String::new()),
            filter_enabled: signal(true),
            list_filter: signal(String::new()),
            selected: signal(None),
            baseline: signal(None),
            sort: signal(None),
            main_win: signal(None),
            last_placement: stored(None),
            has_mounted_once: stored(false),
            running: signal(true),
            listen: signal("127.0.0.1:8877".into()),
            use_system_proxy: signal(false),
            about_open: signal(false),
            settings_open: signal(false),
            watch_rules: signal(Vec::new()),
            watch_eval: signal(crate::watch::WatchEvaluation::default()),
            watch_tracker: stored(crate::watch::WatchTracker::default()),
            seen_watch_rules: stored(Vec::new()),
            seen_watch_gen: stored(u64::MAX),
            spill_gc_at: stored(now),
            update_probe_at: stored(now),
            tray_clear_rev: signal(0),
            cached_active_layout: stored(None),
        };
        TEST_CONTROLS.with(|slot| *slot.borrow_mut() = Some(controls));

        let rt_change = rt.clone();
        let rt_close = rt.clone();
        let rt_content = rt.clone();
        let rt_maint = rt.clone();
        let rt_show = rt.clone();
        let rt_clear = rt.clone();
        let rt_tray_close = rt.clone();
        let root_rt = rt;
        let m_root = mounts.clone();
        let d_root = disposals.clone();

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .child(
                nsh_core::element::desktop::window()
                    .id("main")
                    .open(root_rt.main_open)
                    .on_open_change(move |ev| on_main_window_open_change(&rt_change, ev.open))
                    .on_close_press(move |_| close_main_window_to_tray(&rt_close))
                    .content(move || {
                        rt_content
                            .main_win
                            .set(Some(nsh_core::element::desktop::current_window()));
                        workspace_body(rt_content.clone(), mounts.clone(), disposals.clone())
                    }),
            )
            .child(
                button()
                    .on_press(move |_: &()| run_background_maintenance(&rt_maint))
                    .child(text("DO_MAINTENANCE")),
            )
            .child(
                button()
                    .on_press(move |_: &()| reveal_main_window(rt_show.main_win, rt_show.main_open))
                    .child(text("TRAY_SHOW")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        if clear_captured_session(&rt_clear.controls) {
                            rt_clear
                                .tray_clear_rev
                                .set(rt_clear.tray_clear_rev.get_untracked().wrapping_add(1));
                        }
                    })
                    .child(text("TRAY_CLEAR")),
            )
            .child(
                button()
                    .on_press(move |_: &()| close_main_window_to_tray(&rt_tray_close))
                    .child(text("TRAY_CLOSE")),
            )
            .child(text(move || {
                format!(
                    "ROOT_STATE|open={}|settings={}|about={}|mounts={}|disposals={}|running={}|listen={}",
                    root_rt.main_open.get(),
                    root_rt.settings_open.get(),
                    root_rt.about_open.get(),
                    m_root.get(),
                    d_root.get(),
                    root_rt.running.get(),
                    root_rt.listen.get(),
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let mounts = Rc::new(Cell::new(0_u32));
        let disposals = Rc::new(Cell::new(0_u32));
        TEST_COUNTERS.with(|c| *c.borrow_mut() = (mounts.clone(), disposals.clone()));

        let mut core = Core::new(ApproxTextEngine, Size::new(px(1200.0), px(760.0)));
        let mut registry = Registry::new();
        registry.register::<TrayLifecycleHarness>();
        core.set_components(registry);
        core.set_environment(nsh_core::element::Environment {
            window: Size::new(px(1200.0), px(760.0)),
            os: nsh_core::platform::Os::MacOs,
            idiom: nsh_core::platform::Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        });
        let comp_name = std::any::type_name::<TrayLifecycleHarness>();
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(comp_name.into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply root op");
        }
        let mut tree = HeadlessTree::default();
        tree.pump(&mut core);

        assert!(tree.press_text(&mut core, "OPEN_MODALS"));
        assert!(tree.has_text("ROOT_STATE|open=true|settings=true|about=true|mounts=1|disposals=0|running=true|listen=127.0.0.1:8877"));
        assert!(tree.has_text(
            "WORKSPACE_LIVE|flows=1|paths=/before-close|running=true|listen=127.0.0.1:8877|cleared=0"
        ));

        let controls = TEST_CONTROLS
            .with(|slot| slot.borrow().clone())
            .expect("controls");

        // 1. Simulate pressing main window close while minimize_to_tray is enabled -> unmounts workspace cleanly.
        let modal_id = tree
            .kinds
            .iter()
            .find_map(|(id, kind)| (*kind == ViewKind::Modal).then_some(*id))
            .expect("main window Modal node");
        core.component_event(
            modal_id,
            EventKind::Window,
            &EventPayload::Window(Box::new(nsh_core::desktop::WindowEvent::ClosePress {
                reason: nsh_core::desktop::CloseReason::CloseButton,
            })),
        );
        tree.pump(&mut core);

        assert!(tree.has_text("ROOT_STATE|open=false|settings=false|about=false|mounts=1|disposals=1|running=true|listen=127.0.0.1:8877"));
        assert!(!tree.has_text_containing("WORKSPACE_LIVE|"));

        // 2. Capture another flow + update listen port while closed in tray, run background maintenance.
        record_flow(&controls, "/while-in-tray");
        *controls.listen.lock().unwrap() = "127.0.0.1:19999".into();
        assert!(tree.press_text(&mut core, "DO_MAINTENANCE"));
        assert!(tree.has_text("ROOT_STATE|open=false|settings=false|about=false|mounts=1|disposals=1|running=true|listen=127.0.0.1:19999"));

        // 3. Reopen via tray "Show Window" -> mounts new workspace and restores all live session flows & state.
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.has_text("ROOT_STATE|open=true|settings=false|about=false|mounts=2|disposals=1|running=true|listen=127.0.0.1:19999"));
        assert!(tree.has_text("WORKSPACE_LIVE|flows=2|paths=/before-close,/while-in-tray|running=true|listen=127.0.0.1:19999|cleared=0"));

        // 4. Tray "Clear Traffic" while window is open bumps tray_clear_rev and clears live session.
        assert!(tree.press_text(&mut core, "TRAY_CLEAR"));
        assert!(tree.has_text(
            "WORKSPACE_LIVE|flows=0|paths=|running=true|listen=127.0.0.1:19999|cleared=1"
        ));

        // 5. Close again, record new flow while in tray, reopen again cleanly.
        assert!(tree.press_text(&mut core, "TRAY_CLOSE"));
        assert!(tree.has_text("ROOT_STATE|open=false|settings=false|about=false|mounts=2|disposals=2|running=true|listen=127.0.0.1:19999"));
        record_flow(&controls, "/after-clear");
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.has_text("ROOT_STATE|open=true|settings=false|about=false|mounts=3|disposals=2|running=true|listen=127.0.0.1:19999"));
        assert!(tree.has_text("WORKSPACE_LIVE|flows=1|paths=/after-clear|running=true|listen=127.0.0.1:19999|cleared=0"));

        TEST_CONTROLS.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn layout_render_switches_builtin_and_custom_trees_with_standalone_summary_and_footer_dock() {
    #[component]
    fn LayoutRenderHarness() -> impl IntoElement {
        use crate::layout_spec::{FooterDock, LayoutNode, LayoutSize, LayoutSpec, ViewModule};

        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("layout-render");
        let profiles = make_profiles();
        let compose = make_compose();
        let seq_builder = make_seq_builder();
        let shown = make_shown("https://api.vicanso.dev/v1/health", None, "");
        shown.phase.set(Phase::Empty);
        let rows = signal(Vec::<Arc<RowLine>>::new());
        let selected = signal(None::<u64>);
        let paused = signal(None::<(u64, BreakSide)>);
        let baseline = signal(None::<u64>);
        let folded = signal(0_u8);
        let facets = signal(session::SessionFacets::default());
        let facets_open = signal(false);
        let columns = signal(ListColumns::from_config(&config::Config::default()));
        let sort = signal(None::<ListSort>);
        let list_filter = signal(String::new());
        let effective_filter = signal(String::new());
        let empty_diagnosis = signal(EmptyFilterDiagnosis::default());
        let filter_focus = element_ref();
        let detail_scroll = element_ref();
        let workspace_ref = element_ref();
        let pane_px = Rc::new(Cell::new(500.0_f32));
        let filtered_stats = signal(session::FilteredStats {
            total_count: 3,
            s2xx: 3,
            total_req_bytes: 120,
            total_res_bytes: 480,
            total_bytes: 600,
            avg_elapsed_ms: Some(42),
            ..Default::default()
        });
        let standalone_summary = signal(false);
        let active_layout = signal(LayoutSpec::builtin_split());

        provide_context(profiles);
        provide_context(StandaloneSummaryContext(standalone_summary));
        provide_context(SessionTabBarProps {
            controls: controls.clone(),
            tabs: signal(session_tab_summaries(&controls, 3)),
            active_tab: signal(LIVE_TAB_ID),
            selected,
            baseline,
            compose,
            seq_builder,
        });
        provide_context(seq_builder);
        provide_context(DiagramResumeRows(rows));

        let onboarding = OnboardingProps {
            controls: controls.clone(),
            running: signal(false),
            listen: signal("127.0.0.1:8877".into()),
            ca_ui: make_ca_signals(false),
            use_system_proxy: signal(false),
            adb_count: signal(0),
            flow_total: signal(3),
            onboarding_traffic: signal(OnboardingTraffic {
                total_flows: 3,
                has_decrypted_https: false,
                has_tls_error: false,
            }),
            compose,
            selected,
            profiles,
            install_ca: Rc::new(|| {}),
            toggle_system_proxy: Rc::new(|| {}),
            qr_open: signal(false),
            pending_qr: signal(false),
        };

        let workspace_host = WorkspaceLayoutHost {
            workspace_ref,
            profiles,
            list_width: profiles.list_width,
            controls: controls.clone(),
            pane_overrides: signal(std::collections::BTreeMap::new()),
            on_save_pane_size: Rc::new(|_, _| {}),
            modules: WorkspaceModules {
                sidebar: Rc::new(move || {
                    facet_sidebar(
                        facets,
                        list_filter,
                        profiles.sidebar_width,
                        profiles.sidebar_collapsed,
                    )
                    .into_element()
                }),
                flow_list: Rc::new({
                    let c = controls.clone();
                    move || {
                        flow_list(
                            rows,
                            facets,
                            facets_open,
                            selected,
                            paused,
                            compose,
                            EmptyFilterState {
                                effective_filter,
                                main_filter: profiles.filter,
                                filter_enabled: profiles.filter_enabled,
                                list_filter,
                                diagnosis: empty_diagnosis,
                                running: signal(false),
                                on_clear_primary: None,
                            },
                            profiles,
                            columns,
                            sort,
                            c.clone(),
                            shown,
                            baseline,
                            filter_focus,
                            Rc::new(|| {}),
                        )
                        .into_element()
                    }
                }),
                detail: Rc::new({
                    let c = controls.clone();
                    let onb = onboarding.clone();
                    move || {
                        detail_pane(
                            shown,
                            c.clone(),
                            selected,
                            paused,
                            compose,
                            seq_builder,
                            rows,
                            list_filter,
                            effective_filter,
                            baseline,
                            folded,
                            Rc::clone(&pane_px),
                            filtered_stats,
                            detail_scroll,
                            profiles.layout_mode,
                            onb.clone(),
                        )
                        .into_element()
                    }
                }),
                summary: Rc::new({
                    let c = controls.clone();
                    move || {
                        view()
                            .flex_1()
                            .w_full()
                            .h_full()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .child(text("STANDALONE_SUMMARY_SLOT"))
                            .child(summary_pane(
                                filtered_stats,
                                selected,
                                c.clone(),
                                rows,
                                seq_builder,
                                compose,
                                shown.phase,
                            ))
                            .into_element()
                    }
                }),
            },
        };

        let set_spec = move |spec: LayoutSpec| {
            standalone_summary.set(spec.has_standalone_summary());
            active_layout.set(spec);
        };

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                view()
                    .flex_row()
                    .child(
                        button()
                            .on_press(move |_: &()| {
                                set_spec(LayoutSpec {
                                    name: "custom-top".into(),
                                    footer: FooterDock::Top,
                                    root: LayoutNode::row(vec![
                                        LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
                                        LayoutNode::col(vec![
                                            LayoutNode::pane(
                                                ViewModule::FlowList,
                                                LayoutSize::Fr(1.0),
                                            ),
                                            LayoutNode::pane(
                                                ViewModule::Summary,
                                                LayoutSize::Px(140.0),
                                            ),
                                        ]),
                                        LayoutNode::pane(
                                            ViewModule::Sidebar,
                                            LayoutSize::Px(220.0),
                                        ),
                                    ]),
                                });
                            })
                            .child(text("TO_CUSTOM_TOP")),
                    )
                    .child(
                        button()
                            .on_press(move |_: &()| {
                                set_spec(LayoutSpec {
                                    name: "list-only".into(),
                                    footer: FooterDock::Hidden,
                                    root: LayoutNode::row(vec![LayoutNode::pane(
                                        ViewModule::FlowList,
                                        LayoutSize::Fr(1.0),
                                    )]),
                                });
                            })
                            .child(text("TO_LIST_ONLY")),
                    )
                    .child(
                        button()
                            .on_press(move |_: &()| {
                                profiles.layout_mode.set(config::LayoutMode::TopBottom);
                                set_spec(LayoutSpec::builtin_top_bottom());
                            })
                            .child(text("TO_TOP_BOTTOM")),
                    )
                    .child(
                        button()
                            .on_press(move |_: &()| {
                                profiles.layout_mode.set(config::LayoutMode::Split);
                                set_spec(LayoutSpec::builtin_split());
                            })
                            .child(text("TO_SPLIT")),
                    ),
            )
            .child(when(
                move || active_layout.with(|s| s.footer) == FooterDock::Top,
                move || view().child(text("FOOTER_TOP_SLOT")),
            ))
            .child(render_workspace_layout(active_layout, workspace_host))
            .child(when(
                move || active_layout.with(|s| s.footer) == FooterDock::Bottom,
                move || view().child(text("FOOTER_BOTTOM_SLOT")),
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (apps_title, summary_title, empty_detail_title, clear_label) = match lang {
            Lang::Zh => ("应用 / 来源", "当前列表统计", "还没有选择", "清空会话"),
            Lang::En => (
                "Apps",
                "Filtered summary",
                "Nothing selected",
                "Clear session",
            ),
        };
        let mut core = Core::new(ApproxTextEngine, Size::new(px(1200.0), px(760.0)));
        let mut registry = Registry::new();
        registry.register::<LayoutRenderHarness>();
        core.set_components(registry);
        core.set_environment(nsh_core::element::Environment {
            window: Size::new(px(1200.0), px(760.0)),
            os: nsh_core::platform::Os::MacOs,
            idiom: nsh_core::platform::Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        });
        let comp_name = std::any::type_name::<LayoutRenderHarness>();
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(comp_name.into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply harness root");
        }
        let mut tree = HeadlessTree::default();
        tree.pump(&mut core);

        let count_summary = |tree: &HeadlessTree| -> usize {
            tree.all_live_texts()
                .iter()
                .filter(|t| t.as_str() == summary_title)
                .count()
        };
        let has_flow_list = |tree: &HeadlessTree| -> bool {
            tree.labels
                .values()
                .any(|label| label.as_deref() == Some(clear_label))
        };

        // 1. Initial built-in Split: sidebar, flow_list, detail, inline summary, bottom footer.
        assert!(
            tree.has_text(apps_title),
            "step1 missing apps_title={apps_title}; live_texts={:?}",
            tree.all_live_texts()
        );
        assert!(has_flow_list(&tree));
        assert!(tree.has_text(empty_detail_title));
        assert!(!tree.has_text("STANDALONE_SUMMARY_SLOT"));
        assert_eq!(count_summary(&tree), 1);
        assert!(tree.has_text("FOOTER_BOTTOM_SLOT"));
        assert!(!tree.has_text("FOOTER_TOP_SLOT"));

        // 2. Custom nested tree with standalone Summary + FooterDock::Top (no duplicate summary).
        assert!(tree.press_text(&mut core, "TO_CUSTOM_TOP"));
        assert!(
            tree.has_text(apps_title),
            "step2 missing apps_title={apps_title}; live_texts={:?}",
            tree.all_live_texts()
        );
        assert!(has_flow_list(&tree));
        assert!(
            tree.has_text(empty_detail_title),
            "step2 missing detail; live_texts={:?}",
            tree.all_live_texts()
        );
        assert!(tree.has_text("STANDALONE_SUMMARY_SLOT"));
        assert_eq!(count_summary(&tree), 1);
        assert!(tree.has_text("FOOTER_TOP_SLOT"));
        assert!(!tree.has_text("FOOTER_BOTTOM_SLOT"));

        // 3. List-only tree + FooterDock::Hidden unmounts sidebar, detail, summary, and footer.
        assert!(tree.press_text(&mut core, "TO_LIST_ONLY"));
        assert!(has_flow_list(&tree));
        assert!(!tree.has_text(apps_title));
        assert!(!tree.has_text(empty_detail_title));
        assert!(!tree.has_text("STANDALONE_SUMMARY_SLOT"));
        assert_eq!(count_summary(&tree), 0);
        assert!(!tree.has_text("FOOTER_TOP_SLOT"));
        assert!(!tree.has_text("FOOTER_BOTTOM_SLOT"));

        // 4. Built-in Top/Bottom and back to Split restore cleanly.
        assert!(tree.press_text(&mut core, "TO_TOP_BOTTOM"));
        assert!(has_flow_list(&tree));
        assert!(tree.has_text(empty_detail_title));
        assert!(!tree.has_text(apps_title));
        assert_eq!(count_summary(&tree), 1);
        assert!(tree.has_text("FOOTER_BOTTOM_SLOT"));

        assert!(tree.press_text(&mut core, "TO_SPLIT"));
        assert!(
            tree.has_text(apps_title),
            "step5 missing apps_title={apps_title}; live_texts={:?}",
            tree.all_live_texts()
        );
        assert!(has_flow_list(&tree));
        assert!(tree.has_text(empty_detail_title));
        assert_eq!(count_summary(&tree), 1);
        assert!(tree.has_text("FOOTER_BOTTOM_SLOT"));
    }
}

#[test]
fn custom_tsx_layout_switch_editor_save_and_error_fallback_zh_en() {
    struct TempLayoutDirGuard(std::path::PathBuf);
    impl Drop for TempLayoutDirGuard {
        fn drop(&mut self) {
            set_test_layout_data_dir(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let dir = std::env::temp_dir().join(format!(
            "zniff-v05-tsx-layout-{}-{}",
            std::process::id(),
            TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let layouts_dir = dir.join("layouts");
        std::fs::create_dir_all(&layouts_dir).unwrap();
        let _guard = TempLayoutDirGuard(dir.clone());

        std::fs::write(
            layouts_dir.join("focus.tsx"),
            r#"import { Layout, Row, Col, Pane } from "zniff/layout";
export default () => (
  <Layout name="Focus QA" footer="top">
    <Row>
      <Pane name="sidebar" size={220} />
      <Col>
        <Pane name="flow_list" size="1fr" />
        <Pane name="summary" size={140} />
      </Col>
      <Pane name="detail" size="48%" />
    </Row>
  </Layout>
);
"#,
        )
        .unwrap();
        std::fs::write(
            layouts_dir.join("broken.tsx"),
            "export default () => (<Layout><Row></Layout>);",
        )
        .unwrap();
        set_test_layout_data_dir(Some(dir.clone()));

        #[component]
        fn CustomTsxLayoutHarness() -> impl IntoElement {
            const MINIMAL_TSX: &str = r#"import { Layout, Row, Pane } from "zniff/layout";
export default () => (
  <Layout name="Minimal QA" footer="bottom">
    <Row>
      <Pane name="flow_list" size="42%" />
      <Pane name="detail" size="1fr" />
    </Row>
  </Layout>
);
"#;
            let lang = TEST_LANG.with(Cell::get);
            provide_base_contexts(lang);
            let controls = make_controls("tsx-layout");
            let profiles = make_profiles();
            let compose = make_compose();
            let seq_builder = make_seq_builder();
            let shown = make_shown("https://api.vicanso.dev/v1/items", None, "");
            shown.phase.set(Phase::Empty);
            let rows = signal(Vec::<Arc<RowLine>>::new());
            let selected = signal(None::<u64>);
            let paused = signal(None::<(u64, BreakSide)>);
            let baseline = signal(None::<u64>);
            let folded = signal(0_u8);
            let facets = signal(session::SessionFacets::default());
            let facets_open = signal(false);
            let columns = signal(ListColumns::from_config(&config::Config::default()));
            let sort = signal(None::<ListSort>);
            let list_filter = signal(String::new());
            let effective_filter = signal(String::new());
            let empty_diagnosis = signal(EmptyFilterDiagnosis::default());
            let filter_focus = element_ref();
            let detail_scroll = element_ref();
            let workspace_ref = element_ref();
            let pane_px = Rc::new(Cell::new(500.0_f32));
            let filtered_stats = signal(session::FilteredStats::default());
            let layout_ui = init_layout_ui_state(
                "",
                &std::collections::BTreeMap::new(),
                profiles.layout_mode.get_untracked(),
            );
            let active_layout = layout_ui.active_spec;
            let standalone_summary = signal(active_layout.get_untracked().has_standalone_summary());
            effect(move || {
                let id = layout_ui.active_id.get();
                let mode = profiles.layout_mode.get();
                let rev = layout_ui.reload_rev.get();
                let _ = (id, mode, rev);
                evaluate_active_layout(layout_ui, mode, current_layout_context());
                standalone_summary
                    .set(active_layout.with_untracked(|s| s.has_standalone_summary()));
            });

            provide_context(profiles);
            provide_context(layout_ui);
            provide_context(StandaloneSummaryContext(standalone_summary));
            provide_context(SessionTabBarProps {
                controls: controls.clone(),
                tabs: signal(session_tab_summaries(&controls, 1)),
                active_tab: signal(LIVE_TAB_ID),
                selected,
                baseline,
                compose,
                seq_builder,
            });
            provide_context(seq_builder);
            provide_context(DiagramResumeRows(rows));

            let onboarding = OnboardingProps {
                controls: controls.clone(),
                running: signal(false),
                listen: signal("127.0.0.1:8877".into()),
                ca_ui: make_ca_signals(false),
                use_system_proxy: signal(false),
                adb_count: signal(0),
                flow_total: signal(0),
                onboarding_traffic: signal(OnboardingTraffic::default()),
                compose,
                selected,
                profiles,
                install_ca: Rc::new(|| {}),
                toggle_system_proxy: Rc::new(|| {}),
                qr_open: signal(false),
                pending_qr: signal(false),
            };

            let controls_pane = controls.clone();
            let workspace_host = WorkspaceLayoutHost {
                workspace_ref,
                profiles,
                list_width: profiles.list_width,
                controls: controls.clone(),
                pane_overrides: layout_ui.active_pane_overrides,
                on_save_pane_size: Rc::new(move |path, px_val| {
                    save_pane_size_override(layout_ui, path, px_val, &controls_pane);
                }),
                modules: WorkspaceModules {
                    sidebar: Rc::new(move || {
                        facet_sidebar(
                            facets,
                            list_filter,
                            profiles.sidebar_width,
                            profiles.sidebar_collapsed,
                        )
                        .into_element()
                    }),
                    flow_list: Rc::new({
                        let c = controls.clone();
                        move || {
                            flow_list(
                                rows,
                                facets,
                                facets_open,
                                selected,
                                paused,
                                compose,
                                EmptyFilterState {
                                    effective_filter,
                                    main_filter: profiles.filter,
                                    filter_enabled: profiles.filter_enabled,
                                    list_filter,
                                    diagnosis: empty_diagnosis,
                                    running: signal(false),
                                    on_clear_primary: None,
                                },
                                profiles,
                                columns,
                                sort,
                                c.clone(),
                                shown,
                                baseline,
                                filter_focus,
                                Rc::new(|| {}),
                            )
                            .into_element()
                        }
                    }),
                    detail: Rc::new({
                        let c = controls.clone();
                        let onb = onboarding.clone();
                        move || {
                            detail_pane(
                                shown,
                                c.clone(),
                                selected,
                                paused,
                                compose,
                                seq_builder,
                                rows,
                                list_filter,
                                effective_filter,
                                baseline,
                                folded,
                                Rc::clone(&pane_px),
                                filtered_stats,
                                detail_scroll,
                                profiles.layout_mode,
                                onb.clone(),
                            )
                            .into_element()
                        }
                    }),
                    summary: Rc::new({
                        let c = controls.clone();
                        move || {
                            view()
                                .flex_1()
                                .w_full()
                                .h_full()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(text("STANDALONE_SUMMARY_SLOT"))
                                .child(summary_pane(
                                    filtered_stats,
                                    selected,
                                    c.clone(),
                                    rows,
                                    seq_builder,
                                    compose,
                                    shown.phase,
                                ))
                                .into_element()
                        }
                    }),
                },
            };

            let draft_controls = controls.clone();
            view()
                .w(px(1200.0))
                .h(px(760.0))
                .flex_col()
                .child(
                    view()
                        .flex_row()
                        .gap_1()
                        .child(toolbar_sidebar_toggle(
                            profiles.layout_mode,
                            profiles.sidebar_collapsed,
                            draft_controls.clone(),
                        ))
                        .child(toolbar_layout_switcher(
                            profiles.layout_mode,
                            draft_controls.clone(),
                        ))
                        .child(
                            button()
                                .on_press(move |_: &()| {
                                    layout_ui.draft_source.set(MINIMAL_TSX.to_string());
                                    validate_draft_layout(layout_ui, current_layout_context());
                                })
                                .child(text("FILL_MINIMAL_TSX")),
                        ),
                )
                .child(layout_error_banner(
                    layout_ui,
                    profiles,
                    draft_controls.clone(),
                ))
                .child(when(
                    move || active_layout.with(|s| s.footer) == crate::layout_spec::FooterDock::Top,
                    move || view().child(text("FOOTER_TOP_SLOT")),
                ))
                .child(render_workspace_layout(active_layout, workspace_host))
                .child(when(
                    move || {
                        active_layout.with(|s| s.footer) == crate::layout_spec::FooterDock::Bottom
                    },
                    move || view().child(text("FOOTER_BOTTOM_SLOT")),
                ))
                .child(settings_custom_layouts_section(
                    layout_ui,
                    profiles.layout_mode,
                    draft_controls,
                ))
        }

        let (
            switcher_lbl,
            collapse_lbl,
            expand_lbl,
            custom_menu_lbl,
            id_lbl,
            new_lbl,
            validate_lbl,
            save_lbl,
            del_lbl,
            confirm_del_lbl,
            fallback_lbl,
            err_prefix,
        ) = match lang {
            Lang::Zh => (
                "分栏布局",
                "收起域名与应用栏",
                "展开域名与应用栏",
                "自定义布局…",
                "布局 ID",
                "新建",
                "校验",
                "保存并应用",
                "删除",
                "再次点击确认删除",
                "切回默认",
                "自定义布局出错",
            ),
            Lang::En => (
                "Split layout",
                "Collapse Domains & Apps sidebar",
                "Expand Domains & Apps sidebar",
                "Custom layouts…",
                "Layout ID",
                "New",
                "Validate",
                "Save & Apply",
                "Delete",
                "Confirm delete",
                "Use built-in",
                "Custom layout error",
            ),
        };

        let mut core = Core::new(ApproxTextEngine, Size::new(px(1200.0), px(760.0)));
        let mut registry = Registry::new();
        registry.register::<CustomTsxLayoutHarness>();
        core.set_components(registry);
        core.set_environment(nsh_core::element::Environment {
            window: Size::new(px(1200.0), px(760.0)),
            os: nsh_core::platform::Os::MacOs,
            idiom: nsh_core::platform::Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        });
        let comp_name = std::any::type_name::<CustomTsxLayoutHarness>();
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(comp_name.into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply harness");
        }
        let mut tree = HeadlessTree::default();
        tree.pump(&mut core);

        let has_sidebar_toggle = |tree: &HeadlessTree| -> bool {
            tree.labels.values().any(|l| {
                matches!(
                    l.as_deref(),
                    Some(lbl) if lbl == collapse_lbl || lbl == expand_lbl
                )
            })
        };

        // Initial built-in split shows sidebar toggle and bottom footer.
        assert!(has_sidebar_toggle(&tree));
        assert!(tree.has_text("FOOTER_BOTTOM_SLOT"));
        assert!(!tree.has_text("STANDALONE_SUMMARY_SLOT"));

        // 1. Open toolbar layout popover and switch to pre-populated `focus.tsx` (`Focus QA`).
        assert!(tree.press_label(&mut core, switcher_lbl));
        assert!(tree.has_text(custom_menu_lbl));
        assert!(tree.press_text(&mut core, "Focus QA"));
        assert!(has_sidebar_toggle(&tree));
        assert!(tree.has_text("STANDALONE_SUMMARY_SLOT"));
        assert!(tree.has_text("FOOTER_TOP_SLOT"));
        assert!(!tree.has_text("FOOTER_BOTTOM_SLOT"));

        // 2. Open editor via Settings "New", type `minimal` ID, load `minimal.tsx` without sidebar, validate + save.
        assert!(tree.press_text(&mut core, new_lbl));
        let input_id = tree
            .live_node_ids()
            .into_iter()
            .find(|id| {
                tree.kinds.get(id) == Some(&ViewKind::Input)
                    && tree.labels.get(id).and_then(|l| l.as_deref()) == Some(id_lbl)
            })
            .expect("layout id input mounted");
        core.component_event(
            input_id,
            EventKind::ChangeText,
            &EventPayload::Text {
                text: "minimal".into(),
                count: 1,
            },
        );
        tree.pump(&mut core);
        assert!(tree.press_text(&mut core, "FILL_MINIMAL_TSX"));
        assert!(tree.press_text(&mut core, validate_lbl));
        assert!(tree.has_text_containing("Minimal QA"));
        assert!(tree.press_text(&mut core, save_lbl));

        assert!(layouts_dir.join("minimal.tsx").exists());
        assert!(layouts_dir.join("zniff-layout.d.ts").exists());
        assert!(!has_sidebar_toggle(&tree), "minimal layout has no sidebar");
        assert!(tree.has_text("FOOTER_BOTTOM_SLOT"));

        // 3. Switch to `broken.tsx` via toolbar popover -> falls back to built-in Split + banner -> Use built-in clears it.
        assert!(tree.press_label(&mut core, switcher_lbl));
        assert!(tree.press_text(&mut core, "broken"));
        assert!(
            tree.has_text_containing(err_prefix),
            "expected error banner `{err_prefix}`; texts={:?}",
            tree.all_live_texts()
        );
        assert!(has_sidebar_toggle(&tree));
        assert!(tree.press_text(&mut core, fallback_lbl));
        assert!(!tree.has_text_containing(err_prefix));

        // 4. Two-click 3.5s delete confirmation for `minimal`: first click arms without deleting, second deletes.
        assert!(tree.has_text("Minimal QA"));
        assert!(tree.press_text(&mut core, del_lbl));
        assert!(tree.has_text(confirm_del_lbl));
        assert!(layouts_dir.join("minimal.tsx").exists());
        assert!(tree.press_text(&mut core, confirm_del_lbl));
        assert!(!layouts_dir.join("minimal.tsx").exists());
    }
}

#[test]
fn tray_close_reopen_restores_real_flow_table_and_dirty_filter_state() {
    use std::cell::RefCell;

    type IdTriplet = Rc<Cell<(u64, u64, u64)>>;

    thread_local! {
        static TEST_RT: RefCell<Option<AppRuntime>> = const { RefCell::new(None) };
        static TEST_IDS: RefCell<Option<IdTriplet>> = const { RefCell::new(None) };
    }

    fn record_flow_for(controls: &Controls, host: &str, path: &str) -> u64 {
        let id = controls.hub.with_session(|session| {
            let id = session.open(crate::session::FlowDraft {
                protocol: crate::session::Protocol::Http2,
                method: "GET".into(),
                host: host.into(),
                path: path.into(),
                url: format!("https://{host}{path}"),
                request_headers: Vec::new(),
            });
            session.set_response(
                id,
                200,
                vec![("content-type".into(), "application/json".into())],
                None,
            );
            session.mark_finished(id);
            id
        });
        controls.hub.touch();
        id
    }

    fn workspace_with_real_flow_list(rt: AppRuntime) -> impl IntoElement {
        let controls = rt.controls.clone();
        let langs = rt.langs;
        let lang = ui_lang(langs);
        let saved = saved_config();
        let filter = rt.filter;
        let filter_enabled = rt.filter_enabled;
        let list_filter = rt.list_filter;
        let initial_filter = filter.get_untracked();
        let initial_enabled = filter_enabled.get_untracked();
        let initial_list = list_filter.get_untracked();
        let initial_effective = session::combine_filters(
            if initial_enabled { &initial_filter } else { "" },
            &initial_list,
        );
        let effective_filter = signal(initial_effective);
        effect(move || {
            let primary = if filter_enabled.get() {
                filter.get()
            } else {
                String::new()
            };
            effective_filter.set(session::combine_filters(&primary, &list_filter.get()));
        });
        let selected = rt.selected;
        let baseline = rt.baseline;
        let sort = rt.sort;
        let columns = signal(ListColumns::from_config(&saved));
        let mut cache = RowCache::new(lang);
        let snap =
            compute_workspace_initial_snapshot(&rt, columns.get_untracked(), lang, &mut cache);

        let rows = signal(snap.rows);
        let filtered_stats = signal(snap.filtered_stats);
        let empty_diagnosis = signal(snap.empty_diagnosis);
        let facets = signal(snap.facets);
        let filter_catalog = signal(snap.filter_catalog);
        let facets_open = signal(false);
        let paused = signal(snap.paused);
        let session_tabs_sig = signal(snap.session_tabs);
        let active_tab_sig = signal(snap.active_tab);
        let shown = make_shown("", None, "");
        shown.phase.set(Phase::Empty);
        if snap.selected.is_some() {
            publish_detail_for_controls(&shown, &controls, snap.selected, lang, snap.baseline);
        } else {
            clear_shown(shown);
        }

        let mut profiles = make_profiles();
        profiles.filter = filter;
        profiles.filter_enabled = filter_enabled;
        profiles.show_tray = rt.show_tray;
        profiles.minimize_to_tray = rt.minimize_to_tray;
        profiles.list_width = signal(660.0);
        let compose = make_compose();
        let seq_builder = make_seq_builder();

        let _ = provide_popover_dismiss();
        provide_context(filter_catalog);
        provide_context(profiles);
        provide_context(SessionTabBarProps {
            tabs: session_tabs_sig,
            active_tab: active_tab_sig,
            controls: controls.clone(),
            selected,
            baseline,
            compose,
            seq_builder,
        });

        let refresh_controls = controls.clone();
        let refresh_rt = rt;
        let refresh_lang = lang;

        view()
            .flex_1()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        let mut c = RowCache::new(refresh_lang);
                        let next = compute_workspace_initial_snapshot(
                            &refresh_rt,
                            columns.get_untracked(),
                            refresh_lang,
                            &mut c,
                        );
                        rows.set(next.rows);
                        filtered_stats.set(next.filtered_stats);
                        facets.set(next.facets);
                        session_tabs_sig.set(next.session_tabs);
                        active_tab_sig.set(next.active_tab);
                        if next.selected.is_some() {
                            publish_detail_for_controls(
                                &shown,
                                &refresh_controls,
                                next.selected,
                                refresh_lang,
                                next.baseline,
                            );
                        }
                    })
                    .child(text("SIMULATE_SAME_SESSION_TICK")),
            )
            .child(text(move || {
                format!(
                    "DIRTY_STATE|filter={}|enabled={}|list={}|selected={:?}|sort={:?}|tab={}|total={}|shown_url={}",
                    filter.get(),
                    filter_enabled.get(),
                    list_filter.get(),
                    selected.get(),
                    sort.get(),
                    active_tab_sig.get(),
                    filtered_stats.get().total_count,
                    shown.url.get(),
                )
            }))
            .child(flow_list(
                rows,
                facets,
                facets_open,
                selected,
                paused,
                compose,
                EmptyFilterState {
                    effective_filter,
                    main_filter: filter,
                    filter_enabled,
                    list_filter,
                    diagnosis: empty_diagnosis,
                    running: rt_running_sig(false),
                    on_clear_primary: None,
                },
                profiles,
                columns,
                sort,
                controls,
                shown,
                baseline,
                element_ref(),
                Rc::new(|| {}),
            ))
    }

    fn rt_running_sig(v: bool) -> Signal<bool> {
        signal(v)
    }

    #[component]
    fn RealFlowListTrayHarness() -> impl IntoElement {
        let lang = TEST_LANG.with(Cell::get);
        provide_base_contexts(lang);
        let controls = make_controls("tray-real-flow-list");
        save_config(|cfg| {
            cfg.filter = "host:api.vicanso.dev".into();
            cfg.show_tray = true;
            cfg.minimize_to_tray = true;
        })
        .expect("save initial test config");

        let id1 = record_flow_for(&controls, "api.vicanso.dev", "/v1/before-api");
        let id2 = record_flow_for(&controls, "other.vicanso.dev", "/v1/before-other");
        let ids = Rc::new(Cell::new((id1, id2, 0_u64)));
        TEST_IDS.with(|slot| *slot.borrow_mut() = Some(ids.clone()));

        let now = clock::now();
        let ui_cfg = saved_config();
        let rt = AppRuntime {
            controls,
            langs: signal(Some(lang)),
            show_tray: signal(ui_cfg.show_tray),
            minimize_to_tray: signal(ui_cfg.minimize_to_tray),
            main_open: signal(true),
            filter: signal(ui_cfg.filter),
            filter_enabled: signal(true),
            list_filter: signal(String::new()),
            selected: signal(None),
            baseline: signal(None),
            sort: signal(None),
            main_win: signal(None),
            last_placement: stored(None),
            has_mounted_once: stored(false),
            running: signal(true),
            listen: signal("127.0.0.1:8877".into()),
            use_system_proxy: signal(false),
            about_open: signal(false),
            settings_open: signal(false),
            watch_rules: signal(Vec::new()),
            watch_eval: signal(crate::watch::WatchEvaluation::default()),
            watch_tracker: stored(crate::watch::WatchTracker::default()),
            seen_watch_rules: stored(Vec::new()),
            seen_watch_gen: stored(u64::MAX),
            spill_gc_at: stored(now),
            update_probe_at: stored(now),
            tray_clear_rev: signal(0),
            cached_active_layout: stored(None),
        };
        TEST_RT.with(|slot| *slot.borrow_mut() = Some(rt.clone()));

        let rt_change = rt.clone();
        let rt_close = rt.clone();
        let rt_content = rt.clone();
        let rt_dirty = rt.clone();
        let rt_show = rt.clone();
        let rt_clear = rt.clone();
        let rt_evict = rt.clone();
        let rt_import = rt.clone();
        let rt_pending_filter = rt.clone();
        let root_rt = rt;

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .child(
                nsh_core::element::desktop::window()
                    .id("main")
                    .open(root_rt.main_open)
                    .on_open_change(move |ev| on_main_window_open_change(&rt_change, ev.open))
                    .on_close_press(move |_| close_main_window_to_tray(&rt_close))
                    .content(move || {
                        rt_content
                            .main_win
                            .set(Some(nsh_core::element::desktop::current_window()));
                        workspace_with_real_flow_list(rt_content.clone())
                    }),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        rt_dirty.filter_enabled.set(false);
                        rt_dirty.list_filter.set("vicanso".into());
                        rt_dirty.selected.set(Some(id2));
                        rt_dirty.sort.set(Some(ListSort {
                            column: ListColumn::Id,
                            desc: true,
                        }));
                    })
                    .child(text("APPLY_DIRTY_STATE")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        rt_pending_filter.filter.set("host:sub.vicanso.dev".into());
                        close_main_window_to_tray(&rt_pending_filter);
                    })
                    .child(text("CLOSE_WITH_PENDING_FILTER")),
            )
            .child(
                button()
                    .on_press(move |_: &()| reveal_main_window(rt_show.main_win, rt_show.main_open))
                    .child(text("TRAY_SHOW")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        if clear_captured_session(&rt_clear.controls) {
                            clear_runtime_selection(&rt_clear);
                            rt_clear
                                .tray_clear_rev
                                .set(rt_clear.tray_clear_rev.get_untracked().wrapping_add(1));
                        }
                    })
                    .child(text("TRAY_CLEAR")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        let (mut sess, _) = build_imported_session(Vec::new());
                        let imp_id = sess.open(crate::session::FlowDraft {
                            protocol: crate::session::Protocol::Http11,
                            method: "POST".into(),
                            host: "import.vicanso.dev".into(),
                            path: "/v1/imported-item".into(),
                            url: "https://import.vicanso.dev/v1/imported-item".into(),
                            request_headers: Vec::new(),
                        });
                        sess.set_response(imp_id, 201, Vec::new(), None);
                        sess.mark_finished(imp_id);
                        rt_import
                            .controls
                            .session_tabs
                            .lock()
                            .unwrap()
                            .open_session("vicanso-import.har".into(), sess)
                            .expect("open imported tab");
                        rt_import.selected.set(Some(imp_id));
                        rt_import.filter_enabled.set(false);
                        rt_import.list_filter.set(String::new());
                    })
                    .child(text("OPEN_IMPORTED_AND_SELECT")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        rt_evict.selected.set(Some(999_999));
                    })
                    .child(text("SET_EVICTED_SELECTION")),
            )
            .child(text(move || {
                format!(
                    "ROOT_VIS|open={}|selected={:?}",
                    root_rt.main_open.get(),
                    root_rt.selected.get(),
                )
            }))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let mut core = Core::new(ApproxTextEngine, Size::new(px(1200.0), px(760.0)));
        let mut registry = Registry::new();
        registry.register::<RealFlowListTrayHarness>();
        core.set_components(registry);
        core.set_environment(nsh_core::element::Environment {
            window: Size::new(px(1200.0), px(760.0)),
            os: nsh_core::platform::Os::MacOs,
            idiom: nsh_core::platform::Idiom::Desktop,
            reduce_motion: true,
            ..Default::default()
        });
        let comp_name = std::any::type_name::<RealFlowListTrayHarness>();
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(comp_name.into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply root op");
        }
        let mut tree = HeadlessTree::default();
        tree.pump(&mut core);

        let rt = TEST_RT.with(|slot| slot.borrow().clone()).expect("rt");
        let ids_cell = TEST_IDS.with(|slot| slot.borrow().clone()).expect("ids");
        let (_id1, id2, _) = ids_cell.get();

        // 1. Frame 1 immediately renders matching primary-filtered flow in real DataTable.
        assert!(tree.has_text_containing("/v1/before-api"));
        assert!(!tree.has_text_containing("/v1/before-other"));

        // 2. Apply dirty state: pause primary filter, set secondary list_filter, select id2, set sort,
        //    then run one open-window refresh tick (simulating open 16ms sync prior to closing).
        assert!(tree.press_text(&mut core, "APPLY_DIRTY_STATE"));
        assert!(tree.press_text(&mut core, "SIMULATE_SAME_SESSION_TICK"));
        assert!(tree.has_text_containing("/v1/before-api"));
        assert!(tree.has_text_containing("/v1/before-other"));
        assert!(tree.has_text_containing(&format!(
            "DIRTY_STATE|filter=host:api.vicanso.dev|enabled=false|list=vicanso|selected=Some({id2})"
        )));

        // 3. Close main window via red X -> unmounts DataTable, keeps live session in memory.
        let modal_id = tree
            .kinds
            .iter()
            .find_map(|(id, kind)| (*kind == ViewKind::Modal).then_some(*id))
            .expect("main window modal");
        core.component_event(
            modal_id,
            EventKind::Window,
            &EventPayload::Window(Box::new(nsh_core::desktop::WindowEvent::ClosePress {
                reason: nsh_core::desktop::CloseReason::CloseButton,
            })),
        );
        tree.pump(&mut core);
        assert!(tree.has_text_containing("ROOT_VIS|open=false"));
        assert!(!tree.has_text_containing("/v1/before-api"));
        assert!(!tree.has_text_containing("/v1/before-other"));

        // 4. Record another flow on other.vicanso.dev while closed, then reopen ->
        //    must immediately restore paused primary filter, list_filter, sort, selection, detail URL,
        //    AND show both pre-close and while-closed flows on frame 1 without 16ms timer wait!
        let _id3 = record_flow_for(&rt.controls, "other.vicanso.dev", "/v1/while-closed-other");
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.has_text_containing("ROOT_VIS|open=true"));
        assert!(tree.has_text_containing("/v1/before-api"));
        assert!(tree.has_text_containing("/v1/before-other"));
        assert!(tree.has_text_containing("/v1/while-closed-other"));
        assert!(tree.has_text_containing(&format!(
            "DIRTY_STATE|filter=host:api.vicanso.dev|enabled=false|list=vicanso|selected=Some({id2})|sort=Some(ListSort {{ column: Id, desc: true }})|tab=0|total=3|shown_url=https://other.vicanso.dev/v1/before-other"
        )));

        // 5. Same-session incremental refresh (A -> A') preserves selection & filter state.
        assert!(tree.press_text(&mut core, "SIMULATE_SAME_SESSION_TICK"));
        assert!(tree.has_text_containing(&format!(
            "DIRTY_STATE|filter=host:api.vicanso.dev|enabled=false|list=vicanso|selected=Some({id2})"
        )));

        // 6. Close with pending primary filter edit flushes `remember_filter` to saved config.
        assert!(tree.press_text(&mut core, "CLOSE_WITH_PENDING_FILTER"));
        assert_eq!(saved_config().filter, "host:sub.vicanso.dev");

        // 7. Reopen, switch to imported tab + select imported flow, close & reopen ->
        //    imported tab, row, and selection restore immediately on frame 1.
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.press_text(&mut core, "OPEN_IMPORTED_AND_SELECT"));
        let modal_id2 = tree
            .kinds
            .iter()
            .find_map(|(id, kind)| (*kind == ViewKind::Modal).then_some(*id))
            .expect("reopened main window modal");
        core.component_event(
            modal_id2,
            EventKind::Window,
            &EventPayload::Window(Box::new(nsh_core::desktop::WindowEvent::ClosePress {
                reason: nsh_core::desktop::CloseReason::CloseButton,
            })),
        );
        tree.pump(&mut core);
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.has_text_containing("/v1/imported-item"));
        assert!(tree.has_text_containing("shown_url=https://import.vicanso.dev/v1/imported-item"));

        // 8. Switch back to live tab via tray clear / evicted selection -> normalized cleanly on reopen.
        rt.controls
            .session_tabs
            .lock()
            .unwrap()
            .switch_to(LIVE_TAB_ID);
        assert!(tree.press_text(&mut core, "TRAY_CLEAR"));
        assert!(tree.press_text(&mut core, "SET_EVICTED_SELECTION"));
        let modal_id3 = tree
            .kinds
            .iter()
            .find_map(|(id, kind)| (*kind == ViewKind::Modal).then_some(*id))
            .expect("reopened main window modal 3");
        core.component_event(
            modal_id3,
            EventKind::Window,
            &EventPayload::Window(Box::new(nsh_core::desktop::WindowEvent::ClosePress {
                reason: nsh_core::desktop::CloseReason::CloseButton,
            })),
        );
        tree.pump(&mut core);
        let _id4 = record_flow_for(&rt.controls, "sub.vicanso.dev", "/v1/fresh-after-clear");
        assert!(tree.press_text(&mut core, "TRAY_SHOW"));
        assert!(tree.has_text_containing("/v1/fresh-after-clear"));
        assert!(tree.has_text_containing("selected=None"));

        TEST_RT.with(|slot| *slot.borrow_mut() = None);
        TEST_IDS.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn built_in_layout_skips_disk_scan_until_switcher_opened() {
    struct TempDirGuard(std::path::PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            set_test_layout_data_dir(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let dir = std::env::temp_dir().join(format!(
        "zniff-v05-lazy-layouts-{}-{}",
        std::process::id(),
        TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let layouts_dir = dir.join("layouts");
    std::fs::create_dir_all(&layouts_dir).unwrap();
    std::fs::write(
        layouts_dir.join("lazy_focus.tsx"),
        r#"import { Layout, Row, Pane } from "zniff/layout";
export default () => (
  <Layout name="Lazy Focus" footer="bottom">
    <Row>
      <Pane name="flow_list" size="45%" />
      <Pane name="detail" size="1fr" />
    </Row>
  </Layout>
);
"#,
    )
    .unwrap();
    let _guard = TempDirGuard(dir.clone());
    set_test_layout_data_dir(Some(dir));

    #[component]
    fn LazySwitcherHarness() -> impl IntoElement {
        provide_base_contexts(Lang::En);
        let controls = make_controls("lazy-switcher");
        let profiles = make_profiles();
        let layout_ui = init_layout_ui_state(
            "",
            &std::collections::BTreeMap::new(),
            profiles.layout_mode.get_untracked(),
        );
        provide_context(profiles);
        provide_context(layout_ui);
        let _ = provide_popover_dismiss();
        view()
            .w(px(1200.0))
            .h(px(760.0))
            .child(toolbar_layout_switcher(profiles.layout_mode, controls))
            .child(text(move || {
                format!(
                    "LAZY_STATE|loaded={}|count={}",
                    layout_ui.layouts_loaded.get(),
                    layout_ui.custom_layouts.with(Vec::len),
                )
            }))
    }

    let (mut core, mut tree) = mount::<LazySwitcherHarness>(Size::new(px(1200.0), px(760.0)));

    // Built-in boot must NOT touch `zniff-layout.d.ts` or scan `layouts/`.
    assert!(!layouts_dir.join("zniff-layout.d.ts").exists());
    assert!(tree.has_text("LAZY_STATE|loaded=false|count=0"));

    // Opening the toolbar layout switcher lazily loads custom layouts and generates `.d.ts`.
    assert!(tree.press_label(&mut core, "Split layout"));
    assert!(tree.has_text("LAZY_STATE|loaded=true|count=1"));
    assert!(layouts_dir.join("zniff-layout.d.ts").exists());
    assert!(tree.has_text("Lazy Focus"));
}

#[test]
fn custom_layout_size_only_change_preserves_mounted_flow_list_and_selection() {
    use crate::layout_spec::{FooterDock, LayoutNode, LayoutSize, LayoutSpec, ViewModule};

    thread_local! {
        static MOUNT_COUNT: Cell<u32> = const { Cell::new(0) };
    }
    MOUNT_COUNT.set(0);

    #[component]
    fn SizeOnlyPreserveHarness() -> impl IntoElement {
        provide_base_contexts(Lang::En);
        let controls = make_controls("size-only-preserve");
        let profiles = make_profiles();
        let selected = signal(Some(77_u64));
        let workspace_ref = element_ref();
        provide_context(SidebarLayoutRef(workspace_ref));

        let initial_spec = LayoutSpec {
            name: "Size Test".into(),
            footer: FooterDock::Bottom,
            root: LayoutNode::row(vec![
                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Percent(45.0)),
                LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
            ]),
        };
        let active_spec = signal(initial_spec);
        let host = WorkspaceLayoutHost {
            workspace_ref,
            profiles,
            list_width: profiles.list_width,
            controls,
            pane_overrides: signal(std::collections::BTreeMap::new()),
            on_save_pane_size: Rc::new(|_, _| {}),
            modules: WorkspaceModules {
                sidebar: Rc::new(|| view().child(text("SIDEBAR")).into_element()),
                flow_list: Rc::new(move || {
                    MOUNT_COUNT.set(MOUNT_COUNT.get() + 1);
                    view()
                        .child(text("FLOW_LIST_MOUNTED"))
                        .child(text(move || format!("SEL={:?}", selected.get())))
                        .into_element()
                }),
                detail: Rc::new(|| view().child(text("DETAIL_PANE")).into_element()),
                summary: Rc::new(|| view().child(text("SUMMARY_PANE")).into_element()),
            },
        };

        view()
            .w(px(1200.0))
            .h(px(760.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        // A -> A': same Row topology, only percentage width changes.
                        active_spec.set(LayoutSpec {
                            name: "Size Test".into(),
                            footer: FooterDock::Bottom,
                            root: LayoutNode::row(vec![
                                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Percent(58.0)),
                                LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
                            ]),
                        });
                    })
                    .child(text("RESIZE_PANE_ONLY")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        // A -> B: topology flips from Row to Col.
                        active_spec.set(LayoutSpec {
                            name: "Size Test".into(),
                            footer: FooterDock::Bottom,
                            root: LayoutNode::col(vec![
                                LayoutNode::pane(ViewModule::FlowList, LayoutSize::Percent(50.0)),
                                LayoutNode::pane(ViewModule::Detail, LayoutSize::Fr(1.0)),
                            ]),
                        });
                    })
                    .child(text("FLIP_TOPOLOGY_COL")),
            )
            .child(render_workspace_layout(active_spec, host))
    }

    let (mut core, mut tree) = mount::<SizeOnlyPreserveHarness>(Size::new(px(1200.0), px(760.0)));
    assert_eq!(MOUNT_COUNT.get(), 1);
    assert!(tree.has_text("FLOW_LIST_MOUNTED"));
    assert!(tree.has_text("SEL=Some(77)"));

    // Size-only change must NOT unmount/remount flow_list.
    assert!(tree.press_text(&mut core, "RESIZE_PANE_ONLY"));
    assert_eq!(
        MOUNT_COUNT.get(),
        1,
        "size-only layout change must preserve mounted flow_list without rebuild"
    );
    assert!(tree.has_text("SEL=Some(77)"));

    // Topology change rebuilds cleanly once while keeping shared selection intact.
    assert!(tree.press_text(&mut core, "FLIP_TOPOLOGY_COL"));
    assert_eq!(MOUNT_COUNT.get(), 2);
    assert!(tree.has_text("SEL=Some(77)"));
}

#[test]
fn circuit_breaker_banner_trips_and_recovers_via_retry_button_zh_en() {
    struct TempDirGuard(std::path::PathBuf, String);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            crate::script_worker::reset_script_circuit(&self.1);
            set_test_layout_data_dir(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let script_id = format!(
            "flaky_ui_{}",
            TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        crate::script_worker::reset_script_circuit(&script_id);

        let dir = std::env::temp_dir().join(format!(
            "zniff-v05-breaker-{}-{}",
            std::process::id(),
            TEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let layouts_dir = dir.join("layouts");
        std::fs::create_dir_all(&layouts_dir).unwrap();
        let script_path = layouts_dir.join(format!("{script_id}.tsx"));
        std::fs::write(&script_path, "export default () => { while (true) {} };").unwrap();
        let _guard = TempDirGuard(dir.clone(), script_id.clone());
        set_test_layout_data_dir(Some(dir.clone()));

        thread_local! {
            static BREAKER_ID: RefCell<String> = const { RefCell::new(String::new()) };
        }
        BREAKER_ID.with(|slot| *slot.borrow_mut() = script_id.clone());

        #[component]
        fn BreakerBannerHarness() -> impl IntoElement {
            let lang = TEST_LANG.with(Cell::get);
            provide_base_contexts(lang);
            let controls = make_controls("breaker-banner");
            let profiles = make_profiles();
            let id = BREAKER_ID.with(|slot| slot.borrow().clone());
            let layout_ui = init_layout_ui_state(
                &id,
                &std::collections::BTreeMap::new(),
                profiles.layout_mode.get_untracked(),
            );
            let active_spec = layout_ui.active_spec;
            view()
                .w(px(1200.0))
                .h(px(760.0))
                .flex_col()
                .child(
                    button()
                        .on_press(move |_: &()| {
                            evaluate_active_layout(
                                layout_ui,
                                profiles.layout_mode.get_untracked(),
                                current_layout_context(),
                            );
                        })
                        .child(text("REEVAL_ACTIVE")),
                )
                .child(layout_error_banner(layout_ui, profiles, controls))
                .child(text(move || {
                    format!(
                        "ACTIVE_SPEC|name={}|tripped={}|footer={:?}",
                        active_spec.with(|s| s.name.clone()),
                        layout_ui.circuit_tripped.get(),
                        active_spec.with(|s| s.footer),
                    )
                }))
        }

        let (mut core, mut tree) = mount::<BreakerBannerHarness>(Size::new(px(1200.0), px(760.0)));
        // 1 timeout during init + 2 more explicit evaluations = 3 strikes -> trips breaker.
        assert!(tree.has_text_containing("tripped=false"));
        assert!(tree.press_text(&mut core, "REEVAL_ACTIVE"));
        assert!(tree.has_text_containing("tripped=false"));
        assert!(tree.press_text(&mut core, "REEVAL_ACTIVE"));
        assert!(tree.has_text_containing("tripped=true"));
        assert!(crate::script_worker::is_circuit_tripped(&script_id));

        let retry_lbl = match lang {
            Lang::Zh => "重试启用",
            Lang::En => "Retry script",
        };
        let paused_prefix = match lang {
            Lang::Zh => "自定义布局连续超时已自动熔断",
            Lang::En => "Custom layout auto-paused after repeated timeouts",
        };
        assert!(tree.has_text_containing(paused_prefix));
        assert!(tree.has_text(retry_lbl));

        // Fix the script on disk and recover via the Retry button in the banner.
        std::fs::write(
            &script_path,
            r#"import { Layout, Row, Pane } from "zniff/layout";
export default () => (
  <Layout name="Recovered QA" footer="top">
    <Row>
      <Pane name="flow_list" size="48%" />
      <Pane name="detail" size="1fr" />
    </Row>
  </Layout>
);
"#,
        )
        .unwrap();

        assert!(tree.press_text(&mut core, retry_lbl));
        assert!(!crate::script_worker::is_circuit_tripped(&script_id));
        assert!(!tree.has_text_containing(paused_prefix));
        assert!(tree.has_text("ACTIVE_SPEC|name=Recovered QA|tripped=false|footer=Top"));
    }
}

#[test]
fn hook_preview_cards_transition_from_hook_flow_to_plain_flow_resets_cards() {
    struct TempCleanup(std::path::PathBuf);
    impl Drop for TempCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let controls = make_controls("hook-preview-cards");
    let dir = controls.hub.ca.dir().to_path_buf();
    let _cleanup = TempCleanup(dir.clone());
    let hooks_dir = dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    std::fs::write(
        hooks_dir.join("dec.tsx"),
        r#"
import { Section, Badge, KeyValue, Code } from "zniff/hook";
export function onPreview(ctx) {
  return {
    res: { body: '{"plain":"ok"}', contentType: "application/json", note: "Decrypted" },
    cards: (
      <Section title="解密摘要">
        <Badge tone="ok">AES-256-GCM</Badge>
        <KeyValue label="Host" value={ctx.host} />
        <Code language="json">{'{"plain":"ok"}'}</Code>
      </Section>
    ),
  };
}
"#,
    )
    .unwrap();

    controls
        .hub
        .set_script_hooks(&[crate::hook_spec::ScriptHookConfig {
            id: "dec".into(),
            name: "Decrypt".into(),
            script: "dec.tsx".into(),
            enabled: true,
            match_pattern: "api.vicanso.dev".into(),
            stages: vec![crate::hook_spec::HookStage::Preview],
            timeout_ms: 500,
            vars: std::collections::BTreeMap::new(),
        }]);
    controls.hub.set_rules_enabled(true);

    let id_a = seed_http_flow(
        &controls,
        "api.vicanso.dev",
        "/v1/cipher",
        200,
        b"\x01\x02encrypted",
    );
    let id_b = seed_http_flow(
        &controls,
        "cdn.vicanso.dev",
        "/plain",
        200,
        br#"{"plain_flow":true}"#,
    );

    #[derive(Clone)]
    struct HookPreviewHarnessState {
        controls: Arc<Controls>,
        id_a: u64,
        id_b: u64,
    }

    thread_local! {
        static HOOK_HARNESS: RefCell<Option<HookPreviewHarnessState>> = const { RefCell::new(None) };
    }
    HOOK_HARNESS.with(|slot| {
        *slot.borrow_mut() = Some(HookPreviewHarnessState {
            controls: controls.clone(),
            id_a,
            id_b,
        });
    });

    #[component]
    fn PreviewCardHarness() -> impl IntoElement {
        provide_base_contexts(Lang::Zh);
        provide_mono_font(false);
        let st = HOOK_HARNESS.with(|slot| slot.borrow().as_ref().unwrap().clone());
        let shown = make_shown("", Some(200), "");
        let c1 = st.controls.clone();
        let s1 = shown;
        let id_a = st.id_a;
        let c2 = st.controls.clone();
        let s2 = shown;
        let id_b = st.id_b;
        view()
            .w(px(900.0))
            .h(px(700.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        publish_detail_for_controls(&s1, &c1, Some(id_a), Lang::Zh, None);
                    })
                    .child(text("SELECT_A")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        publish_detail_for_controls(&s2, &c2, Some(id_b), Lang::Zh, None);
                    })
                    .child(text("SELECT_B")),
            )
            .child(hook_preview_cards(shown.hook_cards))
            .child(text(move || {
                format!(
                    "STATE|pretty={}|cards={}|note={}",
                    shown.pretty.get(),
                    shown.hook_cards.with(Vec::len),
                    shown.response_note.get(),
                )
            }))
    }

    let (mut core, mut tree) = mount::<PreviewCardHarness>(Size::new(px(900.0), px(700.0)));
    assert!(tree.press_text(&mut core, "SELECT_A"));
    // Poll briefly until async preview worker caches the result for flow A.
    let flow_a = controls.hub.with_session(|s| s.hydrated(id_a)).unwrap();
    for _ in 0..80 {
        if matches!(
            controls.hub.preview_hook_body(&flow_a, true),
            Some(crate::webhook::PreviewLookup::Ready(_))
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(tree.press_text(&mut core, "SELECT_A"));
    assert!(tree.has_text("STATE|pretty=true|cards=1|note=hook 预览解码 · Decrypted"));
    assert!(tree.has_text("解密摘要"));
    assert!(tree.has_text("AES-256-GCM"));
    assert!(tree.has_text("api.vicanso.dev"));

    // Transition A -> B (plain flow without hook): pretty stays preserved, cards and hook note reset.
    assert!(tree.press_text(&mut core, "SELECT_B"));
    assert!(tree.has_text("STATE|pretty=true|cards=0|note="));
    assert!(!tree.has_text("解密摘要"));
    assert!(!tree.has_text("AES-256-GCM"));
}

#[test]
fn hook_pure_insertion_components_actions_and_modal_transitions() {
    use crate::hook_spec::{
        HookViewAction, HookViewTabItem, PreviewCardNode, PreviewTone, TextSizeTier,
    };

    struct TempCleanup(std::path::PathBuf);
    impl Drop for TempCleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let controls = make_controls("hook-pure-insert");
    let dir = controls.hub.ca.dir().to_path_buf();
    let _cleanup = TempCleanup(dir.clone());
    let hooks_dir = dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    std::fs::write(
        hooks_dir.join("ins.tsx"),
        r#"
import { Badge } from "zniff/hook";
export function onPreview(ctx) {
  return {
    view: <Badge tone="accent" setFilter="host:api.vicanso.dev">FILTER_V</Badge>,
    req: { view: <Badge tone="neutral">REQ_PREVIEW_SIDE</Badge> },
    res: { view: <Badge tone="ok">RES_PREVIEW_SIDE</Badge> },
  };
}
"#,
    )
    .unwrap();
    controls
        .hub
        .set_script_hooks(&[crate::hook_spec::ScriptHookConfig {
            id: "ins".into(),
            name: "Insert Preview".into(),
            script: "ins.tsx".into(),
            enabled: true,
            match_pattern: "api.vicanso.dev".into(),
            stages: vec![crate::hook_spec::HookStage::Preview],
            timeout_ms: 500,
            vars: std::collections::BTreeMap::new(),
        }]);
    controls.hub.set_rules_enabled(true);

    let id_a = controls.hub.open(crate::session::FlowDraft {
        protocol: crate::session::Protocol::Http11,
        method: "POST".into(),
        host: "api.vicanso.dev".into(),
        path: "/v1/orders?page=1".into(),
        url: "https://api.vicanso.dev/v1/orders?page=1".into(),
        request_headers: vec![("X-Origin-Hdr".into(), "kept".into())],
    });
    controls.hub.append_body_bytes_for_test(
        id_a,
        crate::session::Direction::Request,
        b"{\"req\":1}",
    );
    controls.hub.set_response_for_test(
        id_a,
        200,
        vec![("Content-Type".into(), "application/json".into())],
    );
    controls.hub.append_body_bytes_for_test(
        id_a,
        crate::session::Direction::Response,
        b"{\"res\":2}",
    );
    controls.hub.mark_finished(id_a);

    let id_b = seed_http_flow(
        &controls,
        "cdn.vicanso.dev",
        "/plain",
        200,
        br#"{"plain":true}"#,
    );

    // Live views from onRequest / onResponse stages stored in Hub's sidecar store.
    controls.hub.append_live_hook_view(
        id_a,
        false,
        vec![PreviewCardNode::Callout {
            tone: PreviewTone::Warn,
            title: Some("Req Live Callout".into()),
            text: "Kept alongside original body".into(),
            children: vec![PreviewCardNode::Badge {
                tone: PreviewTone::Accent,
                text: "JUMP_B".into(),
                action: HookViewAction {
                    select_flow_id: Some(id_b),
                    ..Default::default()
                },
            }],
            action: HookViewAction::default(),
        }],
    );
    controls.hub.append_live_hook_view(
        id_a,
        true,
        vec![
            PreviewCardNode::Collapsible {
                title: "细节折叠".into(),
                default_open: false,
                children: vec![PreviewCardNode::Text {
                    text: "隐藏详细字段".into(),
                    tone: PreviewTone::Ok,
                    mono: false,
                    bold: false,
                    size: Some(TextSizeTier::Sm),
                    action: HookViewAction::default(),
                }],
            },
            PreviewCardNode::Tabs {
                items: vec![
                    HookViewTabItem {
                        label: "概要".into(),
                        children: vec![PreviewCardNode::Badge {
                            tone: PreviewTone::Ok,
                            text: "TAB1_ACTIVE".into(),
                            action: HookViewAction::default(),
                        }],
                    },
                    HookViewTabItem {
                        label: "表格".into(),
                        children: vec![PreviewCardNode::Table {
                            columns: vec!["Field".into(), "Val".into()],
                            rows: vec![vec!["env".into(), "vicanso-prod".into()]],
                        }],
                    },
                ],
            },
        ],
    );

    #[derive(Clone)]
    struct PureInsertState {
        controls: Arc<Controls>,
        id_a: u64,
        id_b: u64,
    }
    thread_local! {
        static PURE_INSERT_ST: RefCell<Option<PureInsertState>> = const { RefCell::new(None) };
    }
    PURE_INSERT_ST.with(|slot| {
        *slot.borrow_mut() = Some(PureInsertState {
            controls: controls.clone(),
            id_a,
            id_b,
        });
    });

    #[component]
    fn PureInsertHarness() -> impl IntoElement {
        provide_base_contexts(Lang::Zh);
        provide_mono_font(false);
        let st = PURE_INSERT_ST.with(|slot| slot.borrow().as_ref().unwrap().clone());
        let profiles = make_profiles();
        let selected = signal(None::<u64>);
        let modal_open = signal(None::<HookRunModalData>);
        let shown = make_shown("", Some(200), "");
        let ctx = HookViewActionCtx::for_inspector(
            selected,
            profiles.filter,
            Some(profiles.filter_enabled),
        );
        let c1 = st.controls.clone();
        let c2 = st.controls.clone();
        let id_a = st.id_a;
        let id_b = st.id_b;
        view()
            .w(px(960.0))
            .h(px(760.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        selected.set(Some(id_a));
                        publish_detail_for_controls(&shown, &c1, Some(id_a), Lang::Zh, None);
                    })
                    .child(text("LOAD_A")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        selected.set(Some(id_b));
                        publish_detail_for_controls(&shown, &c2, Some(id_b), Lang::Zh, None);
                    })
                    .child(text("LOAD_B")),
            )
            .child(
                button()
                    .on_press(move |_: &()| {
                        modal_open.set(Some(HookRunModalData {
                            hook_id: "audit".into(),
                            summary: "Modal summary ok".into(),
                            cards: vec![PreviewCardNode::Badge {
                                tone: PreviewTone::Ok,
                                text: "MODAL_BADGE".into(),
                                action: HookViewAction::default(),
                            }],
                            duration_ms: 12,
                        }));
                    })
                    .child(text("OPEN_RUN_MODAL")),
            )
            .child(hook_preview_cards_with_actions(
                shown.hook_cards,
                ctx.clone(),
            ))
            .child(hook_preview_cards_with_actions(
                shown.req_hook_cards,
                ctx.clone(),
            ))
            .child(text(move || {
                format!("ORIG_REQ={}", shown.request_body.get())
            }))
            .child(hook_preview_cards_with_actions(shown.res_hook_cards, ctx))
            .child(text(move || {
                format!("ORIG_RES={}", shown.response_body.get())
            }))
            .child(text(move || {
                format!(
                    "HUD|sel={:?}|filter={}|pretty={}|ov={}|req={}|res={}",
                    selected.get(),
                    profiles.filter.get(),
                    shown.pretty.get(),
                    shown.hook_cards.with(Vec::len),
                    shown.req_hook_cards.with(Vec::len),
                    shown.res_hook_cards.with(Vec::len),
                )
            }))
            .child(hook_run_modal(modal_open, selected, profiles))
    }

    let (mut core, mut tree) = mount::<PureInsertHarness>(Size::new(px(960.0), px(760.0)));
    assert!(tree.press_text(&mut core, "LOAD_A"));
    let flow_a = controls.hub.with_session(|s| s.hydrated(id_a)).unwrap();
    for _ in 0..80 {
        let req_ready = matches!(
            controls.hub.preview_hook_body(&flow_a, false),
            Some(crate::webhook::PreviewLookup::Ready(_))
        );
        let res_ready = matches!(
            controls.hub.preview_hook_body(&flow_a, true),
            Some(crate::webhook::PreviewLookup::Ready(_))
        );
        if req_ready && res_ready {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(tree.press_text(&mut core, "LOAD_A"));

    // Pure insertion: inserted stage views AND original bodies coexist cleanly.
    assert!(tree.has_text("FILTER_V"));
    assert!(tree.has_text("REQ_PREVIEW_SIDE"));
    assert!(tree.has_text("RES_PREVIEW_SIDE"));
    assert!(tree.has_text("Req Live Callout"));
    assert!(tree.has_text_containing("\"req\": 1"));
    assert!(tree.has_text_containing("\"res\": 2"));
    assert!(tree.has_text("TAB1_ACTIVE"));
    assert!(!tree.has_text("vicanso-prod"));
    assert!(!tree.has_text("隐藏详细字段"));

    // Interactive Tabs + Collapsible + Host Actions (`setFilter`, `selectFlowId`).
    assert!(tree.press_text(&mut core, "表格"));
    assert!(tree.has_text("vicanso-prod"));
    assert!(tree.press_text(&mut core, "细节折叠"));
    assert!(tree.has_text("隐藏详细字段"));

    assert!(tree.press_text(&mut core, "FILTER_V"));
    assert!(tree.has_text(&format!(
        "HUD|sel=Some({id_a})|filter=host:api.vicanso.dev|pretty=true|ov=1|req=2|res=3"
    )));

    assert!(tree.press_text(&mut core, "JUMP_B"));
    assert!(tree.has_text_containing(&format!("HUD|sel=Some({id_b})|")));

    // A -> B transition clears all 3 view signals while keeping pretty and showing plain B.
    assert!(tree.press_text(&mut core, "LOAD_B"));
    assert!(tree.has_text(&format!(
        "HUD|sel=Some({id_b})|filter=host:api.vicanso.dev|pretty=true|ov=0|req=0|res=0"
    )));
    assert!(!tree.has_text("Req Live Callout"));

    // onRun modal opens cleanly and closes on Escape.
    assert!(tree.press_text(&mut core, "OPEN_RUN_MODAL"));
    assert!(tree.has_text("MODAL_BADGE"));
    assert!(tree.has_text("Modal summary ok"));
    assert!(tree.press_escape(&mut core, false));
    assert!(!tree.has_text("MODAL_BADGE"));
}

#[test]
fn transparent_proxy_empty_targets_blocks_start_expands_and_warns_when_cleared_zh_en() {
    #[derive(Clone)]
    struct TpHarnessState {
        controls: Arc<Controls>,
    }

    thread_local! {
        static TP_HARNESS: RefCell<Option<TpHarnessState>> = const { RefCell::new(None) };
    }

    #[component]
    fn TpCardHarness() -> impl IntoElement {
        let lang = TEST_LANG.get();
        provide_base_contexts(lang);
        let controls = make_controls("tp-empty-targets");
        let tp_status = signal(crate::transparent_proxy::TransparentProxyStatus {
            embedded: true,
            in_applications: true,
            state: crate::transparent_proxy::TpState::Idle,
            active: false,
            error: String::new(),
            passwordless_ready: false,
            setcap_command: String::new(),
        });
        let tp_apps = signal(vec![
            crate::transparent_proxy::RunningApp {
                name: "Firefox".into(),
                bundle_id: "org.mozilla.firefox".into(),
                exec_name: "firefox".into(),
            },
            crate::transparent_proxy::RunningApp {
                name: "curl".into(),
                bundle_id: String::new(),
                exec_name: "curl".into(),
            },
        ]);
        let open_popover = signal(true);
        TP_HARNESS.with(|slot| {
            *slot.borrow_mut() = Some(TpHarnessState {
                controls: controls.clone(),
            });
        });

        let set_running_sig = tp_status;
        view()
            .w(px(520.0))
            .h(px(640.0))
            .flex_col()
            .child(
                button()
                    .on_press(move |_: &()| {
                        set_running_sig.update(|st| {
                            st.state = crate::transparent_proxy::TpState::Running;
                            st.active = true;
                        });
                    })
                    .child(text("SET_RUNNING")),
            )
            .child(text(move || {
                format!("TP_ON={}", tp_status.with(|s| s.is_on_or_starting()))
            }))
            .child(transparent_proxy_card(
                controls,
                tp_status,
                tp_apps,
                open_popover,
            ))
    }

    for lang in [Lang::Zh, Lang::En] {
        TEST_LANG.set(lang);
        let (mut core, mut tree) = mount::<TpCardHarness>(Size::new(px(520.0), px(640.0)));
        let st = TP_HARNESS
            .with(|slot| slot.borrow().clone())
            .expect("tp harness");

        let (
            start_btn,
            stop_btn,
            scope_lbl,
            sel_apps,
            all_apps,
            idle_sub,
            prompt_sub,
            prompt_banner,
            run_ok_sub,
            run_idle_sub,
            run_idle_banner,
        ) = match lang {
            Lang::Zh => (
                "开启",
                "停止",
                "拦截范围",
                "指定应用",
                "全部应用",
                "按应用直连抓包，不改系统代理",
                "请先选择目标应用或切为「全部应用」",
                "再点击「开启」",
                "运行中 · 不改系统代理",
                "运行中 · 未选择应用（暂不拦截）",
                "暂不会拦截流量",
            ),
            Lang::En => (
                "Start",
                "Stop",
                "Capture Scope",
                "Selected Apps",
                "All Apps",
                "Per-app capture, no OS proxy",
                "Select target apps or switch to All Apps",
                "then click Start",
                "Running · No OS proxy change",
                "Running · No app selected (idle)",
                "no traffic is intercepted yet",
            ),
        };

        // Initial collapsed idle state with no targets shows normal idle subtitle & no warning.
        assert!(tree.has_text("TP_ON=false"));
        assert!(tree.has_text(idle_sub));
        assert!(!tree.has_text(scope_lbl));
        assert!(!tree.has_text_containing(prompt_banner));

        // 1. Clicking Start with empty targets blocks helper startup, auto-expands card, and prompts user.
        assert!(tree.press_text(&mut core, start_btn));
        assert!(matches!(
            *st.controls.status.lock().unwrap(),
            StatusMsg::TransparentProxyNeedsTargets
        ));
        assert!(tree.has_text("TP_ON=false"));
        assert!(tree.has_text(scope_lbl));
        assert!(tree.has_text("Firefox"));
        assert!(tree.has_text(prompt_sub));
        assert!(tree.has_text_containing(prompt_banner));

        // 2. A -> B: selecting a target chip persists config and clears the pre-start warning.
        assert!(tree.press_text(&mut core, "Firefox"));
        assert_eq!(saved_config().tp_targets, "Firefox");
        assert!(tree.has_text(idle_sub));
        assert!(!tree.has_text(prompt_sub));
        assert!(!tree.has_text_containing(prompt_banner));

        // 3. B -> C: transition to Running, then clear targets while running -> stays Running with warning.
        assert!(tree.press_text(&mut core, "SET_RUNNING"));
        assert!(tree.has_text("TP_ON=true"));
        assert!(tree.has_text(stop_btn));
        assert!(tree.has_text(run_ok_sub));
        assert!(!tree.has_text_containing(run_idle_banner));

        assert!(tree.press_text(&mut core, "Firefox"));
        assert_eq!(saved_config().tp_targets, "");
        assert!(tree.has_text("TP_ON=true"));
        assert!(tree.has_text(stop_btn));
        assert!(tree.has_text(run_idle_sub));
        assert!(tree.has_text_containing(run_idle_banner));

        // 4. C -> D: switching to All Apps clears warning; switching back to Selected Apps restores warning.
        assert!(tree.press_text(&mut core, all_apps));
        assert!(saved_config().tp_capture_all);
        assert!(tree.has_text(run_ok_sub));
        assert!(!tree.has_text_containing(run_idle_banner));

        assert!(tree.press_text(&mut core, sel_apps));
        assert!(!saved_config().tp_capture_all);
        assert!(tree.has_text(run_idle_sub));
        assert!(tree.has_text_containing(run_idle_banner));
    }
}
