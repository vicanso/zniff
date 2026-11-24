//! Actionable repair suggestions and recovery buttons for failed (`ERR`) and `407` flows.

#[allow(unused_imports)]
use super::*;

/// High-level recovery category inferred from a flow's status code and error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorCategory {
    /// Client or origin TLS certificate trust / pinning / handshake failure.
    TlsTrust,
    /// Upstream connection refusal, DNS resolution failure, or network timeout.
    NetworkUpstream,
    /// HTTP 407 Proxy Authentication Required.
    ProxyAuth,
}

/// Concrete one-click recovery action rendered inside an ERR flow's detail summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorAction {
    BypassTls,
    InstallCa,
    OpenInBrowser,
    CheckUpstream,
    AddResolve,
    OpenSecurity,
}

impl ErrorAction {
    pub(crate) fn label(self, lang: Lang, bypassed: bool) -> &'static str {
        match self {
            Self::BypassTls if bypassed => t(lang, "已在直通名单", "In bypass list"),
            Self::BypassTls => t(lang, "加入直通", "Bypass TLS"),
            Self::InstallCa => t(lang, "安装证书", "Install CA"),
            Self::OpenInBrowser => t(lang, "隔离浏览器打开", "Open in Browser"),
            Self::CheckUpstream => t(lang, "检查上游", "Check Upstream"),
            Self::AddResolve => t(lang, "添加 resolve", "Add resolve"),
            Self::OpenSecurity => t(lang, "打开安全设置", "Open Security"),
        }
    }

    pub(crate) fn glyph(self, bypassed: bool) -> Glyph {
        match self {
            Self::BypassTls if bypassed => Glyph::Check,
            Self::BypassTls => Glyph::Shield,
            Self::InstallCa => Glyph::Badge,
            Self::OpenInBrowser => Glyph::Globe,
            Self::CheckUpstream => Glyph::Sliders,
            Self::AddResolve => Glyph::Prompt,
            Self::OpenSecurity => Glyph::Shield,
        }
    }
}

/// Classifies a flow's `(status, error)` into an actionable recovery category.
pub(crate) fn classify_error(status: Option<u16>, error: &str) -> Option<ErrorCategory> {
    if status == Some(407) {
        return Some(ErrorCategory::ProxyAuth);
    }
    let raw = error.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if is_non_actionable_notice(&lower) {
        return None;
    }
    if is_proxy_auth_error(&lower) {
        return Some(ErrorCategory::ProxyAuth);
    }
    if is_tls_trust_error(&lower) {
        return Some(ErrorCategory::TlsTrust);
    }
    if is_network_upstream_error(&lower) {
        return Some(ErrorCategory::NetworkUpstream);
    }
    None
}

pub(crate) fn is_non_actionable_notice(lower: &str) -> bool {
    const IGNORED: &[&str] = &[
        "直通名单",
        "bypass list",
        "断点丢弃",
        "dropped at the break",
        "规则阻断",
        "blocked by rule",
        "不是 tls",
        "not tls",
        "响应断点跳过",
        "response break skipped",
    ];
    IGNORED.iter().any(|needle| lower.contains(needle))
}

pub(crate) fn is_tunnel_notice(error: &str) -> bool {
    let lower = error.trim().to_ascii_lowercase();
    const TUNNEL_HINTS: &[&str] = &["直通名单", "bypass list", "不是 tls", "not tls"];
    TUNNEL_HINTS.iter().any(|needle| lower.contains(needle))
}

fn is_proxy_auth_error(lower: &str) -> bool {
    lower.contains("proxy authentication")
        || lower.contains("proxy-authorization")
        || lower.contains("代理认证")
        || (lower.contains("407") && (lower.contains("proxy") || lower.contains("auth")))
}

fn is_tls_trust_error(lower: &str) -> bool {
    const TLS_NEEDLES: &[&str] = &[
        "证书被拒绝",
        "certificate rejected",
        "客户端 tls",
        "client tls",
        "源站证书",
        "origin certificate",
        "源站 tls",
        "origin tls",
        "源站中止握手",
        "origin aborted the handshake",
        "读取 clienthello",
        "read clienthello",
        "reading clienthello",
        "unknown ca",
        "unknownca",
        "bad certificate",
        "badcertificate",
        "certificate unknown",
        "tls alert",
        "handshake",
        "pinning",
        "钉扎",
        "untrusted",
        "不信任",
    ];
    TLS_NEEDLES.iter().any(|needle| lower.contains(needle))
}

fn is_network_upstream_error(lower: &str) -> bool {
    const NET_NEEDLES: &[&str] = &[
        "连接源站超时",
        "connecting to the origin timed out",
        "连接上游代理",
        "上游代理",
        "upstream proxy",
        "没有服务在听",
        "nothing is listening",
        "connection refused",
        "econnrefused",
        "拒绝连接",
        "主机名没有解析",
        "did not resolve",
        "dns",
        "lookup address",
        "nodename nor servname",
        "name or service not known",
        "nxdomain",
        "timed out",
        "timeout",
        "超时",
        "unreachable",
        "no route to host",
        "connection reset",
        "econnreset",
        "连接重置",
        "映射连接",
        "map-remote connect",
    ];
    NET_NEEDLES.iter().any(|needle| lower.contains(needle))
}

/// Returns ordered recovery actions for a flow on desktop (`is_desktop = true`) or web (`false`).
#[cfg(test)]
pub(crate) fn actions_for_error(
    status: Option<u16>,
    error: &str,
    host: &str,
    is_desktop: bool,
) -> Vec<ErrorAction> {
    actions_for_error_with_ca(status, error, host, is_desktop, false)
}

/// Returns ordered recovery actions, omitting `InstallCa` when the host root CA is already trusted.
pub(crate) fn actions_for_error_with_ca(
    status: Option<u16>,
    error: &str,
    host: &str,
    is_desktop: bool,
    ca_trusted: bool,
) -> Vec<ErrorAction> {
    let Some(category) = classify_error(status, error) else {
        return Vec::new();
    };
    let has_host = !normalize_flow_host(host).is_empty();
    match category {
        ErrorCategory::TlsTrust => {
            let mut actions = Vec::new();
            if has_host {
                actions.push(ErrorAction::BypassTls);
            }
            if is_desktop {
                if !ca_trusted {
                    actions.push(ErrorAction::InstallCa);
                }
                actions.push(ErrorAction::OpenInBrowser);
            }
            actions
        }
        ErrorCategory::NetworkUpstream => {
            let mut actions = vec![ErrorAction::CheckUpstream];
            if has_host {
                actions.push(ErrorAction::AddResolve);
            }
            actions
        }
        ErrorCategory::ProxyAuth => vec![ErrorAction::OpenSecurity],
    }
}

/// Extracts a clean lowercase hostname (without port or scheme) from a flow URL or authority.
pub(crate) fn extract_host_from_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    normalize_flow_host(authority)
}

/// Normalizes a hostname or `host:port` authority into a lowercase host suitable for rules/bypass.
pub(crate) fn normalize_flow_host(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let host = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if let Some((h, port)) = trimmed.rsplit_once(':')
        && !h.contains(':')
        && port.chars().all(|c| c.is_ascii_digit())
    {
        h
    } else {
        trimmed
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Builds the initial URL to open in an isolated browser for a flow.
pub(crate) fn browser_url_for_flow(url: &str, host: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let clean_host = normalize_flow_host(host);
    if clean_host.is_empty() {
        None
    } else {
        Some(format!("https://{clean_host}"))
    }
}

/// Returns whether `host` is already covered by the bypass list text in normal (non-`only:`) mode.
pub(crate) fn bypass_contains_host(current: &str, host: &str) -> bool {
    let target = normalize_flow_host(host);
    if target.is_empty() {
        return false;
    }
    let mut only_mode = false;
    let mut listed = false;
    for raw in current.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let token = token
            .strip_prefix("only:")
            .map(|rest| {
                only_mode = true;
                rest.trim()
            })
            .unwrap_or(token);
        if token.is_empty() || token.eq_ignore_ascii_case("only") {
            only_mode = true;
            continue;
        }
        if token == "*" || normalize_flow_host(token) == target {
            listed = true;
        } else if let Some(suffix) = normalize_flow_host(token).strip_prefix("*.")
            && !suffix.is_empty()
            && target.ends_with(&format!(".{suffix}"))
        {
            listed = true;
        }
    }
    if only_mode { !listed } else { listed }
}

/// Appends `host` to the bypass list string if it is not already bypassed.
pub(crate) fn append_bypass_host(current: &str, host: &str) -> Option<String> {
    let target = normalize_flow_host(host);
    if target.is_empty() || bypass_contains_host(current, &target) {
        return None;
    }
    let trimmed = current.trim();
    if trimmed.is_empty() {
        return Some(target);
    }
    // If `only:` allow-list mode is active, remove the target host so it becomes tunneled.
    if trimmed
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
        .any(|tok| {
            let t = tok.trim();
            t.eq_ignore_ascii_case("only") || t.starts_with("only:")
        })
    {
        let mut kept = Vec::new();
        for raw in
            trimmed.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
        {
            let token = raw.trim();
            if token.is_empty() {
                continue;
            }
            if let Some(rest) = token.strip_prefix("only:") {
                if normalize_flow_host(rest) == target {
                    kept.push("only:".to_string());
                } else {
                    kept.push(token.to_string());
                }
            } else if normalize_flow_host(token) != target {
                kept.push(token.to_string());
            }
        }
        return Some(kept.join(" | "));
    }
    Some(format!("{trimmed} | {target}"))
}

fn add_host_to_bypass(controls: &Controls, profiles: ProfileSignals, host: &str) {
    let clean_host = normalize_flow_host(host);
    let Some(next) = append_bypass_host(&profiles.bypass.get_untracked(), host) else {
        return;
    };
    profiles.bypass.set(next.clone());
    controls.hub.set_bypass(&next);
    if let Err(err) = save_config(|config| config.bypass_hosts = next) {
        set_status(
            controls,
            StatusMsg::PrefUnsaved {
                name: PrefName::Bypass,
                err: err.to_string(),
            },
        );
    } else if !clean_host.is_empty() {
        set_status(controls, StatusMsg::HostAddedToBypass(clean_host));
    }
}

pub(crate) fn execute_error_action(
    action: ErrorAction,
    controls: &Arc<Controls>,
    profiles: ProfileSignals,
    url: &str,
    host: &str,
) {
    match action {
        ErrorAction::BypassTls => add_host_to_bypass(controls, profiles, host),
        ErrorAction::InstallCa => {
            #[cfg(not(target_arch = "wasm32"))]
            install_ca(controls.clone());
        }
        ErrorAction::OpenInBrowser => {
            #[cfg(not(target_arch = "wasm32"))]
            launch_browser_action(controls.clone(), browser_url_for_flow(url, host));
            #[cfg(target_arch = "wasm32")]
            let _ = (url, host);
        }
        ErrorAction::CheckUpstream => {
            profiles.rules_panel_tab.set(1);
            profiles.panel_open.set(true);
        }
        ErrorAction::AddResolve => {
            let clean_host = normalize_flow_host(host);
            if !clean_host.is_empty() {
                profiles
                    .quick_rule_prefill
                    .set(Some(QuickRulePrefill::resolve_host(clean_host)));
            }
            profiles.rules_panel_tab.set(0);
            profiles.panel_open.set(true);
        }
        ErrorAction::OpenSecurity => {
            profiles.access_open.set(true);
        }
    }
}

/// Renders the error message banner and contextual one-click repair buttons in the detail summary.
pub(crate) fn error_summary_banner(
    shown: Shown,
    controls: Arc<Controls>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let is_desktop = cfg!(not(target_arch = "wasm32"));
    let langs = ui_langs();
    let ca_ctx = use_context::<TlsFailureBannerSignals>();
    let ca_controls = controls.clone();
    let visible = move || !shown.error.get().is_empty() || shown.status.get() == Some(407);
    let actions = computed(move || {
        let url = shown.url.get();
        let host = extract_host_from_url(&url);
        let ca_trusted = ca_ctx
            .as_ref()
            .map(|s| s.ca_status.get() == ca::CaStatus::Trusted)
            .unwrap_or_else(|| lock_ca_status(&ca_controls) == ca::CaStatus::Trusted);
        actions_for_error_with_ca(
            shown.status.get(),
            &shown.error.get(),
            &host,
            is_desktop,
            ca_trusted,
        )
    });
    let is_neutral = move || {
        let err = shown.error.get();
        shown.status.get() != Some(407) && is_non_actionable_notice(&err.to_ascii_lowercase())
    };
    when(visible, move || {
        let controls = controls.clone();
        view()
            .w_full()
            .overflow_hidden()
            .px_3()
            .py_2()
            .gap_2()
            .rounded(px(RADIUS_MD))
            .bg(move || {
                if is_neutral() {
                    bg_control()
                } else {
                    status_err_bg()
                }
            })
            .border_1()
            .border_color(move || {
                if is_neutral() {
                    border_control()
                } else {
                    status_err_fg()
                }
            })
            .child(
                text(move || -> SharedString {
                    let lang = ui_lang(langs);
                    let err = shown.error.get();
                    if !err.is_empty() {
                        localize_error(lang, &err).into()
                    } else {
                        t(
                            lang,
                            "代理认证失败（HTTP 407），请检查代理认证配置。",
                            "Proxy authentication required (HTTP 407). Check proxy auth settings.",
                        )
                        .into()
                    }
                })
                .selectable(true)
                .on_press_up(dismiss_keyboard)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if is_neutral() {
                        fg_secondary()
                    } else {
                        status_err_fg()
                    }
                }),
            )
            .child(when(
                move || !actions.with(Vec::is_empty),
                move || {
                    let controls = controls.clone();
                    view()
                        .w_full()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(each(
                            move || actions.get(),
                            |act| *act as u8,
                            move |item, _| {
                                let action = item.get_untracked();
                                error_action_chip(action, shown, controls.clone(), profiles, langs)
                            },
                        ))
                },
            ))
    })
}

fn error_action_chip(
    action: ErrorAction,
    shown: Shown,
    controls: Arc<Controls>,
    profiles: ProfileSignals,
    langs: Signal<Option<Lang>>,
) -> impl IntoElement {
    let is_bypassed = move || {
        action == ErrorAction::BypassTls && {
            let host = extract_host_from_url(&shown.url.get());
            bypass_contains_host(&profiles.bypass.get(), &host)
        }
    };
    view()
        .role(AccessibilityRole::Button)
        .h(px(24.0))
        .px_2()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if is_bypassed() {
                status_ok_fg()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_bypassed() {
                status_ok_bg()
            } else {
                bg_card()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_bypassed() {
                    status_ok_bg()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .disabled(is_bypassed)
        .disabled_style(|style| style.opacity(0.6))
        .on_press(move || {
            let url = shown.url.get_untracked();
            let host = extract_host_from_url(&url);
            execute_error_action(action, &controls, profiles, &url, &host);
        })
        .child(glyph_px(
            12.0,
            move || action.glyph(is_bypassed()),
            move || {
                color(if is_bypassed() {
                    status_ok_fg()
                } else {
                    fg_secondary()
                })
            },
        ))
        .child(
            text(move || -> SharedString { action.label(ui_lang(langs), is_bypassed()).into() })
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if is_bypassed() {
                        status_ok_fg()
                    } else {
                        fg_primary()
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_tls_trust_and_pinning_errors() {
        let cases = [
            "证书被拒绝",
            "证书被拒绝。猜测：客户端不信任 zniff CA，请安装并信任根证书。",
            "证书被拒绝。猜测：该客户端多半做了证书钉扎，不接受代理签发的证书。",
            "客户端 TLS 失败: received fatal alert CertificateUnknown",
            "源站证书不在内置根证书里，已改为隧道",
            "源站中止握手（handshake failure）",
            "Client TLS failed: unknown ca",
        ];
        for err in cases {
            assert_eq!(
                classify_error(None, err),
                Some(ErrorCategory::TlsTrust),
                "expected TlsTrust for: {err}"
            );
        }
    }

    #[test]
    fn classifies_network_dns_timeout_and_upstream_errors() {
        let cases = [
            "连接源站超时，已改为隧道",
            "连接上游代理。猜测：配置里的上游代理没有接通。",
            "上游 HTTP/1.1。猜测：源站端口没有服务在听。",
            "上游 HTTP/2。猜测：主机名没有解析到地址。",
            "connection refused (os error 61)",
            "dns error: failed to lookup address information: nodename nor servname provided",
            "request timed out",
        ];
        for err in cases {
            assert_eq!(
                classify_error(None, err),
                Some(ErrorCategory::NetworkUpstream),
                "expected NetworkUpstream for: {err}"
            );
        }
    }

    #[test]
    fn classifies_407_proxy_auth_even_without_error_text() {
        assert_eq!(
            classify_error(Some(407), ""),
            Some(ErrorCategory::ProxyAuth)
        );
        assert_eq!(
            classify_error(None, "HTTP 407 Proxy Authentication Required"),
            Some(ErrorCategory::ProxyAuth)
        );
        assert_eq!(
            classify_error(None, "代理认证失败"),
            Some(ErrorCategory::ProxyAuth)
        );
    }

    #[test]
    fn ignores_non_actionable_notices_and_normal_responses() {
        assert_eq!(classify_error(Some(200), ""), None);
        assert_eq!(classify_error(Some(500), ""), None);
        assert_eq!(classify_error(None, "直通名单，未解密"), None);
        assert_eq!(classify_error(None, "断点丢弃"), None);
        assert_eq!(classify_error(None, "规则阻断 (连接已断开)"), None);
        assert_eq!(classify_error(None, "不是 TLS，已改为隧道"), None);
    }

    #[test]
    fn generates_desktop_and_wasm_actions_per_category() {
        assert_eq!(
            actions_for_error(None, "证书被拒绝", "api.example.com", true),
            vec![
                ErrorAction::BypassTls,
                ErrorAction::InstallCa,
                ErrorAction::OpenInBrowser,
            ]
        );
        assert_eq!(
            actions_for_error_with_ca(None, "证书被拒绝", "api.example.com", true, true),
            vec![ErrorAction::BypassTls, ErrorAction::OpenInBrowser]
        );
        // On wasm32, desktop-only InstallCa and OpenInBrowser are hidden.
        assert_eq!(
            actions_for_error(None, "证书被拒绝", "api.example.com", false),
            vec![ErrorAction::BypassTls]
        );
        assert_eq!(
            actions_for_error(None, "证书被拒绝", "", false),
            Vec::<ErrorAction>::new()
        );
        assert_eq!(
            actions_for_error(None, "connection refused", "api.example.com", true),
            vec![ErrorAction::CheckUpstream, ErrorAction::AddResolve]
        );
        assert_eq!(
            actions_for_error(None, "connection refused", "", true),
            vec![ErrorAction::CheckUpstream]
        );
        assert_eq!(
            actions_for_error(Some(407), "", "api.example.com", false),
            vec![ErrorAction::OpenSecurity]
        );
    }

    #[test]
    fn host_extraction_browser_url_and_bypass_append_work() {
        assert_eq!(
            extract_host_from_url("https://API.Example.com:8443/v1/users?a=1"),
            "api.example.com"
        );
        assert_eq!(
            extract_host_from_url("connect.example.com:443"),
            "connect.example.com"
        );
        assert_eq!(extract_host_from_url("http://[::1]:8080/test"), "::1");

        assert_eq!(
            browser_url_for_flow("https://api.example.com/v1", "api.example.com"),
            Some("https://api.example.com/v1".to_string())
        );
        assert_eq!(
            browser_url_for_flow("api.example.com:443", "api.example.com"),
            Some("https://api.example.com".to_string())
        );

        assert_eq!(
            append_bypass_host("", "API.Example.com:443"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            append_bypass_host("foo.dev", "api.example.com"),
            Some("foo.dev | api.example.com".to_string())
        );
        assert_eq!(
            append_bypass_host("foo.dev | *.example.com", "api.example.com"),
            None
        );
        assert!(bypass_contains_host("*.example.com", "api.example.com"));
        assert!(!bypass_contains_host(
            "only:api.example.com",
            "api.example.com"
        ));
        assert_eq!(
            append_bypass_host("only:api.example.com | other.dev", "api.example.com"),
            Some("only: | other.dev".to_string())
        );
    }

    #[test]
    fn bilingual_labels_are_non_empty() {
        for action in [
            ErrorAction::BypassTls,
            ErrorAction::InstallCa,
            ErrorAction::OpenInBrowser,
            ErrorAction::CheckUpstream,
            ErrorAction::AddResolve,
            ErrorAction::OpenSecurity,
        ] {
            assert!(!action.label(Lang::Zh, false).is_empty());
            assert!(!action.label(Lang::En, false).is_empty());
        }
        assert_eq!(ErrorAction::BypassTls.label(Lang::Zh, true), "已在直通名单");
        assert_eq!(
            ErrorAction::BypassTls.label(Lang::En, true),
            "In bypass list"
        );
    }
}
