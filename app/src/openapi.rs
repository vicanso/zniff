//! OpenAPI 3.0.3 schema synthesis from captured HTTP(S) flows.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::redact::{RedactOptions, redact_flow};
use crate::session::{Flow, Protocol};

const MAX_SCHEMA_DEPTH: usize = 12;
const MAX_OBJECT_PROPS: usize = 128;

#[derive(Default)]
struct EndpointAccum {
    tag: String,
    path_params: Vec<String>,
    query_params: BTreeMap<String, Option<InferredSchema>>,
    request_content_type: Option<String>,
    request_schema: Option<InferredSchema>,
    form_fields: BTreeMap<String, InferredSchema>,
    responses: BTreeMap<u16, ResponseAccum>,
}

#[derive(Default)]
struct ResponseAccum {
    content_type: Option<String>,
    schema: Option<InferredSchema>,
}

#[derive(Clone, Debug, PartialEq)]
enum InferredSchema {
    Null,
    Bool {
        nullable: bool,
    },
    Integer {
        nullable: bool,
    },
    Number {
        nullable: bool,
    },
    String {
        nullable: bool,
        format: Option<&'static str>,
    },
    Array {
        items: Box<InferredSchema>,
        nullable: bool,
    },
    Object {
        props: BTreeMap<String, InferredSchema>,
        required: BTreeSet<String>,
        nullable: bool,
    },
    Any {
        nullable: bool,
    },
}

impl InferredSchema {
    fn from_json(value: &Value, depth: usize) -> Self {
        if depth >= MAX_SCHEMA_DEPTH {
            return Self::Any { nullable: false };
        }
        match value {
            Value::Null => Self::Null,
            Value::Bool(_) => Self::Bool { nullable: false },
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    Self::Integer { nullable: false }
                } else {
                    Self::Number { nullable: false }
                }
            }
            Value::String(s) => Self::String {
                nullable: false,
                format: detect_string_format(s),
            },
            Value::Array(items) => {
                let mut merged: Option<Self> = None;
                for item in items.iter().take(32) {
                    let s = Self::from_json(item, depth + 1);
                    merged = Some(match merged {
                        Some(cur) => cur.merge(s, depth + 1),
                        None => s,
                    });
                }
                Self::Array {
                    items: Box::new(merged.unwrap_or(Self::Any { nullable: false })),
                    nullable: false,
                }
            }
            Value::Object(map) => {
                let mut props = BTreeMap::new();
                let mut required = BTreeSet::new();
                for (k, v) in map.iter().take(MAX_OBJECT_PROPS) {
                    props.insert(k.clone(), Self::from_json(v, depth + 1));
                    if !v.is_null() {
                        required.insert(k.clone());
                    }
                }
                Self::Object {
                    props,
                    required,
                    nullable: false,
                }
            }
        }
    }

    fn with_nullable(mut self) -> Self {
        match &mut self {
            Self::Null => {}
            Self::Bool { nullable }
            | Self::Integer { nullable }
            | Self::Number { nullable }
            | Self::String { nullable, .. }
            | Self::Array { nullable, .. }
            | Self::Object { nullable, .. }
            | Self::Any { nullable } => *nullable = true,
        }
        self
    }

    fn merge(self, other: Self, depth: usize) -> Self {
        if depth >= MAX_SCHEMA_DEPTH {
            return Self::Any { nullable: true };
        }
        match (self, other) {
            (Self::Null, Self::Null) => Self::Null,
            (Self::Null, o) | (o, Self::Null) => o.with_nullable(),
            (Self::Bool { nullable: a }, Self::Bool { nullable: b }) => {
                Self::Bool { nullable: a || b }
            }
            (Self::Integer { nullable: a }, Self::Integer { nullable: b }) => {
                Self::Integer { nullable: a || b }
            }
            (
                Self::Integer { nullable: a } | Self::Number { nullable: a },
                Self::Integer { nullable: b } | Self::Number { nullable: b },
            ) => Self::Number { nullable: a || b },
            (
                Self::String {
                    nullable: a,
                    format: fa,
                },
                Self::String {
                    nullable: b,
                    format: fb,
                },
            ) => Self::String {
                nullable: a || b,
                format: if fa == fb { fa } else { None },
            },
            (
                Self::Array {
                    items: ia,
                    nullable: a,
                },
                Self::Array {
                    items: ib,
                    nullable: b,
                },
            ) => Self::Array {
                items: Box::new(ia.merge(*ib, depth + 1)),
                nullable: a || b,
            },
            (
                Self::Object {
                    mut props,
                    required: ra,
                    nullable: na,
                },
                Self::Object {
                    props: pb,
                    required: rb,
                    nullable: nb,
                },
            ) => {
                let req_intersection: BTreeSet<String> = ra.intersection(&rb).cloned().collect();
                for (k, vb) in pb {
                    if let Some(va) = props.remove(&k) {
                        props.insert(k, va.merge(vb, depth + 1));
                    } else if props.len() < MAX_OBJECT_PROPS {
                        props.insert(k, vb);
                    }
                }
                Self::Object {
                    props,
                    required: req_intersection,
                    nullable: na || nb,
                }
            }
            _ => Self::Any { nullable: true },
        }
    }

    fn to_json(&self) -> Value {
        let mut obj = Map::new();
        match self {
            Self::Null => {
                obj.insert("nullable".into(), Value::Bool(true));
            }
            Self::Bool { nullable } => {
                obj.insert("type".into(), json!("boolean"));
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::Integer { nullable } => {
                obj.insert("type".into(), json!("integer"));
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::Number { nullable } => {
                obj.insert("type".into(), json!("number"));
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::String { nullable, format } => {
                obj.insert("type".into(), json!("string"));
                if let Some(fmt) = format {
                    obj.insert("format".into(), json!(fmt));
                }
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::Array { items, nullable } => {
                obj.insert("type".into(), json!("array"));
                obj.insert("items".into(), items.to_json());
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::Object {
                props,
                required,
                nullable,
            } => {
                obj.insert("type".into(), json!("object"));
                let prop_map: Map<String, Value> = props
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect();
                obj.insert("properties".into(), Value::Object(prop_map));
                if !required.is_empty() {
                    obj.insert(
                        "required".into(),
                        Value::Array(required.iter().map(|r| json!(r)).collect()),
                    );
                }
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
            Self::Any { nullable } => {
                if *nullable {
                    obj.insert("nullable".into(), Value::Bool(true));
                }
            }
        }
        Value::Object(obj)
    }
}

fn detect_string_format(s: &str) -> Option<&'static str> {
    if is_uuid(s) {
        return Some("uuid");
    }
    if s.len() >= 20
        && s.len() <= 32
        && s.as_bytes().get(4) == Some(&b'-')
        && s.as_bytes().get(7) == Some(&b'-')
        && s.as_bytes().get(10) == Some(&b'T')
        && (s.ends_with('Z') || s.contains('+'))
    {
        return Some("date-time");
    }
    None
}

fn is_uuid(seg: &str) -> bool {
    let bytes = seg.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 8 | 13 | 18 | 23) || b.is_ascii_hexdigit())
}

fn is_dynamic_segment(seg: &str) -> bool {
    if seg.is_empty() {
        return false;
    }
    if seg.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if is_uuid(seg) {
        return true;
    }
    seg.len() >= 16 && seg.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Normalizes dynamic ID/UUID path segments into `{id}`, `{id2}`, ... and returns
/// the OpenAPI path template alongside extracted path parameter names.
pub fn normalize_path(raw_path: &str) -> (String, Vec<String>) {
    let clean = raw_path.split(['?', '#']).next().unwrap_or("/");
    let clean = if clean.starts_with('/') { clean } else { "/" };
    let mut out_segs = Vec::new();
    let mut params = Vec::new();
    let mut id_counter = 1usize;
    for seg in clean.split('/').skip(1) {
        if seg.is_empty() {
            continue;
        }
        if is_dynamic_segment(seg) {
            let param_name = if id_counter == 1 {
                "id".to_string()
            } else {
                format!("id{id_counter}")
            };
            id_counter += 1;
            out_segs.push(format!("{{{param_name}}}"));
            params.push(param_name);
        } else {
            out_segs.push(seg.to_string());
        }
    }
    if out_segs.is_empty() {
        ("/".to_string(), params)
    } else {
        (format!("/{}", out_segs.join("/")), params)
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn clean_mime(ct: Option<&str>) -> Option<String> {
    let raw = ct?.split(';').next()?.trim().to_ascii_lowercase();
    (!raw.is_empty()).then_some(raw)
}

fn parse_query_pairs(target: &str) -> Vec<(String, String)> {
    let Some((_, q)) = target.split_once('?') else {
        return Vec::new();
    };
    let q = q.split('#').next().unwrap_or(q);
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (pair.to_string(), String::new()),
        })
        .collect()
}

fn schema_from_string_scalar(raw: &str) -> InferredSchema {
    if raw == "true" || raw == "false" {
        InferredSchema::Bool { nullable: false }
    } else if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
        InferredSchema::Integer { nullable: false }
    } else {
        InferredSchema::String {
            nullable: false,
            format: detect_string_format(raw),
        }
    }
}

fn merge_schema_slot(slot: &mut Option<InferredSchema>, incoming: InferredSchema) {
    *slot = Some(match slot.take() {
        Some(existing) => existing.merge(incoming, 0),
        None => incoming,
    });
}

fn origin_from_url(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then(|| format!("{scheme}://{authority}"))
}

fn build_endpoint_operation(acc: &EndpointAccum, method: &str, path: &str) -> Value {
    let mut op = Map::new();
    if !acc.tag.is_empty() {
        op.insert("tags".into(), json!([acc.tag]));
    }
    op.insert(
        "summary".into(),
        json!(format!("{} {path}", method.to_ascii_uppercase())),
    );
    let mut parameters = Vec::new();
    for p in &acc.path_params {
        parameters.push(json!({
            "name": p,
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
        }));
    }
    for (name, schema) in &acc.query_params {
        let schema_val = schema
            .as_ref()
            .map(InferredSchema::to_json)
            .unwrap_or_else(|| json!({ "type": "string" }));
        parameters.push(json!({
            "name": name,
            "in": "query",
            "required": false,
            "schema": schema_val
        }));
    }
    if !parameters.is_empty() {
        op.insert("parameters".into(), Value::Array(parameters));
    }
    if let Some(ref req_schema) = acc.request_schema {
        let mime = acc
            .request_content_type
            .as_deref()
            .unwrap_or("application/json");
        op.insert(
            "requestBody".into(),
            json!({
                "content": {
                    mime: { "schema": req_schema.to_json() }
                }
            }),
        );
    } else if !acc.form_fields.is_empty() {
        let props: Map<String, Value> = acc
            .form_fields
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect();
        op.insert(
            "requestBody".into(),
            json!({
                "content": {
                    "application/x-www-form-urlencoded": {
                        "schema": {
                            "type": "object",
                            "properties": Value::Object(props)
                        }
                    }
                }
            }),
        );
    }
    let mut responses = Map::new();
    if acc.responses.is_empty() {
        responses.insert("default".into(), json!({ "description": "Response" }));
    } else {
        for (code, resp) in &acc.responses {
            let mut r_obj = Map::new();
            r_obj.insert("description".into(), json!(format!("HTTP {code}")));
            if let Some(ref schema) = resp.schema {
                let mime = resp.content_type.as_deref().unwrap_or("application/json");
                r_obj.insert(
                    "content".into(),
                    json!({
                        mime: { "schema": schema.to_json() }
                    }),
                );
            }
            responses.insert(code.to_string(), Value::Object(r_obj));
        }
    }
    op.insert("responses".into(), Value::Object(responses));
    Value::Object(op)
}

/// Synthesizes an OpenAPI 3.0.3 JSON document from the provided flows, honoring redaction.
pub fn build_openapi_value(flows: &[Flow], redact: &RedactOptions) -> Value {
    let mut servers = BTreeSet::new();
    let mut paths: BTreeMap<String, BTreeMap<String, EndpointAccum>> = BTreeMap::new();

    for raw_flow in flows {
        if raw_flow.protocol == Protocol::Tunnel
            || raw_flow.method.eq_ignore_ascii_case("CONNECT")
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

        if let Some(origin) = origin_from_url(&flow.url) {
            servers.insert(origin);
        }
        let (norm_path, path_params) = normalize_path(&flow.path);
        let method_key = flow.method.to_ascii_lowercase();
        let method_map = paths.entry(norm_path).or_default();
        let acc = method_map
            .entry(method_key)
            .or_insert_with(|| EndpointAccum {
                tag: flow.host.clone(),
                path_params,
                ..Default::default()
            });

        for (qk, qv) in parse_query_pairs(&flow.path) {
            let slot = acc.query_params.entry(qk).or_insert(None);
            merge_schema_slot(slot, schema_from_string_scalar(&qv));
        }

        let req_ct = clean_mime(header_value(&flow.request_headers, "content-type"));
        if !flow.request_body.is_empty() {
            if let Ok(json_val) = serde_json::from_slice::<Value>(&flow.request_body) {
                acc.request_content_type
                    .get_or_insert_with(|| req_ct.unwrap_or_else(|| "application/json".into()));
                merge_schema_slot(
                    &mut acc.request_schema,
                    InferredSchema::from_json(&json_val, 0),
                );
            } else if req_ct
                .as_deref()
                .is_some_and(|ct| ct == "application/x-www-form-urlencoded")
                && let Ok(text) = std::str::from_utf8(&flow.request_body)
            {
                for pair in text.split('&').filter(|p| !p.is_empty()) {
                    let (fk, fv) = pair.split_once('=').unwrap_or((pair, ""));
                    let next_s = schema_from_string_scalar(fv);
                    if let Some(prev) = acc.form_fields.remove(fk) {
                        acc.form_fields
                            .insert(fk.to_string(), prev.merge(next_s, 0));
                    } else {
                        acc.form_fields.insert(fk.to_string(), next_s);
                    }
                }
            }
        }

        if let Some(code) = flow.status {
            let resp_acc = acc.responses.entry(code).or_default();
            let res_ct = clean_mime(header_value(&flow.response_headers, "content-type"));
            if !flow.response_body.is_empty()
                && let Ok(json_val) = serde_json::from_slice::<Value>(&flow.response_body)
            {
                resp_acc
                    .content_type
                    .get_or_insert_with(|| res_ct.unwrap_or_else(|| "application/json".into()));
                merge_schema_slot(
                    &mut resp_acc.schema,
                    InferredSchema::from_json(&json_val, 0),
                );
            }
        }
    }

    let mut path_obj = Map::new();
    for (norm_path, methods) in &paths {
        let mut method_obj = Map::new();
        for (method, acc) in methods {
            method_obj.insert(
                method.clone(),
                build_endpoint_operation(acc, method, norm_path),
            );
        }
        path_obj.insert(norm_path.clone(), Value::Object(method_obj));
    }

    let servers_val: Vec<Value> = servers
        .into_iter()
        .map(|url| json!({ "url": url }))
        .collect();
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "zniff Captured API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Synthesized from captured HTTP(S) traffic in zniff"
        },
        "servers": servers_val,
        "paths": Value::Object(path_obj)
    })
}

pub fn build_openapi_json(flows: &[Flow], redact: &RedactOptions) -> String {
    serde_json::to_string_pretty(&build_openapi_value(flows, redact))
        .unwrap_or_else(|_| "{}".to_string())
}

pub fn build_openapi_yaml(flows: &[Flow], redact: &RedactOptions) -> String {
    json_value_to_yaml(&build_openapi_value(flows, redact))
}

fn yaml_needs_quotes(s: &str) -> bool {
    if s.is_empty()
        || s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" | "y" | "n"
        )
    {
        return true;
    }
    if s.parse::<f64>().is_ok() {
        return true;
    }
    if let Some(first) = s.chars().next()
        && matches!(
            first,
            '-' | '?'
                | ':'
                | ','
                | '['
                | ']'
                | '{'
                | '}'
                | '#'
                | '&'
                | '*'
                | '!'
                | '|'
                | '>'
                | '\''
                | '"'
                | '%'
                | '@'
                | '`'
        )
    {
        return true;
    }
    s.chars()
        .any(|c| c.is_control() || matches!(c, ':' | '#' | '{' | '}' | '[' | ']' | ','))
}

fn format_yaml_scalar_str(s: &str) -> String {
    if yaml_needs_quotes(s) {
        serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
    } else {
        s.to_string()
    }
}

fn write_yaml_value(out: &mut String, value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => out.push_str(&format_yaml_scalar_str(s)),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push('\n');
                }
                out.push_str(&pad);
                match item {
                    Value::Object(map) if !map.is_empty() => {
                        out.push_str("- ");
                        for (k_idx, (k, v)) in map.iter().enumerate() {
                            if k_idx > 0 {
                                out.push('\n');
                                out.push_str(&pad);
                                out.push_str("  ");
                            }
                            write_yaml_entry(out, k, v, indent + 2);
                        }
                    }
                    Value::Array(nested) if !nested.is_empty() => {
                        out.push_str("-\n");
                        write_yaml_value(out, item, indent + 2);
                    }
                    _ => {
                        out.push_str("- ");
                        write_yaml_value(out, item, indent + 2);
                    }
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            for (idx, (k, v)) in map.iter().enumerate() {
                if idx > 0 {
                    out.push('\n');
                }
                out.push_str(&pad);
                write_yaml_entry(out, k, v, indent);
            }
        }
    }
}

fn write_yaml_entry(out: &mut String, key: &str, value: &Value, indent: usize) {
    out.push_str(&format_yaml_scalar_str(key));
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push_str(":\n");
            write_yaml_value(out, value, indent + 2);
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str(":\n");
            write_yaml_value(out, value, indent + 2);
        }
        _ => {
            out.push_str(": ");
            write_yaml_value(out, value, indent + 2);
        }
    }
}

/// Converts a `serde_json::Value` (such as an OpenAPI document) into YAML 1.2 block syntax.
pub fn json_value_to_yaml(value: &Value) -> String {
    let mut out = String::new();
    write_yaml_value(&mut out, value, 0);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Returns `(path_count, operation_count)` for summary reporting.
pub fn count_openapi_operations(doc: &Value) -> (usize, usize) {
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return (0, 0);
    };
    let ops = paths
        .values()
        .filter_map(Value::as_object)
        .map(Map::len)
        .sum();
    (paths.len(), ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Direction, FlowDraft, Protocol, Session};

    fn sample_flow(
        _id: u64,
        method: &str,
        path: &str,
        status: u16,
        req_body: &[u8],
        res_body: &[u8],
    ) -> Flow {
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: method.into(),
            host: "api.vicanso.dev".into(),
            path: path.into(),
            url: format!("https://api.vicanso.dev{path}"),
            request_headers: vec![
                ("Authorization".into(), "Bearer super-secret-jwt".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
        });
        if !req_body.is_empty() {
            session.append_body(id, Direction::Request, req_body);
        }
        session.set_response(
            id,
            status,
            vec![("Content-Type".into(), "application/json".into())],
            None,
        );
        if !res_body.is_empty() {
            session.append_body(id, Direction::Response, res_body);
        }
        session.mark_finished(id);
        session.hydrated(id).unwrap()
    }

    #[test]
    fn normalizes_numeric_uuid_and_hex_segments_into_path_parameters() {
        let (p1, params1) =
            normalize_path("/v1/users/12345/orders/550e8400-e29b-41d4-a716-446655440000?a=1");
        assert_eq!(p1, "/v1/users/{id}/orders/{id2}");
        assert_eq!(params1, vec!["id", "id2"]);

        let (p2, params2) = normalize_path("/v1/tokens/0123456789abcdef0123");
        assert_eq!(p2, "/v1/tokens/{id}");
        assert_eq!(params2, vec!["id"]);
    }

    #[test]
    fn merges_multi_sample_schemas_with_nullable_required_intersection_and_redaction() {
        let f1 = sample_flow(
            1,
            "GET",
            "/v1/users/101?active=true",
            200,
            b"",
            br#"{"id":101,"name":"Alice","token":"secret-tok","extra":"x"}"#,
        );
        let f2 = sample_flow(
            2,
            "GET",
            "/v1/users/202?active=false",
            200,
            b"",
            br#"{"id":202,"name":null,"token":"secret-tok-2"}"#,
        );
        let opts = RedactOptions {
            enabled: true,
            ..Default::default()
        };
        let doc = build_openapi_value(&[f1, f2], &opts);
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(doc["servers"][0]["url"], "https://api.vicanso.dev");
        let (path_cnt, op_cnt) = count_openapi_operations(&doc);
        assert_eq!((path_cnt, op_cnt), (1, 1));

        let get_op = &doc["paths"]["/v1/users/{id}"]["get"];
        let schema = &get_op["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(schema["properties"]["id"]["type"], "integer");
        assert_eq!(schema["properties"]["name"]["type"], "string");
        assert_eq!(schema["properties"]["name"]["nullable"], true);
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "id"));
        assert!(!required.iter().any(|v| v == "name"));
        assert!(!required.iter().any(|v| v == "extra"));
        // Redaction masks sensitive keys before synthesis -> `token` must be present, value masked.
        assert_eq!(schema["properties"]["token"]["type"], "string");
    }

    #[test]
    fn yaml_export_produces_valid_block_yaml_for_openapi_document() {
        let f = sample_flow(
            1,
            "POST",
            "/v1/users/42?flag=true",
            201,
            br#"{"name":"alice"}"#,
            br#"{"id":42,"ok":true}"#,
        );
        let yaml = build_openapi_yaml(&[f], &RedactOptions::default());
        assert!(yaml.contains("openapi: 3.0.3\n"), "{yaml}");
        assert!(yaml.contains("paths:\n"), "{yaml}");
        assert!(yaml.contains("  \"/v1/users/{id}\":\n"), "{yaml}");
        assert!(yaml.contains("    post:\n"), "{yaml}");
        assert!(yaml.contains("      \"201\":\n"), "{yaml}");
        assert!(
            yaml.contains("  - url: \"https://api.vicanso.dev\"\n"),
            "{yaml}"
        );
    }
}
