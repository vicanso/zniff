//! Contextual quick-rule prefill helpers and menu actions for captured flows.

#[allow(unused_imports)]
use super::*;

pub(crate) const QUICK_RULE_LOCAL_KIND: u8 = 2;
pub(crate) const QUICK_RULE_DELAY_KIND: u8 = 3;
pub(crate) const QUICK_RULE_HEADER_KIND: u8 = 6;
pub(crate) const QUICK_RULE_BLOCK_KIND: u8 = 13;
pub(crate) const QUICK_RULE_RESOLVE_KIND: u8 = 14;

/// Structured prefill payload applied to the Quick Add Rule form when opened from a flow or error.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct QuickRulePrefill {
    pub(crate) kind: u8,
    pub(crate) host: String,
    pub(crate) path: String,
    pub(crate) op: String,
    pub(crate) from: String,
    pub(crate) to: String,
    /// Optional mode toggle applied to `QuickRuleModeSignals::signal_for(kind)`.
    pub(crate) mode: Option<bool>,
}

impl QuickRulePrefill {
    pub(crate) fn resolve_host(host: String) -> Self {
        Self {
            kind: QUICK_RULE_RESOLVE_KIND,
            host,
            ..Self::default()
        }
    }
}

/// Bundle of Quick Add form signals updated together when a contextual prefill arrives.
#[derive(Clone, Copy)]
pub(crate) struct QuickRuleDraftSignals {
    pub(crate) kind: Signal<u8>,
    pub(crate) host: Signal<String>,
    pub(crate) path: Signal<String>,
    pub(crate) op: Signal<String>,
    pub(crate) from: Signal<String>,
    pub(crate) to: Signal<String>,
    pub(crate) modes: QuickRuleModeSignals,
}

/// Contextual one-click quick-rule templates offered in the flow row and detail menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlowQuickRuleAction {
    MockResponse,
    ModifyReqHeader,
    DelayEndpoint,
    BlockDomain,
}

impl FlowQuickRuleAction {
    pub(crate) const ALL: [Self; 4] = [
        Self::MockResponse,
        Self::ModifyReqHeader,
        Self::DelayEndpoint,
        Self::BlockDomain,
    ];

    pub(crate) fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::MockResponse => ("Mock 此响应", "Mock this response"),
            Self::ModifyReqHeader => ("改此请求头", "Modify request header"),
            Self::DelayEndpoint => ("延迟此接口", "Delay this endpoint"),
            Self::BlockDomain => ("阻断此域名", "Block this domain"),
        }
    }
}

/// Strips query/fragment parts and returns a clean leading-slash path suitable for rule patterns.
pub(crate) fn normalize_rule_path(raw: &str) -> String {
    let base = raw.split(['?', '#']).next().unwrap_or(raw).trim();
    if base.starts_with('/') && !base.contains(char::is_whitespace) {
        base.to_string()
    } else {
        String::new()
    }
}

/// Builds a `QuickRulePrefill` for a captured flow and contextual quick-rule action.
pub(crate) fn prefill_for_flow_action(
    action: FlowQuickRuleAction,
    host: &str,
    path: &str,
    status: Option<u16>,
) -> Option<QuickRulePrefill> {
    let clean_host = normalize_flow_host(host);
    if clean_host.is_empty() || clean_host.contains(char::is_whitespace) {
        return None;
    }
    let clean_path = normalize_rule_path(path);
    let prefill = match action {
        FlowQuickRuleAction::MockResponse => {
            let code = status.filter(|c| (100..600).contains(c)).unwrap_or(200);
            QuickRulePrefill {
                kind: QUICK_RULE_LOCAL_KIND,
                host: clean_host,
                path: clean_path,
                to: code.to_string(),
                mode: Some(false),
                ..QuickRulePrefill::default()
            }
        }
        FlowQuickRuleAction::ModifyReqHeader => QuickRulePrefill {
            kind: QUICK_RULE_HEADER_KIND,
            host: clean_host,
            path: clean_path,
            mode: Some(false),
            ..QuickRulePrefill::default()
        },
        FlowQuickRuleAction::DelayEndpoint => QuickRulePrefill {
            kind: QUICK_RULE_DELAY_KIND,
            host: clean_host,
            path: clean_path,
            from: "1000ms".to_string(),
            ..QuickRulePrefill::default()
        },
        FlowQuickRuleAction::BlockDomain => QuickRulePrefill {
            kind: QUICK_RULE_BLOCK_KIND,
            host: clean_host,
            mode: Some(false),
            ..QuickRulePrefill::default()
        },
    };
    Some(prefill)
}

/// Applies a `QuickRulePrefill` into the Quick Add Rule form signals.
pub(crate) fn apply_quick_rule_prefill(prefill: QuickRulePrefill, draft: QuickRuleDraftSignals) {
    draft.kind.set(prefill.kind);
    draft.host.set(prefill.host);
    draft.path.set(prefill.path);
    draft.op.set(prefill.op);
    draft.from.set(prefill.from);
    draft.to.set(prefill.to);
    if let Some(mode) = prefill.mode {
        draft.modes.signal_for(prefill.kind).set(mode);
    }
}

/// Prefills the Quick Add Rule form from the selected flow and opens the rules panel.
pub(crate) fn open_quick_rule_for_flow(
    controls: &Controls,
    profiles: ProfileSignals,
    id: Option<u64>,
    action: FlowQuickRuleAction,
) {
    if refuse_relay(controls, id) {
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let Some(prefill) = prefill_for_flow_action(action, &flow.host, &flow.path, flow.status) else {
        return;
    };
    profiles.quick_rule_prefill.set(Some(prefill));
    profiles.rules_panel_tab.set(0);
    profiles.panel_open.set(true);
}

/// Compact grouped section of contextual quick-rule menu items shared by row and detail menus.
pub(crate) fn flow_quick_rule_section(
    enabled: impl Fn() -> bool + Clone + 'static,
    on_pick: impl Fn(FlowQuickRuleAction) + Clone + 'static,
) -> impl IntoElement {
    view()
        .w_full()
        .gap_0p5()
        .child(menu_rule())
        .child(
            view().px_2p5().py_0p5().child(
                text(tr("快速规则", "Quick rule"))
                    .number_of_lines(1)
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg_muted()),
            ),
        )
        .children(FlowQuickRuleAction::ALL.into_iter().map(move |action| {
            let (zh, en) = action.label();
            let on_pick = on_pick.clone();
            action_item_enabled(tr(zh, en), enabled.clone(), move || on_pick(action))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_response_prefills_local_rule_with_host_path_and_status() {
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::MockResponse,
            "API.Example.com:443",
            "/v1/users?page=2#top",
            Some(201),
        )
        .expect("prefill should be built");

        assert_eq!(prefill.kind, QUICK_RULE_LOCAL_KIND);
        assert_eq!(quick_kind(prefill.kind), crate::rules::QuickRule::Local);
        assert_eq!(prefill.host, "api.example.com");
        assert_eq!(prefill.path, "/v1/users");
        assert_eq!(prefill.from, "");
        assert_eq!(prefill.to, "201");
        assert_eq!(prefill.mode, Some(false));

        let pattern = crate::rules::combine_rule_pattern(&prefill.host, &prefill.path);
        let line = crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(prefill.kind),
            &pattern,
            prefill.mode.unwrap_or(false),
            &prefill.from,
            &prefill.to,
            &prefill.op,
        )
        .expect("mock response prefill should be immediately addable");
        assert_eq!(line.trim(), "map-local api.example.com/v1/users status=201");
    }

    #[test]
    fn mock_response_defaults_status_to_200_when_absent() {
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::MockResponse,
            "api.example.com",
            "/v1/ping",
            None,
        )
        .expect("prefill should be built");
        assert_eq!(prefill.to, "200");
    }

    #[test]
    fn modify_req_header_prefills_header_rule_in_req_phase() {
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::ModifyReqHeader,
            "api.example.com",
            "/v1/orders?limit=10",
            Some(200),
        )
        .expect("prefill should be built");

        assert_eq!(prefill.kind, QUICK_RULE_HEADER_KIND);
        assert_eq!(quick_kind(prefill.kind), crate::rules::QuickRule::Header);
        assert_eq!(prefill.host, "api.example.com");
        assert_eq!(prefill.path, "/v1/orders");
        assert_eq!(prefill.from, "");
        assert_eq!(prefill.to, "");
        assert_eq!(prefill.mode, Some(false));

        let pattern = crate::rules::combine_rule_pattern(&prefill.host, &prefill.path);
        let line = crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(prefill.kind),
            &pattern,
            prefill.mode.unwrap_or(true),
            "X-Debug",
            "1",
            &prefill.op,
        )
        .expect("header prefill should build a req header rule");
        assert_eq!(
            line.trim(),
            "header api.example.com/v1/orders req set X-Debug 1"
        );
    }

    #[test]
    fn delay_endpoint_prefills_1000ms_for_host_and_path() {
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::DelayEndpoint,
            "api.example.com",
            "/v1/slow?x=1",
            Some(200),
        )
        .expect("prefill should be built");

        assert_eq!(prefill.kind, QUICK_RULE_DELAY_KIND);
        assert_eq!(quick_kind(prefill.kind), crate::rules::QuickRule::Delay);
        assert_eq!(prefill.host, "api.example.com");
        assert_eq!(prefill.path, "/v1/slow");
        assert_eq!(prefill.from, "1000ms");
        assert_eq!(prefill.to, "");
        assert_eq!(prefill.mode, None);

        let pattern = crate::rules::combine_rule_pattern(&prefill.host, &prefill.path);
        let line = crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(prefill.kind),
            &pattern,
            false,
            &prefill.from,
            &prefill.to,
            &prefill.op,
        )
        .expect("delay prefill should be immediately addable");
        assert_eq!(line.trim(), "delay api.example.com/v1/slow 1000ms");
    }

    #[test]
    fn block_domain_prefills_host_only_in_status_mode() {
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::BlockDomain,
            "tracker.example.com:443",
            "/pixel.gif?id=9",
            Some(200),
        )
        .expect("prefill should be built");

        assert_eq!(prefill.kind, QUICK_RULE_BLOCK_KIND);
        assert_eq!(quick_kind(prefill.kind), crate::rules::QuickRule::Block);
        assert_eq!(prefill.host, "tracker.example.com");
        assert_eq!(prefill.path, "");
        assert_eq!(prefill.from, "");
        assert_eq!(prefill.mode, Some(false));

        let pattern = crate::rules::combine_rule_pattern(&prefill.host, &prefill.path);
        let line = crate::rules::append_quick_rule_with_op(
            "",
            quick_kind(prefill.kind),
            &pattern,
            prefill.mode.unwrap_or(false),
            &prefill.from,
            &prefill.to,
            &prefill.op,
        )
        .expect("block prefill should be immediately addable");
        assert_eq!(line.trim(), "block tracker.example.com");
    }

    #[test]
    fn empty_host_returns_none_and_non_slash_path_is_cleared() {
        assert!(
            prefill_for_flow_action(FlowQuickRuleAction::DelayEndpoint, "   ", "/a", None)
                .is_none()
        );
        let prefill = prefill_for_flow_action(
            FlowQuickRuleAction::DelayEndpoint,
            "example.com",
            "CONNECT tunnel",
            None,
        )
        .expect("valid host with non-path should still prefill");
        assert_eq!(prefill.path, "");
    }

    #[test]
    fn resolve_host_prefill_uses_resolve_kind() {
        let prefill = QuickRulePrefill::resolve_host("api.example.com".to_string());
        assert_eq!(prefill.kind, QUICK_RULE_RESOLVE_KIND);
        assert_eq!(quick_kind(prefill.kind), crate::rules::QuickRule::Resolve);
        assert_eq!(prefill.host, "api.example.com");
        assert_eq!(prefill.path, "");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn apply_prefill_updates_draft_signals_and_mode_slot() {
        use nsh_core::{ApproxTextEngine, Core, Op, Prop, ROOT_ID, Tag};
        use std::sync::atomic::{AtomicBool, Ordering};

        static RAN: AtomicBool = AtomicBool::new(false);
        RAN.store(false, Ordering::SeqCst);

        #[component]
        fn PrefillHarness() -> impl IntoElement {
            let draft = QuickRuleDraftSignals {
                kind: signal(0_u8),
                host: signal("old.test".to_string()),
                path: signal("/old".to_string()),
                op: signal("OldOp".to_string()),
                from: signal("old_from".to_string()),
                to: signal("old_to".to_string()),
                modes: QuickRuleModeSignals::new(),
            };
            assert!(draft.modes.req_res.get_untracked());

            let prefill = prefill_for_flow_action(
                FlowQuickRuleAction::ModifyReqHeader,
                "api.example.com",
                "/v2/items",
                Some(200),
            )
            .unwrap();
            apply_quick_rule_prefill(prefill, draft);

            assert_eq!(draft.kind.get_untracked(), QUICK_RULE_HEADER_KIND);
            assert_eq!(draft.host.get_untracked(), "api.example.com");
            assert_eq!(draft.path.get_untracked(), "/v2/items");
            assert_eq!(draft.op.get_untracked(), "");
            assert_eq!(draft.from.get_untracked(), "");
            assert_eq!(draft.to.get_untracked(), "");
            assert!(!draft.modes.req_res.get_untracked());
            RAN.store(true, Ordering::SeqCst);
            view()
        }

        let mut core = Core::new(ApproxTextEngine, Size::new(px(400.0), px(200.0)));
        let mut registry = Registry::new();
        registry.register::<PrefillHarness>();
        core.set_components(registry);
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(std::any::type_name::<PrefillHarness>().into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply op");
        }
        let _ = core.commit();
        assert!(RAN.load(Ordering::SeqCst));
    }
}
