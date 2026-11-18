//! Batch local Mock directory exporter with auto-generated `map-local` rules.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::redact::{RedactOptions, redact_flow};
use crate::rules;
use crate::session::{Flow, Protocol};

/// One host entry exported to disk along with its local directory root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockHostRule {
    pub host: String,
    pub host_dir: PathBuf,
}

/// Summary returned after exporting captured flows into a local Mock folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockExportSummary {
    pub root_dir: PathBuf,
    pub file_count: usize,
    pub host_rules: Vec<MockHostRule>,
}

fn sanitize_host_dir(host: &str) -> String {
    let clean: String = host
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." {
        "default-host".to_string()
    } else {
        clean
    }
}

fn sanitize_rel_segment(seg: &str) -> String {
    let clean: String = seg
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => ch,
        })
        .collect();
    if clean.is_empty() || clean == "." || clean == ".." {
        "_".to_string()
    } else {
        clean
    }
}

fn content_type_header(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
}

fn infer_file_rel_path(raw_path: &str, ct: Option<&str>, body: &[u8]) -> PathBuf {
    let clean = raw_path.split(['?', '#']).next().unwrap_or("/");
    let trailing_slash = clean.ends_with('/');
    let mut segs: Vec<String> = clean
        .split('/')
        .filter(|s| !s.is_empty())
        .map(sanitize_rel_segment)
        .collect();

    let is_json = ct.is_some_and(|c| c.to_ascii_lowercase().contains("json"))
        || serde_json::from_slice::<serde_json::Value>(body).is_ok();
    let is_html = ct.is_some_and(|c| c.to_ascii_lowercase().contains("html"));

    if segs.is_empty() || trailing_slash {
        let leaf = if is_html { "index.html" } else { "index.json" };
        segs.push(leaf.to_string());
    } else if let Some(last) = segs.last_mut()
        && Path::new(last).extension().is_none()
    {
        if is_json {
            last.push_str(".json");
        } else if is_html {
            last.push_str(".html");
        } else {
            last.push_str(".bin");
        }
    }

    let mut out = PathBuf::new();
    for seg in segs {
        out.push(seg);
    }
    out
}

/// Writes matched HTTP responses into `<root_dir>/<host>/<path>` so they can be
/// served offline via `map-local <host> "<root_dir>/<host>" fallback`.
pub fn export_mock_directory(
    flows: &[Flow],
    root_dir: &Path,
    redact: &RedactOptions,
) -> Result<MockExportSummary> {
    fs::create_dir_all(root_dir)
        .with_context(|| format!("创建 Mock 目录 {}", root_dir.display()))?;

    let mut written: BTreeSet<PathBuf> = BTreeSet::new();
    let mut hosts: BTreeMap<String, PathBuf> = BTreeMap::new();

    for raw_flow in flows {
        if raw_flow.protocol == Protocol::Tunnel
            || raw_flow.method.eq_ignore_ascii_case("CONNECT")
            || raw_flow.status.is_none()
            || (!raw_flow.url.starts_with("http://") && !raw_flow.url.starts_with("https://"))
        {
            continue;
        }
        let owned;
        let flow = if redact.enabled {
            owned = redact_flow(raw_flow, redact).0;
            &owned
        } else {
            raw_flow
        };

        let host_dir = root_dir.join(sanitize_host_dir(&flow.host));
        let ct = content_type_header(&flow.response_headers);
        let rel = infer_file_rel_path(&flow.path, ct, &flow.response_body);
        let target = host_dir.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建 Mock 子目录 {}", parent.display()))?;
        }
        let content: Vec<u8> = if ct.is_some_and(|c| c.to_ascii_lowercase().contains("json"))
            && let Ok(val) = serde_json::from_slice::<serde_json::Value>(&flow.response_body)
            && let Ok(pretty) = serde_json::to_vec_pretty(&val)
        {
            pretty
        } else {
            flow.response_body.clone()
        };
        fs::write(&target, content)
            .with_context(|| format!("写入 Mock 文件 {}", target.display()))?;
        written.insert(target);
        hosts.entry(flow.host.clone()).or_insert(host_dir);
    }

    let host_rules = hosts
        .into_iter()
        .map(|(host, host_dir)| MockHostRule { host, host_dir })
        .collect();
    Ok(MockExportSummary {
        root_dir: root_dir.to_path_buf(),
        file_count: written.len(),
        host_rules,
    })
}

fn quote_rule_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Appends `map-local <host> "<dir>" fallback` rules that are not yet present in `existing_rules`.
pub fn append_mock_rules(existing_rules: &str, entries: &[MockHostRule]) -> String {
    let mut lines: Vec<String> = existing_rules.lines().map(str::to_string).collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let existing_parsed = rules::parse_rules(existing_rules);
    for entry in entries {
        let already = existing_parsed.iter().any(|rule| {
            matches!(
                rule,
                rules::Rule::MapLocal {
                    host,
                    path_prefix,
                    file: Some(existing_dir),
                    ..
                } if host.eq_ignore_ascii_case(&entry.host)
                    && path_prefix == "/"
                    && existing_dir == &entry.host_dir
            )
        });
        if !already {
            lines.push(format!(
                "map-local {} {} fallback",
                entry.host,
                quote_rule_path(&entry.host_dir)
            ));
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Direction, FlowDraft, Session};

    fn mock_flow(host: &str, path: &str, body: &[u8]) -> Flow {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: host.into(),
            path: path.into(),
            url: format!("https://{host}{path}"),
            request_headers: Vec::new(),
        });
        session.set_response(
            id,
            200,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        session.append_body(id, Direction::Response, body);
        session.mark_finished(id);
        session.hydrated(id).unwrap()
    }

    #[test]
    fn exports_mock_directory_with_map_local_json_resolution_roundtrip() {
        let dir = std::env::temp_dir().join(format!("zniff-mock-gen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let flows = vec![
            mock_flow("api.vicanso.dev", "/", br#"{"root":true}"#),
            mock_flow("api.vicanso.dev", "/v1/users?page=1", br#"{"users":["a"]}"#),
        ];
        let summary = export_mock_directory(&flows, &dir, &RedactOptions::default()).unwrap();
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.host_rules.len(), 1);
        let host_dir = &summary.host_rules[0].host_dir;

        // Verify `resolve_map_local_target` serves both `/` (`index.json`) and `/v1/users` (`users.json`).
        let root_hit = rules::resolve_map_local_target(Some(host_dir), "/", "/").unwrap();
        assert_eq!(root_hit.unwrap().file_name().unwrap(), "index.json");

        let users_hit =
            rules::resolve_map_local_target(Some(host_dir), "/", "/v1/users?page=2").unwrap();
        assert_eq!(users_hit.unwrap().file_name().unwrap(), "users.json");

        let next_rules = append_mock_rules("cors api.vicanso.dev\n", &summary.host_rules);
        assert!(next_rules.contains("map-local api.vicanso.dev"));
        assert!(next_rules.contains("fallback"));
        // Second append deduplicates identical map-local rule.
        assert_eq!(
            append_mock_rules(&next_rules, &summary.host_rules),
            next_rules
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
