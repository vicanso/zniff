//! Exports filtered HTTP(S) flows as API collections:
//! - Postman Collection v2.1 JSON (`.postman_collection.json`)
//! - Bruno (`.bru` text bundle)
//! - `.http` (VS Code REST Client / JetBrains HTTP Client)

use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::present::{content_type, query_pairs};
use crate::redact::{RedactOptions, redact_flow};
use crate::session::{Flow, Protocol};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionFormat {
    Postman,
    Bruno,
    Http,
}

impl CollectionFormat {
    pub const fn file_suffix(self) -> &'static str {
        match self {
            Self::Postman => "postman_collection.json",
            Self::Bruno => "bruno.bru",
            Self::Http => "http",
        }
    }

    pub const fn filter_label(self) -> &'static str {
        match self {
            Self::Postman => "Postman Collection JSON",
            Self::Bruno => "Bruno Collection (.bru)",
            Self::Http => "HTTP Client (.http)",
        }
    }

    pub const fn filter_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Postman => &["json"],
            Self::Bruno => &["bru"],
            Self::Http => &["http", "rest"],
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub const fn web_endpoint(self) -> &'static str {
        match self {
            Self::Postman => "api/export.postman",
            Self::Bruno => "api/export.bruno",
            Self::Http => "api/export.http",
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub const fn web_filename(self) -> &'static str {
        match self {
            Self::Postman => "zniff.postman_collection.json",
            Self::Bruno => "zniff.bruno.bru",
            Self::Http => "zniff.http",
        }
    }

    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Postman => "application/json; charset=utf-8",
            Self::Bruno => "text/plain; charset=utf-8",
            Self::Http => "text/plain; charset=utf-8",
        }
    }
}

/// Builds the exported collection text for the requested [`CollectionFormat`].
pub fn build_collection(
    flows: &[Flow],
    redact_opts: &RedactOptions,
    format: CollectionFormat,
) -> String {
    match format {
        CollectionFormat::Postman => build_postman_collection(flows, redact_opts),
        CollectionFormat::Bruno => build_bruno_bundle(flows, redact_opts),
        CollectionFormat::Http => build_http_file(flows, redact_opts),
    }
}

fn exportable_flows(flows: &[Flow], redact_opts: &RedactOptions) -> Vec<Flow> {
    flows
        .iter()
        .filter(|flow| {
            flow.protocol != Protocol::Tunnel
                && !flow.method.eq_ignore_ascii_case("CONNECT")
                && (flow.url.starts_with("http://") || flow.url.starts_with("https://"))
        })
        .map(|flow| {
            if redact_opts.enabled {
                redact_flow(flow, redact_opts).0
            } else {
                flow.clone()
            }
        })
        .collect()
}

fn export_headers(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("host")
                && !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("connection")
                && !name.eq_ignore_ascii_case("proxy-connection")
                && !name.eq_ignore_ascii_case("proxy-authorization")
        })
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

fn request_label(flow: &Flow) -> String {
    let path_only = flow.path.split('?').next().unwrap_or(&flow.path);
    let target = if path_only.is_empty() { "/" } else { path_only };
    format!("{} {target}", flow.method.to_ascii_uppercase())
}

fn parse_url_parts(url: &str) -> (String, Vec<String>, Option<String>, Vec<String>) {
    let (protocol, rest) = url
        .split_once("://")
        .map(|(p, r)| (p.to_ascii_lowercase(), r))
        .unwrap_or_else(|| ("https".to_string(), url));
    let no_frag = rest.split('#').next().unwrap_or(rest);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let (authority, path_str) = match no_query.find('/') {
        Some(idx) => (&no_query[..idx], &no_query[idx + 1..]),
        None => (no_query, ""),
    };
    let (host_str, port) = if let Some(bracket_end) = authority.rfind(']') {
        let host = &authority[..=bracket_end];
        let port = authority[bracket_end + 1..]
            .strip_prefix(':')
            .filter(|p| !p.is_empty())
            .map(ToString::to_string);
        (host, port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            (h, Some(p.to_string()))
        } else {
            (authority, None)
        }
    } else {
        (authority, None)
    };
    let host: Vec<String> = host_str
        .split('.')
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();
    let path: Vec<String> = path_str
        .split('/')
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();
    (protocol, host, port, path)
}

fn raw_language(ct: Option<&str>) -> &'static str {
    let lower = ct.unwrap_or("").to_ascii_lowercase();
    if lower.contains("json") || lower.contains("graphql") {
        "json"
    } else if lower.contains("xml") {
        "xml"
    } else if lower.contains("html") {
        "html"
    } else if lower.contains("javascript") {
        "javascript"
    } else {
        "text"
    }
}

/// Builds a Postman Collection v2.1 JSON document from filtered flows.
pub fn build_postman_collection(flows: &[Flow], redact_opts: &RedactOptions) -> String {
    let items: Vec<Value> = exportable_flows(flows, redact_opts)
        .iter()
        .map(postman_item)
        .collect();
    let doc = json!({
        "info": {
            "name": "zniff Export",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
            "_exporter_id": "zniff"
        },
        "item": items
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".into())
}

fn postman_item(flow: &Flow) -> Value {
    let headers: Vec<Value> = export_headers(&flow.request_headers)
        .into_iter()
        .map(|(k, v)| json!({ "key": k, "value": v }))
        .collect();

    let (protocol, host, port, path) = parse_url_parts(&flow.url);
    let mut url_obj = serde_json::Map::new();
    url_obj.insert("raw".into(), Value::String(flow.url.clone()));
    url_obj.insert("protocol".into(), Value::String(protocol));
    url_obj.insert("host".into(), json!(host));
    if let Some(p) = port {
        url_obj.insert("port".into(), Value::String(p));
    }
    if !path.is_empty() {
        url_obj.insert("path".into(), json!(path));
    }
    let query: Vec<Value> = query_pairs(&flow.url)
        .into_iter()
        .map(|(k, v)| json!({ "key": k, "value": v }))
        .collect();
    if !query.is_empty() {
        url_obj.insert("query".into(), Value::Array(query));
    }

    let mut req = serde_json::Map::new();
    req.insert(
        "method".into(),
        Value::String(flow.method.to_ascii_uppercase()),
    );
    req.insert("header".into(), Value::Array(headers));
    req.insert("url".into(), Value::Object(url_obj));

    if let Ok(body_text) = std::str::from_utf8(&flow.request_body)
        && !body_text.is_empty()
    {
        let ct = content_type(&flow.request_headers);
        let is_form = ct.is_some_and(|c| c.to_ascii_lowercase().contains("x-www-form-urlencoded"));
        if is_form {
            let pairs: Vec<Value> = query_pairs(&format!("?{body_text}"))
                .into_iter()
                .map(|(k, v)| json!({ "key": k, "value": v }))
                .collect();
            req.insert(
                "body".into(),
                json!({ "mode": "urlencoded", "urlencoded": pairs }),
            );
        } else {
            req.insert(
                "body".into(),
                json!({
                    "mode": "raw",
                    "raw": body_text,
                    "options": { "raw": { "language": raw_language(ct) } }
                }),
            );
        }
    }

    json!({
        "name": request_label(flow),
        "request": Value::Object(req),
        "response": []
    })
}

/// Builds a Bruno `.bru` text bundle from filtered flows.
pub fn build_bruno_bundle(flows: &[Flow], redact_opts: &RedactOptions) -> String {
    let list = exportable_flows(flows, redact_opts);
    let mut out = String::new();
    for (idx, flow) in list.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n// ---\n\n");
        }
        write_bruno_request(&mut out, flow, idx + 1);
    }
    out
}

fn write_bruno_request(out: &mut String, flow: &Flow, seq: usize) {
    let method_lower = flow.method.to_ascii_lowercase();
    let ct = content_type(&flow.request_headers);
    let body_text = std::str::from_utf8(&flow.request_body)
        .ok()
        .filter(|s| !s.is_empty());
    let body_kind = match body_text {
        None => "none",
        Some(_) => {
            let lower = ct.unwrap_or("").to_ascii_lowercase();
            if lower.contains("json") || lower.contains("graphql") {
                "json"
            } else if lower.contains("xml") {
                "xml"
            } else if lower.contains("x-www-form-urlencoded") {
                "form-urlencoded"
            } else {
                "text"
            }
        }
    };

    let _ = writeln!(out, "meta {{");
    let _ = writeln!(out, "  name: {}", request_label(flow));
    let _ = writeln!(out, "  type: http");
    let _ = writeln!(out, "  seq: {seq}");
    let _ = writeln!(out, "}}\n");

    let _ = writeln!(out, "{method_lower} {{");
    let _ = writeln!(out, "  url: {}", flow.url);
    let _ = writeln!(out, "  body: {body_kind}");
    let _ = writeln!(out, "  auth: none");
    let _ = writeln!(out, "}}");

    let params = query_pairs(&flow.url);
    if !params.is_empty() {
        let _ = writeln!(out, "\nparams:query {{");
        for (k, v) in params {
            let _ = writeln!(out, "  {k}: {v}");
        }
        let _ = writeln!(out, "}}");
    }

    let headers = export_headers(&flow.request_headers);
    if !headers.is_empty() {
        let _ = writeln!(out, "\nheaders {{");
        for (k, v) in headers {
            let _ = writeln!(out, "  {k}: {v}");
        }
        let _ = writeln!(out, "}}");
    }

    if let Some(body) = body_text {
        let _ = writeln!(out, "\nbody:{body_kind} {{");
        if body_kind == "form-urlencoded" {
            for (k, v) in query_pairs(&format!("?{body}")) {
                let _ = writeln!(out, "  {k}: {v}");
            }
        } else {
            for line in body.lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
        let _ = writeln!(out, "}}");
    }
}

/// Builds a `.http` file compatible with VS Code REST Client & JetBrains HTTP Client.
pub fn build_http_file(flows: &[Flow], redact_opts: &RedactOptions) -> String {
    let list = exportable_flows(flows, redact_opts);
    let mut out = String::new();
    for (idx, flow) in list.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "### #{} {}", flow.id, request_label(flow));
        let _ = writeln!(
            out,
            "{} {} HTTP/1.1",
            flow.method.to_ascii_uppercase(),
            flow.url
        );
        for (k, v) in export_headers(&flow.request_headers) {
            let _ = writeln!(out, "{k}: {v}");
        }
        if let Ok(body) = std::str::from_utf8(&flow.request_body)
            && !body.is_empty()
        {
            out.push('\n');
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Direction, FlowDraft, Session};

    fn sample_flows() -> Vec<Flow> {
        let mut session = Session::default();
        let id1 = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: "GET".into(),
            host: "api.example.com:8443".into(),
            path: "/v1/items?page=2&token=secret".into(),
            url: "https://api.example.com:8443/v1/items?page=2&token=secret".into(),
            request_headers: vec![
                ("Host".into(), "api.example.com:8443".into()),
                ("Authorization".into(), "Bearer super-secret".into()),
                ("Accept".into(), "application/json".into()),
            ],
        });
        let id2 = session.open(FlowDraft {
            protocol: Protocol::Http2,
            method: "POST".into(),
            host: "api.example.com".into(),
            path: "/v1/users".into(),
            url: "https://api.example.com/v1/users".into(),
            request_headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Content-Length".into(), "16".into()),
            ],
        });
        session.append_body(id2, Direction::Request, br#"{"name":"alice"}"#);
        let id3 = session.open(FlowDraft {
            protocol: Protocol::Tunnel,
            method: "CONNECT".into(),
            host: "tunnel.example.com".into(),
            path: "tunnel.example.com:443".into(),
            url: "tunnel.example.com:443".into(),
            request_headers: vec![],
        });
        vec![
            session.get(id1).unwrap().clone(),
            session.get(id2).unwrap().clone(),
            session.get(id3).unwrap().clone(),
        ]
    }

    #[test]
    fn postman_v21_exports_valid_schema_and_respects_redaction() {
        let flows = sample_flows();
        let json_str = build_postman_collection(&flows, &RedactOptions::new(true, ""));
        let doc: Value = serde_json::from_str(&json_str).expect("valid JSON");
        assert_eq!(
            doc["info"]["schema"],
            "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        );
        let items = doc["item"].as_array().expect("item array");
        assert_eq!(items.len(), 2, "CONNECT tunnel should be excluded");
        assert_eq!(items[0]["request"]["method"], "GET");
        assert_eq!(items[0]["request"]["url"]["port"], "8443");
        assert!(!json_str.contains("super-secret"));
        assert!(!json_str.contains("token=secret"));
        assert_eq!(items[1]["request"]["body"]["mode"], "raw");
        assert_eq!(
            items[1]["request"]["body"]["options"]["raw"]["language"],
            "json"
        );
    }

    #[test]
    fn bruno_bundle_formats_meta_method_params_headers_and_body() {
        let flows = sample_flows();
        let bru = build_bruno_bundle(&flows, &RedactOptions::default());
        assert!(bru.contains("meta {\n  name: GET /v1/items\n  type: http\n  seq: 1\n}"));
        assert!(bru.contains("get {\n  url: https://api.example.com:8443/v1/items?page=2&token=secret\n  body: none\n  auth: none\n}"));
        assert!(bru.contains("params:query {\n  page: 2\n  token: secret\n}"));
        assert!(bru.contains("// ---"));
        assert!(bru.contains(
            "post {\n  url: https://api.example.com/v1/users\n  body: json\n  auth: none\n}"
        ));
        assert!(bru.contains("body:json {\n  {\"name\":\"alice\"}\n}"));
    }

    #[test]
    fn http_file_formats_requests_separated_by_triple_hash() {
        let flows = sample_flows();
        let http = build_http_file(&flows, &RedactOptions::default());
        assert!(http.contains("### #1 GET /v1/items\nGET https://api.example.com:8443/v1/items?page=2&token=secret HTTP/1.1"));
        assert!(http.contains("Accept: application/json"));
        assert!(!http.contains("Host:"));
        assert!(
            http.contains("### #2 POST /v1/users\nPOST https://api.example.com/v1/users HTTP/1.1")
        );
        assert!(http.contains("\n\n{\"name\":\"alice\"}\n"));
    }
}
