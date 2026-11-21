//! Redacted diagnostics bundle export for bug reports and troubleshooting.
//!
//! Strips credentials, tokens, rule bodies, and query parameters so the
//! resulting JSON file can be safely attached to GitHub issues.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::config::Config;
use crate::i18n::Lang;
use crate::session::{Flow, Session};

const SCHEMA_VERSION: u32 = 1;
const MAX_RECENT_ERRORS: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundle {
    pub schema_version: u32,
    pub generated_at_unix_secs: u64,
    pub app: AppDiagnostics,
    pub runtime: RuntimeDiagnostics,
    pub ca: CaDiagnostics,
    pub config: RedactedConfigSummary,
    pub error_counts_by_bucket: BTreeMap<String, usize>,
    pub recent_errors: Vec<ErrorSummary>,
}

impl DiagnosticsBundle {
    pub fn to_pretty_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AppDiagnostics {
    pub name: &'static str,
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub target: &'static str,
    pub dev_mode: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDiagnostics {
    pub capturing: bool,
    pub listen_addr: String,
    pub use_system_proxy: bool,
    pub total_flows: usize,
    pub dropped_flows: u64,
    pub memory_bytes: usize,
    pub config_load_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaDiagnostics {
    pub status: String,
    pub active_external_ca: Option<String>,
    pub detected_external_cas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RedactedConfigSummary {
    pub listen_port: u16,
    pub listen_all: bool,
    pub use_system_proxy: bool,
    pub language: Option<String>,
    pub theme_mode: String,
    pub ui_zoom: u16,
    pub layout_mode: String,
    pub rules_enabled: bool,
    pub breaks_enabled: bool,
    pub show_tunnel_failures: bool,
    pub redact_export: bool,
    pub redact_strict: bool,
    pub external_ca: String,
    pub has_upstream_proxy: bool,
    pub upstream_proxy_redacted: Option<String>,
    pub has_proxy_auth: bool,
    pub has_web_auth: bool,
    pub has_relay_url: bool,
    pub relay_url_redacted: Option<String>,
    pub has_relay_token: bool,
    pub has_relay_view_auth: bool,
    pub relay_accept: bool,
    pub allow_hosts_count: usize,
    pub bypass_hosts_count: usize,
    pub rules_line_count: usize,
    pub rule_groups_count: usize,
    pub saved_profiles_count: usize,
    pub compose_history_count: usize,
}

impl RedactedConfigSummary {
    pub fn from_config(cfg: &Config) -> Self {
        let upstream = cfg.upstream_proxy.trim();
        let relay = cfg.relay_url.trim();
        Self {
            listen_port: cfg.listen_port,
            listen_all: cfg.listen_all,
            use_system_proxy: cfg.use_system_proxy,
            language: cfg.language.clone(),
            theme_mode: format!("{:?}", cfg.theme_mode),
            ui_zoom: cfg.ui_zoom,
            layout_mode: format!("{:?}", cfg.layout_mode),
            rules_enabled: cfg.rules_enabled,
            breaks_enabled: cfg.breaks_enabled,
            show_tunnel_failures: cfg.show_tunnel_failures,
            redact_export: cfg.redact_export,
            redact_strict: cfg.redact_strict,
            external_ca: cfg.external_ca.clone(),
            has_upstream_proxy: !upstream.is_empty(),
            upstream_proxy_redacted: (!upstream.is_empty())
                .then(|| redact_url_credentials(upstream)),
            has_proxy_auth: !cfg.proxy_auth.trim().is_empty(),
            has_web_auth: !cfg.web_auth.trim().is_empty(),
            has_relay_url: !relay.is_empty(),
            relay_url_redacted: (!relay.is_empty()).then(|| redact_url_credentials(relay)),
            has_relay_token: !cfg.relay_token.trim().is_empty(),
            has_relay_view_auth: !cfg.relay_view_auth.trim().is_empty(),
            relay_accept: cfg.relay_accept,
            allow_hosts_count: non_empty_non_comment_lines(&cfg.allow_hosts),
            bypass_hosts_count: non_empty_non_comment_lines(&cfg.bypass_hosts),
            rules_line_count: non_empty_non_comment_lines(&cfg.rules),
            rule_groups_count: cfg.rule_groups.len(),
            saved_profiles_count: cfg.saved_profiles.len(),
            compose_history_count: cfg.compose_history.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ErrorSummary {
    pub flow_id: u64,
    pub protocol: String,
    pub method: String,
    pub host: String,
    pub path_redacted: String,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
}

pub struct DiagnosticsInput<'a> {
    pub generated_at_unix_secs: u64,
    pub dev_mode: bool,
    pub capturing: bool,
    pub listen_addr: String,
    pub use_system_proxy: bool,
    pub config_load_error: Option<String>,
    pub ca: CaDiagnostics,
    pub config: &'a Config,
    pub session: &'a Session,
}

/// Builds a redacted diagnostics bundle from the current runtime state, config, and session.
pub fn build_diagnostics_bundle(input: DiagnosticsInput<'_>) -> DiagnosticsBundle {
    let mut error_counts_by_bucket: BTreeMap<String, usize> = BTreeMap::new();
    let mut all_errors: Vec<&Flow> = Vec::new();

    for flow in input.session.iter() {
        let bucket = if flow.error.is_some() {
            Some("ERR")
        } else if let Some(code) = flow.status {
            if code >= 500 {
                Some("5xx")
            } else if code >= 400 {
                Some("4xx")
            } else {
                None
            }
        } else {
            None
        };
        if let Some(bucket) = bucket {
            *error_counts_by_bucket
                .entry(bucket.to_string())
                .or_insert(0) += 1;
            all_errors.push(flow);
        }
    }

    let recent_errors: Vec<ErrorSummary> = all_errors
        .into_iter()
        .rev()
        .take(MAX_RECENT_ERRORS)
        .map(|flow| ErrorSummary {
            flow_id: flow.id,
            protocol: flow.protocol.label_in(Lang::En).to_string(),
            method: flow.method.clone(),
            host: flow.host.clone(),
            path_redacted: redact_path_query(&flow.path),
            status: flow.status,
            error: flow.error.as_deref().map(redact_error_message),
            elapsed_ms: flow.elapsed_ms(),
        })
        .collect();

    DiagnosticsBundle {
        schema_version: SCHEMA_VERSION,
        generated_at_unix_secs: input.generated_at_unix_secs,
        app: AppDiagnostics {
            name: "zniff",
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            target: if cfg!(target_arch = "wasm32") {
                "wasm32"
            } else {
                "desktop"
            },
            dev_mode: input.dev_mode,
        },
        runtime: RuntimeDiagnostics {
            capturing: input.capturing,
            listen_addr: input.listen_addr,
            use_system_proxy: input.use_system_proxy,
            total_flows: input.session.len(),
            dropped_flows: input.session.dropped(),
            memory_bytes: input.session.memory_bytes(),
            config_load_error: input.config_load_error.as_deref().map(redact_error_message),
        },
        ca: input.ca,
        config: RedactedConfigSummary::from_config(input.config),
        error_counts_by_bucket,
        recent_errors,
    }
}

/// Redacts `user:pass@` credentials and query/fragment strings from a URL.
pub fn redact_url_credentials(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let Some(scheme_end) = without_query.find("://") else {
        return without_query.to_string();
    };
    let after_scheme = &without_query[scheme_end + 3..];
    let authority_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let Some(at_pos) = authority.rfind('@') else {
        return without_query.to_string();
    };
    let host_part = &authority[at_pos + 1..];
    let rest = &after_scheme[authority_end..];
    format!(
        "{}://[REDACTED]@{host_part}{rest}",
        &without_query[..scheme_end]
    )
}

/// Replaces any query string or fragment on a request path with `?[REDACTED]`.
pub fn redact_path_query(path: &str) -> String {
    let base = path.split('#').next().unwrap_or(path);
    match base.split_once('?') {
        Some((p, q)) if !q.is_empty() => format!("{p}?[REDACTED]"),
        Some((p, _)) => p.to_string(),
        None => base.to_string(),
    }
}

/// Redacts embedded credentials or authorization tokens inside error messages.
pub fn redact_error_message(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.split_whitespace().peekable();
    let mut first = true;
    while let Some(word) = chars.next() {
        if !first {
            out.push(' ');
        }
        first = false;
        if word.contains("://") && word.contains('@') {
            out.push_str(&redact_url_credentials(word));
        } else if word.eq_ignore_ascii_case("bearer") || word.eq_ignore_ascii_case("basic") {
            out.push_str(word);
            if chars.peek().is_some() {
                let _ = chars.next();
                out.push_str(" [REDACTED]");
            }
        } else {
            out.push_str(word);
        }
    }
    out
}

fn non_empty_non_comment_lines(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol};

    #[test]
    fn redacts_url_credentials_and_query_params() {
        assert_eq!(
            redact_url_credentials("http://alice:s3cr3t@proxy.corp:8080/path?token=xyz"),
            "http://[REDACTED]@proxy.corp:8080/path"
        );
        assert_eq!(
            redact_url_credentials("socks5://127.0.0.1:1080"),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            redact_path_query("/api/v1/login?api_key=abc&user=bob#frag"),
            "/api/v1/login?[REDACTED]"
        );
        assert_eq!(redact_path_query("/health"), "/health");
    }

    #[test]
    fn redacts_error_message_tokens_and_urls() {
        let msg = "upstream http://bob:pw123@10.0.0.1:8080 rejected Bearer eyJhbGciOiJIUzI1NiJ9";
        let clean = redact_error_message(msg);
        assert!(!clean.contains("pw123"));
        assert!(!clean.contains("eyJhbGci"));
        assert!(clean.contains("http://[REDACTED]@10.0.0.1:8080"));
        assert!(clean.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn config_summary_strips_all_secrets() {
        let cfg = Config {
            proxy_auth: "admin:supersecret".into(),
            web_auth: "web:topsecret".into(),
            relay_url: "https://relayuser:relaypass@relay.example.com/hook?key=1".into(),
            relay_token: "tok_live_999999".into(),
            relay_view_auth: "viewer:viewpass".into(),
            upstream_proxy: "http://u:p@corp-proxy:3128".into(),
            rules: "# comment\napi.example.com header req Authorization: Bearer secret123".into(),
            bypass_hosts: "internal.corp\n# comment\n*.local".into(),
            allow_hosts: "api.example.com".into(),
            ..Config::default()
        };

        let summary = RedactedConfigSummary::from_config(&cfg);
        let json = serde_json::to_string(&summary).unwrap();
        for leaked in [
            "supersecret",
            "topsecret",
            "relaypass",
            "tok_live_999999",
            "viewpass",
            "secret123",
            "u:p@",
            "internal.corp",
        ] {
            assert!(
                !json.contains(leaked),
                "secret leaked into diagnostics: {leaked}"
            );
        }
        assert!(summary.has_proxy_auth);
        assert!(summary.has_web_auth);
        assert!(summary.has_relay_token);
        assert!(summary.has_relay_view_auth);
        assert_eq!(
            summary.upstream_proxy_redacted.as_deref(),
            Some("http://[REDACTED]@corp-proxy:3128")
        );
        assert_eq!(
            summary.relay_url_redacted.as_deref(),
            Some("https://[REDACTED]@relay.example.com/hook")
        );
        assert_eq!(summary.rules_line_count, 1);
        assert_eq!(summary.bypass_hosts_count, 2);
        assert_eq!(summary.allow_hosts_count, 1);
    }

    #[test]
    fn bundle_collects_recent_errors_newest_first_and_redacts_queries() {
        let mut session = Session::default();
        // OK 200 flow -> excluded from recent_errors
        let ok_id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "ok.example.com".into(),
            path: "/ok".into(),
            url: "https://ok.example.com/ok".into(),
            request_headers: vec![],
        });
        session.set_response(ok_id, 200, vec![], None);

        // 502 error flow
        let s502_id = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/pay?secret=xyz".into(),
            url: "https://api.example.com/v1/pay?secret=xyz".into(),
            request_headers: vec![],
        });
        session.set_response(s502_id, 502, vec![], None);

        // TLS error flow
        let err_id = session.open(FlowDraft {
            protocol: Protocol::Tunnel,
            method: "CONNECT".into(),
            host: "pinned.example.com".into(),
            path: String::new(),
            url: "https://pinned.example.com:443".into(),
            request_headers: vec![],
        });
        session.set_error(err_id, "client TLS handshake failed: unknown ca");

        let cfg = Config::default();
        let bundle = build_diagnostics_bundle(DiagnosticsInput {
            generated_at_unix_secs: 1_700_000_000,
            dev_mode: false,
            capturing: true,
            listen_addr: "127.0.0.1:8877".into(),
            use_system_proxy: false,
            config_load_error: None,
            ca: CaDiagnostics {
                status: "trusted".into(),
                active_external_ca: None,
                detected_external_cas: vec![],
            },
            config: &cfg,
            session: &session,
        });

        assert_eq!(bundle.runtime.total_flows, 3);
        assert_eq!(bundle.error_counts_by_bucket.get("ERR"), Some(&1));
        assert_eq!(bundle.error_counts_by_bucket.get("5xx"), Some(&1));
        assert_eq!(bundle.recent_errors.len(), 2);
        // Newest first
        assert_eq!(bundle.recent_errors[0].flow_id, err_id);
        assert_eq!(bundle.recent_errors[1].flow_id, s502_id);
        assert_eq!(bundle.recent_errors[1].path_redacted, "/v1/pay?[REDACTED]");

        let json = bundle.to_pretty_json().unwrap();
        assert!(!json.contains("secret=xyz"));
        assert!(json.contains("\"schema_version\": 1"));
    }
}
