//! Batch TLS handshake failure warning banner shown at the top of the flow list
//! when multiple recent TLS/tunnel failures indicate an untrusted CA or pinning.

use std::collections::HashSet;

#[allow(unused_imports)]
use super::*;

/// Sliding time window used to detect a burst of TLS handshake failures.
pub(crate) const TLS_BURST_WINDOW: Duration = Duration::from_secs(60);

/// Minimum number of recent TLS handshake failures required to show the banner.
pub(crate) const TLS_BURST_THRESHOLD: usize = 3;

/// Summary of recent TLS/tunnel handshake failures in the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TlsFailureBurst {
    pub(crate) failure_count: usize,
    pub(crate) distinct_hosts: usize,
    pub(crate) latest_flow_id: u64,
}

impl TlsFailureBurst {
    /// Computes the recent TLS failure burst from session flows relative to `now`.
    pub(crate) fn from_flows<'a>(
        flows: impl IntoIterator<Item = &'a session::Flow>,
        now: SystemTime,
    ) -> Self {
        let mut failure_count = 0_usize;
        let mut latest_flow_id = 0_u64;
        let mut hosts: HashSet<String> = HashSet::new();

        for flow in flows {
            let Some(err) = flow.error.as_deref() else {
                continue;
            };
            if classify_error(flow.status, err) != Some(ErrorCategory::TlsTrust) {
                continue;
            }
            let ts = flow.finished_at.unwrap_or(flow.started);
            let within_window = match now.duration_since(ts) {
                Ok(age) => age <= TLS_BURST_WINDOW,
                Err(_) => true,
            };
            if !within_window {
                continue;
            }
            failure_count += 1;
            latest_flow_id = latest_flow_id.max(flow.id);
            let host = flow.host.trim();
            if !host.is_empty() {
                hosts.insert(host.to_ascii_lowercase());
            }
        }

        Self {
            failure_count,
            distinct_hosts: hosts.len(),
            latest_flow_id,
        }
    }
}

/// Returns true when the batch TLS failure banner should be visible.
pub(crate) fn should_show_tls_failure_banner(burst: TlsFailureBurst, dismissed: bool) -> bool {
    !dismissed && burst.failure_count >= TLS_BURST_THRESHOLD
}

/// Formats the localized diagnostic summary shown in the batch TLS failure banner.
pub(crate) fn format_tls_failure_banner_text(
    burst: TlsFailureBurst,
    ca_trusted: bool,
    lang: Lang,
) -> String {
    let n = burst.failure_count;
    let hosts = burst.distinct_hosts;
    if ca_trusted {
        match lang {
            Lang::Zh if hosts > 1 => format!(
                "检测到 {n} 次 TLS 握手失败（{hosts} 个域名），相关应用可能未信任系统 CA 或启用了证书钉扎"
            ),
            Lang::Zh => {
                format!("检测到 {n} 次 TLS 握手失败，相关应用可能未信任系统 CA 或启用了证书钉扎")
            }
            Lang::En if hosts > 1 => format!(
                "Detected {n} TLS handshake failures ({hosts} hosts) — app may not trust system CA or uses pinning"
            ),
            Lang::En => format!(
                "Detected {n} TLS handshake failures — app may not trust system CA or uses pinning"
            ),
        }
    } else {
        match lang {
            Lang::Zh if hosts > 1 => {
                format!("检测到 {n} 次 TLS 握手失败（{hosts} 个域名），可能未信任 CA")
            }
            Lang::Zh => format!("检测到 {n} 次 TLS 握手失败，可能未信任 CA"),
            Lang::En if hosts > 1 => {
                format!(
                    "Detected {n} TLS handshake failures ({hosts} hosts) — CA may not be trusted"
                )
            }
            Lang::En => format!("Detected {n} TLS handshake failures — CA may not be trusted"),
        }
    }
}

/// Reactive signals provided via context to render the TLS failure banner in `flow_list`.
#[derive(Clone)]
pub(crate) struct TlsFailureBannerSignals {
    pub(crate) burst: Signal<TlsFailureBurst>,
    pub(crate) dismissed: Signal<bool>,
    pub(crate) ca_status: Signal<ca::CaStatus>,
    pub(crate) controls: Arc<Controls>,
    pub(crate) selected: Signal<Option<u64>>,
    pub(crate) install_ca: Rc<dyn Fn()>,
}

/// Renders the batch TLS failure banner if `TlsFailureBannerSignals` was provided in context.
pub(crate) fn tls_failure_banner_from_context() -> impl IntoElement {
    let ctx = use_context::<TlsFailureBannerSignals>();
    let ctx_view = ctx.clone();
    when(
        move || {
            ctx.as_ref()
                .is_some_and(|s| should_show_tls_failure_banner(s.burst.get(), s.dismissed.get()))
        },
        move || match ctx_view.clone() {
            Some(signals) => tls_failure_banner_view(signals),
            None => view(),
        },
    )
}

fn tls_failure_banner_view(signals: TlsFailureBannerSignals) -> View {
    let burst = signals.burst;
    let dismissed = signals.dismissed;
    let ca_status = signals.ca_status;
    let langs = ui_langs();

    let message = view()
        .flex_1()
        .min_w(px(180.0))
        .flex_row()
        .items_center()
        .gap_2()
        .child(glyph_px(14.0, || Glyph::Alert, || color(status_warn_fg())))
        .child(
            text(move || -> SharedString {
                let ca_trusted = {
                    #[cfg(target_arch = "wasm32")]
                    {
                        let _ = ca_status.get();
                        false
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        ca_status.get() == ca::CaStatus::Trusted
                    }
                };
                format_tls_failure_banner_text(burst.get(), ca_trusted, ui_lang(langs)).into()
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(2)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(status_warn_fg()),
        );

    let actions = tls_banner_actions(&signals, dismissed);

    view()
        .w_full()
        .flex_shrink_0()
        .pl(px(LIST_INSET))
        .pr_3()
        .py_2()
        .bg(status_warn_bg())
        .border_b_1()
        .border_b_color(status_warn_fg())
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(message)
        .child(actions)
}

#[cfg(not(target_arch = "wasm32"))]
fn tls_banner_actions(
    signals: &TlsFailureBannerSignals,
    dismissed: Signal<bool>,
) -> impl IntoElement {
    let ca_status = signals.ca_status;
    let install_cb = signals.install_ca.clone();
    let browser_controls = signals.controls.clone();
    let selected = signals.selected;
    let langs = ui_langs();

    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(tls_banner_button(
            move || -> SharedString {
                let lang = ui_lang(langs);
                match ca_status.get() {
                    ca::CaStatus::InstalledUntrusted => t(lang, "信任证书", "Trust CA").into(),
                    ca::CaStatus::NotInstalled | ca::CaStatus::Trusted => {
                        t(lang, "安装证书", "Install CA").into()
                    }
                }
            },
            true,
            move || install_cb.as_ref()(),
        ))
        .child(tls_banner_button(
            tr("隔离浏览器", "Isolated Browser"),
            false,
            move || {
                let url = selected_browser_start_url(&browser_controls, selected.get());
                launch_browser_action(browser_controls.clone(), url);
            },
        ))
        .child(tls_banner_button(
            tr("忽略", "Dismiss"),
            false,
            move || {
                dismissed.set(true);
            },
        ))
}

#[cfg(target_arch = "wasm32")]
fn tls_banner_actions(
    _signals: &TlsFailureBannerSignals,
    dismissed: Signal<bool>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1p5()
        .child(tls_banner_button(
            tr("忽略", "Dismiss"),
            false,
            move || {
                dismissed.set(true);
            },
        ))
}

fn tls_banner_button(
    label: impl IntoReactive<SharedString>,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    nsh_base::button()
        .on_press(on_press)
        .h(px(ControlSize::Xs.height()))
        .px_2()
        .flex_row()
        .items_center()
        .rounded(px(RADIUS_SM))
        .border_1()
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .border_color(if primary {
            accent_blue()
        } else {
            border_control()
        })
        .bg(if primary { accent_blue() } else { bg_card() })
        .hover(move |s| {
            s.bg(if primary {
                accent_blue_hover()
            } else {
                bg_control()
            })
        })
        .active(move |s| {
            s.bg(if primary {
                accent_blue_active()
            } else {
                bg_control_active()
            })
        })
        .focus_visible(apply_focus_ring)
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if primary {
                    fg_on_accent()
                } else {
                    fg_secondary()
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{FlowDraft, Protocol, Session};

    fn add_tls_error(session: &mut Session, host: &str, started: SystemTime, msg: &str) -> u64 {
        let id = session.open(FlowDraft {
            protocol: Protocol::Tunnel,
            method: "CONNECT".into(),
            host: host.into(),
            path: String::new(),
            url: format!("https://{host}:443"),
            request_headers: vec![],
        });
        session.set_started(id, started);
        session.mark_finished_at(id, started);
        session.set_error(id, msg);
        id
    }

    #[test]
    fn tls_burst_counts_only_recent_tls_errors_and_distinct_hosts() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let recent = now - Duration::from_secs(10);
        let stale = now - Duration::from_secs(120);

        let mut session = Session::default();
        // Stale TLS error (outside 60s window) -> ignored
        add_tls_error(&mut session, "old.example.com", stale, "unknown ca");
        // Recent non-TLS network error -> ignored
        let net_id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "offline.example.com".into(),
            path: "/".into(),
            url: "http://offline.example.com/".into(),
            request_headers: vec![],
        });
        session.set_started(net_id, recent);
        session.set_error(net_id, "connection refused");
        session.mark_finished_at(net_id, recent);

        // Recent TLS errors across 2 distinct hosts
        add_tls_error(
            &mut session,
            "a.example.com",
            recent,
            "client TLS: unknown ca",
        );
        add_tls_error(
            &mut session,
            "A.EXAMPLE.COM",
            recent,
            "certificate rejected",
        );
        let last_id = add_tls_error(
            &mut session,
            "b.example.com",
            recent,
            "origin TLS handshake failed",
        );

        let burst = TlsFailureBurst::from_flows(session.iter(), now);
        assert_eq!(burst.failure_count, 3);
        assert_eq!(burst.distinct_hosts, 2);
        assert_eq!(burst.latest_flow_id, last_id);
        assert!(should_show_tls_failure_banner(burst, false));
        assert!(!should_show_tls_failure_banner(burst, true));
    }

    #[test]
    fn tls_burst_below_threshold_hides_banner() {
        let burst = TlsFailureBurst {
            failure_count: TLS_BURST_THRESHOLD - 1,
            distinct_hosts: 2,
            latest_flow_id: 2,
        };
        assert!(!should_show_tls_failure_banner(burst, false));
    }

    #[test]
    fn tls_banner_copy_adapts_to_ca_trust_and_language() {
        let burst = TlsFailureBurst {
            failure_count: 4,
            distinct_hosts: 3,
            latest_flow_id: 10,
        };
        let zh_untrusted = format_tls_failure_banner_text(burst, false, Lang::Zh);
        assert!(
            zh_untrusted.contains("4")
                && zh_untrusted.contains("3")
                && zh_untrusted.contains("未信任 CA")
        );
        let en_untrusted = format_tls_failure_banner_text(burst, false, Lang::En);
        assert!(en_untrusted.contains("4") && en_untrusted.contains("CA may not be trusted"));

        let zh_trusted = format_tls_failure_banner_text(burst, true, Lang::Zh);
        assert!(zh_trusted.contains("证书钉扎"));
        let en_trusted = format_tls_failure_banner_text(burst, true, Lang::En);
        assert!(en_trusted.contains("pinning"));
    }
}
