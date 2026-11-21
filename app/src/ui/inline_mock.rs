//! Inline Mock response editor in the detail view (E1): edit a flow's status,
//! headers, and body in place, save under `<data_dir>/mocks/`, and auto-append
//! `map-local` (+ modified `header`) rules so changes take effect immediately.

#[allow(unused_imports)]
use super::*;

use std::path::{Path, PathBuf};

/// Reactive state for the inline response mock editor attached to [`Compose`].
#[derive(Clone, Copy)]
pub(crate) struct InlineMockSignals {
    pub(crate) open: Signal<bool>,
    pub(crate) flow_id: Signal<u64>,
    pub(crate) host: Signal<String>,
    pub(crate) path: Signal<String>,
    pub(crate) status: Signal<String>,
    pub(crate) headers: Signal<String>,
    pub(crate) body: Signal<String>,
    pub(crate) original_headers: Signal<Vec<(String, String)>>,
}

impl InlineMockSignals {
    pub(crate) fn new() -> Self {
        Self {
            open: signal(false),
            flow_id: signal(0),
            host: signal(String::new()),
            path: signal(String::new()),
            status: signal(String::new()),
            headers: signal(String::new()),
            body: signal(String::new()),
            original_headers: signal(Vec::new()),
        }
    }

    pub(crate) fn close(self) {
        self.open.set(false);
    }
}

/// Headers managed automatically by the proxy transport that should not be
/// emitted as manual `header res set` overrides.
pub(crate) fn is_hop_by_hop_mock_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "content-length"
            | "transfer-encoding"
            | "content-encoding"
            | "connection"
            | "keep-alive"
            | "date"
    )
}

/// Formats editable response headers, excluding transport-managed hop-by-hop headers.
pub(crate) fn format_mock_headers(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .filter(|(k, _)| !is_hop_by_hop_mock_header(k))
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Formats a response body for inline editing, pretty-printing valid JSON when possible.
pub(crate) fn format_mock_body(headers: &[(String, String)], body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(body);
    let looks_json = present::content_type(headers)
        .is_some_and(|ct| ct.to_ascii_lowercase().contains("json"))
        || text.trim_start().starts_with('{')
        || text.trim_start().starts_with('[');
    if looks_json
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
        && let Ok(pretty) = serde_json::to_string_pretty(&val)
    {
        return pretty;
    }
    text.into_owned()
}

fn sanitize_path_segment(raw: &str, fallback: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            prev_underscore = false;
            ch.to_ascii_lowercase()
        } else {
            if prev_underscore {
                continue;
            }
            prev_underscore = true;
            '_'
        };
        out.push(mapped);
        if out.len() >= max_len {
            break;
        }
    }
    let trimmed = out.trim_matches(['_', '.']);
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn mock_body_ext(headers: &[(String, String)], body: &[u8]) -> &'static str {
    if let Some(ct) = present::content_type(headers) {
        let lower = ct.to_ascii_lowercase();
        if lower.contains("json") {
            return "json";
        }
        if lower.contains("html") {
            return "html";
        }
        if lower.contains("xml") {
            return "xml";
        }
        if lower.contains("javascript") || lower.contains("ecmascript") {
            return "js";
        }
        if lower.contains("css") {
            return "css";
        }
    }
    let trimmed = body.trim_ascii_start();
    if trimmed.starts_with(b"{") || trimmed.starts_with(b"[") {
        "json"
    } else {
        "txt"
    }
}

/// Computes the deterministic mock body file path under `<base_dir>/mocks/<host>/...`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn mock_file_path(
    base_dir: &Path,
    host: &str,
    path: &str,
    flow_id: u64,
    headers: &[(String, String)],
    body: &[u8],
) -> PathBuf {
    let safe_host = sanitize_path_segment(host, "host", 64);
    let clean_path = path.split('?').next().unwrap_or(path).trim_matches('/');
    let safe_path = sanitize_path_segment(clean_path, "root", 48);
    let ext = mock_body_ext(headers, body);
    base_dir
        .join("mocks")
        .join(safe_host)
        .join(format!("{safe_path}_{flow_id}.{ext}"))
}

fn mock_rule_pattern(host: &str, path: &str) -> String {
    let clean_path = path.split('?').next().unwrap_or(path).trim();
    let host = host.trim();
    if clean_path.is_empty() || clean_path == "/" {
        host.to_string()
    } else if clean_path.starts_with('/') {
        format!("{host}{clean_path}")
    } else {
        format!("{host}/{clean_path}")
    }
}

/// Appends a `map-local` rule plus any added/modified/removed response header rules.
pub(crate) fn build_inline_mock_rules(
    existing_rules: &str,
    host: &str,
    path: &str,
    saved_file: Option<&str>,
    status: u16,
    edited_headers: &[(String, String)],
    original_headers: &[(String, String)],
) -> Option<String> {
    let mut next =
        crate::rules::append_map_local_line(existing_rules, host, path, saved_file, Some(status))?;
    let pattern = crate::rules::quote_rule_token(&mock_rule_pattern(host, path));

    for (name, value) in edited_headers {
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() || is_hop_by_hop_mock_header(name) {
            continue;
        }
        let unchanged = original_headers
            .iter()
            .any(|(orig_k, orig_v)| orig_k.eq_ignore_ascii_case(name) && orig_v.trim() == value);
        if unchanged {
            continue;
        }
        let safe_name = crate::rules::quote_rule_token(name);
        let safe_val = crate::rules::quote_rule_token(value);
        next.push_str(&format!(
            "\nheader {pattern} res set {safe_name} {safe_val}"
        ));
    }

    for (orig_k, _) in original_headers {
        let orig_k = orig_k.trim();
        if orig_k.is_empty() || is_hop_by_hop_mock_header(orig_k) {
            continue;
        }
        let still_present = edited_headers
            .iter()
            .any(|(k, _)| k.trim().eq_ignore_ascii_case(orig_k));
        if !still_present {
            let safe_name = crate::rules::quote_rule_token(orig_k);
            next.push_str(&format!("\nheader {pattern} res del {safe_name}"));
        }
    }

    Some(next)
}

/// Opens the inline response mock editor prefilled from the selected flow.
pub(crate) fn begin_inline_mock(controls: &Arc<Controls>, id: Option<u64>, compose: Compose) {
    if refuse_relay(controls, id) {
        return;
    }
    let Some(flow) = selected_flow(controls, id) else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    compose.composing.set(false);
    let mock = compose.inline_mock;
    mock.flow_id.set(flow.id);
    mock.host.set(flow.host.clone());
    mock.path.set(flow.path.clone());
    mock.status.set(flow.status.unwrap_or(200).to_string());
    mock.headers
        .set(format_mock_headers(&flow.response_headers));
    mock.original_headers.set(flow.response_headers.clone());
    mock.body.set(format_mock_body(
        &flow.response_headers,
        &flow.response_body,
    ));
    mock.open.set(true);
}

/// Saves the edited mock response under `<data_dir>/mocks/` and appends rules.
pub(crate) fn save_inline_mock(
    controls: &Arc<Controls>,
    compose: Compose,
    profiles: ProfileSignals,
) {
    let mock = compose.inline_mock;
    let raw_status = mock.status.get_untracked();
    let Some(status) = parse_status_code(&raw_status) else {
        return;
    };
    let host = mock.host.get_untracked();
    let path = mock.path.get_untracked();
    let flow_id = mock.flow_id.get_untracked();
    let edited_headers = parse_header_text(&mock.headers.get_untracked());
    let original_headers = mock.original_headers.get_untracked();
    let body_bytes = mock.body.get_untracked().into_bytes();

    #[cfg(not(target_arch = "wasm32"))]
    let saved_file = {
        let target = mock_file_path(
            &ca::data_dir(),
            &host,
            &path,
            flow_id,
            &edited_headers,
            &body_bytes,
        );
        if let Some(parent) = target.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            set_status(controls, StatusMsg::SaveFailed(err.to_string()));
            return;
        }
        if let Err(err) = std::fs::write(&target, &body_bytes) {
            set_status(controls, StatusMsg::SaveFailed(err.to_string()));
            return;
        }
        Some(target.display().to_string())
    };
    #[cfg(target_arch = "wasm32")]
    let saved_file: Option<String> = {
        let _ = (flow_id, &body_bytes);
        None
    };

    let Some(next_rules) = build_inline_mock_rules(
        &profiles.rules.get_untracked(),
        &host,
        &path,
        saved_file.as_deref(),
        status,
        &edited_headers,
        &original_headers,
    ) else {
        return;
    };

    profiles.rules.set(next_rules.clone());
    controls.hub.set_rules(&next_rules);
    if !profiles.rules_on.get_untracked() {
        profiles.rules_on.set(true);
        controls.hub.set_rules_enabled(true);
    }
    save_config_or_notify(controls, PrefName::Rules, |cfg| {
        cfg.rules = next_rules;
    });
    let label = saved_file.unwrap_or_else(|| format!("{host}{path}"));
    set_status(controls, StatusMsg::MappedLocal(label));
    mock.close();
}

/// Compact one-click Mock trigger button shown beside `detail_actions` in the detail summary.
pub(crate) fn inline_mock_trigger_button(
    shown: Shown,
    controls: Arc<Controls>,
    selected: Signal<Option<u64>>,
    compose: Compose,
) -> impl IntoElement {
    when(
        move || shown.relay.get().is_empty(),
        move || {
            let controls = controls.clone();
            let is_open = move || compose.inline_mock.open.get();
            view()
                .h(px(26.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || {
                    if is_open() {
                        accent_blue_border()
                    } else {
                        border_control()
                    }
                })
                .bg(move || {
                    if is_open() {
                        accent_blue_soft()
                    } else {
                        bg_control()
                    }
                })
                .hover(|style| {
                    style.bg(move || {
                        if is_open() {
                            accent_blue_soft_hover()
                        } else {
                            bg_control_hover()
                        }
                    })
                })
                .active(|style| style.opacity(0.8))
                .cursor(Cursor::Pointer)
                .flex_row()
                .items_center()
                .tooltip(label_tip(
                    "内联编辑响应并生成 map-local 规则",
                    "Edit response inline & append map-local rule",
                ))
                .on_press(move || {
                    if compose.inline_mock.open.get_untracked() {
                        compose.inline_mock.close();
                    } else {
                        begin_inline_mock(&controls, selected.get(), compose);
                    }
                })
                .child(
                    text(tr("Mock", "Mock"))
                        .number_of_lines(1)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if is_open() {
                                accent_blue_text()
                            } else {
                                fg_secondary()
                            }
                        }),
                )
        },
    )
}

/// Inline Mock editor card rendered in the detail view right below the summary.
pub(crate) fn inline_mock_box(
    compose: Compose,
    controls: Arc<Controls>,
    profiles: ProfileSignals,
) -> impl IntoElement {
    let mock = compose.inline_mock;
    let save_controls = controls.clone();
    let status_invalid = move || parse_status_code(&mock.status.get()).is_none();

    view()
        .rounded(px(10.0))
        .border_1()
        .border_color(accent_blue_border())
        .bg(bg_card())
        .overflow_hidden()
        .child(inline_mock_header(
            compose,
            save_controls,
            profiles,
            status_invalid,
        ))
        .child(
            view()
                .p_3p5()
                .gap_2p5()
                .flex_col()
                .child(inline_mock_status_row(mock, status_invalid))
                .child(inline_mock_headers_editor(mock))
                .child(inline_mock_body_editor(mock)),
        )
}

fn inline_mock_header(
    compose: Compose,
    controls: Arc<Controls>,
    profiles: ProfileSignals,
    status_invalid: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let mock = compose.inline_mock;
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .px_3p5()
        .py_2()
        .border_b_1()
        .border_b_color(border_subtle())
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_2()
                .child(section_title(tr("内联 Mock 响应", "Inline Mock Response")))
                .child(
                    mono(move || {
                        SharedString::from(mock_rule_pattern(&mock.host.get(), &mock.path.get()))
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Middle)
                    .text_size(px(11.0))
                    .text_color(fg_muted()),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    view()
                        .h(px(26.0))
                        .px_2p5()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(border_control())
                        .bg(bg_control())
                        .hover(|style| style.bg(bg_control_hover()))
                        .active(|style| style.opacity(0.75))
                        .cursor(Cursor::Pointer)
                        .flex_row()
                        .items_center()
                        .on_press(move || mock.close())
                        .child(
                            text(tr("取消", "Cancel"))
                                .number_of_lines(1)
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg_secondary()),
                        ),
                )
                .child(
                    view()
                        .h(px(26.0))
                        .px_2p5()
                        .rounded(px(6.0))
                        .bg(move || {
                            if status_invalid() {
                                bg_control()
                            } else {
                                accent_blue()
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if status_invalid() {
                                    bg_control()
                                } else {
                                    accent_blue_hover()
                                }
                            })
                        })
                        .active(|style| {
                            style.opacity(move || if status_invalid() { 1.0_f32 } else { 0.85_f32 })
                        })
                        .cursor(Cursor::Pointer)
                        .disabled(status_invalid)
                        .disabled_style(|style| style.cursor(Cursor::Default))
                        .flex_row()
                        .items_center()
                        .on_press(move || {
                            save_inline_mock(&controls, compose, profiles);
                        })
                        .child(
                            text(tr("保存并生效", "Save & Apply"))
                                .number_of_lines(1)
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if status_invalid() {
                                        fg_muted()
                                    } else {
                                        AdaptiveColor::rgb(0xffffff, 0xffffff)
                                    }
                                }),
                        ),
                ),
        )
}

fn inline_mock_status_row(
    mock: InlineMockSignals,
    status_invalid: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            text(tr("状态码", "Status"))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .w(px(88.0))
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || {
                    if status_invalid() {
                        status_warn_fg()
                    } else {
                        border_control()
                    }
                })
                .bg(bg_code())
                .items_center()
                .justify_center()
                .child(
                    input()
                        .value(move || mock.status.get())
                        .placeholder("200")
                        .on_change(move |text: &str| {
                            mock.status.set(text.to_string());
                        })
                        .w_full(),
                ),
        )
        .child(
            text(tr(
                "保存到数据目录 mocks/ 并自动追加 map-local 规则",
                "Saves to <data_dir>/mocks/ and appends a map-local rule",
            ))
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail)
            .text_size(px(11.0))
            .text_color(fg_muted()),
        )
}

fn inline_mock_headers_editor(mock: InlineMockSignals) -> impl IntoElement {
    view()
        .w_full()
        .gap_1()
        .flex_col()
        .child(
            text(tr("响应头 (Name: Value)", "Response headers (Name: Value)"))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
        .child(
            guard_culled_pointer_events(view())
                .w_full()
                .h(px(96.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(border_control())
                .bg(bg_code())
                .overflow_hidden()
                .child(
                    editor()
                        .value(move || SharedString::from(mock.headers.get()))
                        .language("plaintext")
                        .line_numbers(false)
                        .folding(false)
                        .soft_wrap(true)
                        .font_size(Points(12.0))
                        .line_height(Points(18.0))
                        .scroll_beyond_last_line(false)
                        .on_change(move |text: &SharedString| {
                            mock.headers.set(text.to_string());
                        })
                        .size_full(),
                ),
        )
}

fn inline_mock_body_editor(mock: InlineMockSignals) -> impl IntoElement {
    view()
        .w_full()
        .gap_1()
        .flex_col()
        .child(
            text(tr("响应正文", "Response body"))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_secondary()),
        )
        .child(
            guard_culled_pointer_events(view())
                .w_full()
                .h(px(180.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(border_control())
                .bg(bg_code())
                .overflow_hidden()
                .child(
                    editor()
                        .value(move || SharedString::from(mock.body.get()))
                        .language("json")
                        .line_numbers(true)
                        .folding(false)
                        .soft_wrap(true)
                        .font_size(Points(12.0))
                        .line_height(Points(18.0))
                        .scroll_beyond_last_line(false)
                        .on_change(move |text: &SharedString| {
                            mock.body.set(text.to_string());
                        })
                        .size_full(),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_file_path_sanitizes_host_path_and_picks_extension() {
        let base = Path::new("/tmp/zniff-data");
        let headers = vec![(
            "Content-Type".into(),
            "application/json; charset=utf-8".into(),
        )];
        let path = mock_file_path(
            base,
            "API.Example.com:8443",
            "/v1/users/profile?tab=1",
            42,
            &headers,
            br#"{"ok":true}"#,
        );
        assert_eq!(
            path,
            PathBuf::from("/tmp/zniff-data/mocks/api.example.com_8443/v1_users_profile_42.json")
        );
    }

    #[test]
    fn build_inline_mock_rules_emits_valid_map_local_and_header_diffs() {
        let orig = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Old".into(), "1".into()),
            ("Content-Length".into(), "12".into()),
        ];
        let edited = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Mocked".into(), "true".into()),
            ("Cache-Control".into(), "no-store, max-age=0".into()),
        ];
        let rules = build_inline_mock_rules(
            "cors *",
            "api.example.com",
            "/v1/items?page=2",
            Some("/tmp/zniff/mocks/api.example.com/v1_items_7.json"),
            201,
            &edited,
            &orig,
        )
        .expect("rules generated");

        assert!(
            crate::rules::invalid_rule_lines(&rules).is_empty(),
            "{rules}"
        );
        let parsed = crate::rules::parse_rules(&rules);
        // cors + map-local + 2 set headers (X-Mocked, Cache-Control) + 1 del header (X-Old)
        assert_eq!(parsed.len(), 5, "parsed: {parsed:?}\nrules:\n{rules}");
        assert!(rules.contains("map-local api.example.com/v1/items"));
        assert!(rules.contains("status=201"));
        assert!(rules.contains("header api.example.com/v1/items res set X-Mocked true"));
        assert!(rules.contains("header api.example.com/v1/items res del X-Old"));
    }

    #[test]
    fn format_mock_body_pretty_prints_json_and_preserves_plain_text() {
        let json_headers = vec![("Content-Type".into(), "application/json".into())];
        let pretty = format_mock_body(&json_headers, br#"{"a":1,"b":[2,3]}"#);
        assert!(pretty.contains('\n') && pretty.contains("\"a\": 1"));

        let text_headers = vec![("Content-Type".into(), "text/plain".into())];
        assert_eq!(
            format_mock_body(&text_headers, b"hello world"),
            "hello world"
        );
    }

    #[test]
    fn format_mock_headers_omits_hop_by_hop_headers() {
        let headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Content-Length".into(), "99".into()),
            ("Transfer-Encoding".into(), "chunked".into()),
            ("X-Custom".into(), "abc".into()),
        ];
        let text = format_mock_headers(&headers);
        assert_eq!(text, "Content-Type: application/json\nX-Custom: abc");
    }
}
