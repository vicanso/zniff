//! Owned hook stage payloads and QuickJS boundary conversion helpers.

use rquickjs::{Ctx, FromJs, Object, Result as JsResult, Value};

use crate::hook_codec_crypto::extract_bytes_arg;
use crate::hook_params::{
    ExtractedHookBody, apply_query_mutation, apply_set_headers, ensure_default_content_type,
    extract_body_or_structured,
};
use crate::hook_spec::{PreviewCardNode, validate_preview_cards};
use crate::script_worker::{PACKET_MAX_OUTPUT_BYTES, format_js_err};
use crate::session::BODY_CAP;

pub(crate) const MAX_HOOK_HEADERS: usize = 512;

#[derive(Debug, Clone)]
pub(crate) struct HookRequestInputOwned {
    pub flow_id: u64,
    pub method: String,
    pub scheme: String,
    pub host: String,
    pub url: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct HookResponseInputOwned {
    pub flow_id: u64,
    pub method: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct HookPreviewInputOwned {
    pub flow_id: u64,
    pub is_response: bool,
    pub method: String,
    pub host: String,
    pub path: String,
    pub url: String,
    pub status: Option<u16>,
    pub req_headers: Vec<(String, String)>,
    pub req_body: Vec<u8>,
    pub res_headers: Vec<(String, String)>,
    pub res_body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) enum ScriptRequestAction {
    Pass {
        comment: Option<String>,
        view: Vec<PreviewCardNode>,
    },
    Mutate {
        method: Option<String>,
        path: Option<String>,
        headers: Option<Vec<(String, String)>>,
        body: Option<Vec<u8>>,
        comment: Option<String>,
        view: Vec<PreviewCardNode>,
    },
    RespondWith {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        comment: Option<String>,
        view: Vec<PreviewCardNode>,
        res_view: Vec<PreviewCardNode>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptResponseAction {
    pub status: Option<u16>,
    pub headers: Option<Vec<(String, String)>>,
    pub body: Option<Vec<u8>>,
    pub comment: Option<String>,
    pub view: Vec<PreviewCardNode>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptPreviewSide {
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub note: Option<String>,
    pub view: Vec<PreviewCardNode>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptPreviewAction {
    pub side: ScriptPreviewSide,
    pub cards: Vec<PreviewCardNode>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptRunReport {
    pub hook_id: String,
    pub summary: String,
    pub cards: Vec<PreviewCardNode>,
    pub duration_ms: u64,
}

pub(crate) fn headers_to_js<'js>(
    cx: &Ctx<'js>,
    headers: &[(String, String)],
) -> JsResult<rquickjs::Array<'js>> {
    let arr = rquickjs::Array::new(cx.clone())?;
    for (idx, (k, v)) in headers.iter().enumerate() {
        let pair = rquickjs::Array::new(cx.clone())?;
        pair.set(0, k.as_str())?;
        pair.set(1, v.as_str())?;
        arr.set(idx, pair)?;
    }
    Ok(arr)
}

pub(crate) fn js_to_headers<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
) -> Result<Vec<(String, String)>, String> {
    if val.is_array() {
        let arr = rquickjs::Array::from_value(val).map_err(|e| format_js_err(cx, e))?;
        let mut out = Vec::with_capacity(arr.len().min(MAX_HOOK_HEADERS));
        for item in arr.iter::<Value<'js>>().take(MAX_HOOK_HEADERS) {
            let item = item.map_err(|e| format_js_err(cx, e))?;
            let pair = rquickjs::Array::from_value(item)
                .map_err(|_| "header entry must be [name, value]".to_string())?;
            let name = pair
                .get::<String>(0)
                .map_err(|_| "header name must be string".to_string())?;
            let value = pair
                .get::<String>(1)
                .map_err(|_| "header value must be string".to_string())?;
            if !name.trim().is_empty() {
                out.push((name, value));
            }
        }
        return Ok(out);
    }
    if let Some(obj) = val.as_object() {
        let mut out = Vec::new();
        for prop in obj.props::<String, Value<'js>>().take(MAX_HOOK_HEADERS) {
            let (name, v) = prop.map_err(|e| format_js_err(cx, e))?;
            if v.is_null() || v.is_undefined() {
                continue;
            }
            let text = if let Some(s) = v.as_string() {
                s.to_string().map_err(|e| format_js_err(cx, e))?
            } else {
                String::from_js(cx, v).map_err(|e| format_js_err(cx, e))?
            };
            if !name.trim().is_empty() {
                out.push((name, text));
            }
        }
        return Ok(out);
    }
    Err("headers must be [[name, value]] array or object".to_string())
}

pub(crate) fn extract_hook_body<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    allow_json_obj: bool,
) -> Result<(Vec<u8>, bool), String> {
    if let Some(s) = val.as_string() {
        let text = s.to_string().map_err(|e| format_js_err(cx, e))?;
        if text.len() > BODY_CAP {
            return Err("hook body exceeds 1 MB limit".to_string());
        }
        return Ok((text.into_bytes(), false));
    }
    if val.is_object() {
        if let Ok(bytes) = extract_bytes_arg(cx, val.clone()) {
            if bytes.len() > BODY_CAP {
                return Err("hook binary body exceeds 1 MB limit".to_string());
            }
            return Ok((bytes, false));
        }
        if allow_json_obj {
            let json = cx
                .json_stringify(val)
                .map_err(|e| format_js_err(cx, e))?
                .map(|s| s.to_string())
                .transpose()
                .map_err(|e| format_js_err(cx, e))?
                .ok_or_else(|| "cannot JSON-serialize hook body".to_string())?;
            if json.len() > BODY_CAP {
                return Err("hook JSON body exceeds 1 MB limit".to_string());
            }
            return Ok((json.into_bytes(), true));
        }
    }
    Err("hook body must be Uint8Array, ArrayBuffer, or string".to_string())
}

fn optional_str_prop<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
) -> Result<Option<String>, String> {
    let val: Value<'js> = obj.get(key).map_err(|e| format_js_err(cx, e))?;
    if val.is_undefined() || val.is_null() {
        return Ok(None);
    }
    let s = String::from_js(cx, val).map_err(|e| format_js_err(cx, e))?;
    Ok(Some(s))
}

fn parse_view_prop<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
) -> Result<Vec<PreviewCardNode>, String> {
    let val: Value<'js> = obj.get(key).map_err(|e| format_js_err(cx, e))?;
    if val.is_undefined() || val.is_null() {
        return Ok(Vec::new());
    }
    let Some(json_str) = cx
        .json_stringify(val)
        .map_err(|e| format_js_err(cx, e))?
        .map(|s| s.to_string())
        .transpose()
        .map_err(|e| format_js_err(cx, e))?
    else {
        return Ok(Vec::new());
    };
    if json_str.len() > PACKET_MAX_OUTPUT_BYTES {
        return Err("hook preview cards exceed IR size budget".to_string());
    }
    if json_str.trim_start().starts_with('[') {
        serde_json::from_str(&json_str).map_err(|e| format!("invalid preview cards IR: {e}"))
    } else {
        Ok(vec![
            serde_json::from_str(&json_str).map_err(|e| format!("invalid preview card IR: {e}"))?,
        ])
    }
}

fn parse_preview_cards<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
) -> Result<Vec<PreviewCardNode>, String> {
    let mut nodes = parse_view_prop(cx, obj, "view")?;
    for extra in parse_view_prop(cx, obj, "cards")? {
        if nodes.last() != Some(&extra) {
            nodes.push(extra);
        }
    }
    if !nodes.is_empty() {
        validate_preview_cards(&nodes)?;
    }
    Ok(nodes)
}

fn resolve_headers_update<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
    base_headers: &[(String, String)],
    structured: Option<&ExtractedHookBody>,
) -> Result<Option<Vec<(String, String)>>, String> {
    let hdr_val: Value<'js> = obj.get("headers").map_err(|e| format_js_err(cx, e))?;
    let has_headers = !hdr_val.is_undefined() && !hdr_val.is_null();
    let mut next = if has_headers {
        js_to_headers(cx, hdr_val)?
    } else {
        base_headers.to_vec()
    };
    let explicit_hdr_ct = has_headers
        && next
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));

    let set_val: Value<'js> = obj.get("setHeaders").map_err(|e| format_js_err(cx, e))?;
    let has_set_headers = !set_val.is_undefined() && !set_val.is_null();
    let explicit_set_ct = if has_set_headers {
        let (patched, touched_ct) = apply_set_headers(cx, &next, set_val)?;
        next = patched;
        touched_ct
    } else {
        false
    };

    if let Some(ExtractedHookBody {
        default_content_type: Some(ct),
        expected_ct_prefix: Some(prefix),
        ..
    }) = structured
        && !explicit_hdr_ct
        && !explicit_set_ct
    {
        ensure_default_content_type(&mut next, ct, prefix);
    }

    if has_headers || (has_set_headers && next != base_headers) || next != base_headers {
        Ok(Some(next))
    } else {
        Ok(None)
    }
}

pub(crate) fn parse_request_action<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    input_path: &str,
    input_headers: &[(String, String)],
) -> Result<ScriptRequestAction, String> {
    if val.is_undefined() || val.is_null() {
        return Ok(ScriptRequestAction::Pass {
            comment: None,
            view: Vec::new(),
        });
    }
    let Some(obj) = val.into_object() else {
        return Err("onRequest must return an object or void".to_string());
    };
    let comment = optional_str_prop(cx, &obj, "comment")?.and_then(clean_nonempty);
    let view = parse_preview_cards(cx, &obj)?;
    let rw_val: Value<'js> = obj.get("respondWith").map_err(|e| format_js_err(cx, e))?;
    if !rw_val.is_undefined() && !rw_val.is_null() {
        let Some(rw) = rw_val.into_object() else {
            return Err("respondWith must be an object".to_string());
        };
        let status_opt: Option<u32> = rw.get("status").ok();
        let status = status_opt
            .and_then(|s| u16::try_from(s).ok())
            .filter(|s| (100..600).contains(s))
            .ok_or_else(|| "respondWith.status must be 100..599".to_string())?;
        let extracted = extract_body_or_structured(cx, &rw, true)?;
        let headers = resolve_headers_update(cx, &rw, &[], extracted.as_ref())?.unwrap_or_default();
        let body = extracted.map_or_else(Vec::new, |b| b.bytes);
        let res_view = parse_preview_cards(cx, &rw)?;
        return Ok(ScriptRequestAction::RespondWith {
            status,
            headers,
            body,
            comment,
            view,
            res_view,
        });
    }

    let method = optional_str_prop(cx, &obj, "method")?.and_then(|m| {
        let trimmed = m.trim().to_ascii_uppercase();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let mut next_path = optional_str_prop(cx, &obj, "path")?.filter(|p| p.starts_with('/'));
    let query_val: Value<'js> = obj.get("query").map_err(|e| format_js_err(cx, e))?;
    if !query_val.is_undefined() && !query_val.is_null() {
        let base = next_path.as_deref().unwrap_or(input_path);
        let patched = apply_query_mutation(cx, base, query_val)?;
        if patched != input_path {
            next_path = Some(patched);
        }
    }

    let extracted = extract_body_or_structured(cx, &obj, false)?;
    let headers = resolve_headers_update(cx, &obj, input_headers, extracted.as_ref())?;
    let body = extracted.map(|b| b.bytes);

    if method.is_none() && next_path.is_none() && headers.is_none() && body.is_none() {
        Ok(ScriptRequestAction::Pass { comment, view })
    } else {
        Ok(ScriptRequestAction::Mutate {
            method,
            path: next_path,
            headers,
            body,
            comment,
            view,
        })
    }
}

pub(crate) fn parse_response_action<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    input_headers: &[(String, String)],
) -> Result<ScriptResponseAction, String> {
    if val.is_undefined() || val.is_null() {
        return Ok(ScriptResponseAction {
            status: None,
            headers: None,
            body: None,
            comment: None,
            view: Vec::new(),
        });
    }
    let Some(obj) = val.into_object() else {
        return Err("onResponse must return an object or void".to_string());
    };
    let status_val: Value<'js> = obj.get("status").map_err(|e| format_js_err(cx, e))?;
    let status = if status_val.is_undefined() || status_val.is_null() {
        None
    } else {
        let code = u32::from_js(cx, status_val).map_err(|e| format_js_err(cx, e))?;
        let parsed = u16::try_from(code)
            .ok()
            .filter(|s| (100..600).contains(s))
            .ok_or_else(|| "response status must be 100..599".to_string())?;
        Some(parsed)
    };
    let extracted = extract_body_or_structured(cx, &obj, false)?;
    let headers = resolve_headers_update(cx, &obj, input_headers, extracted.as_ref())?;
    let body = extracted.map(|b| b.bytes);
    let comment = optional_str_prop(cx, &obj, "comment")?.and_then(clean_nonempty);
    let view = parse_preview_cards(cx, &obj)?;
    Ok(ScriptResponseAction {
        status,
        headers,
        body,
        comment,
        view,
    })
}

fn parse_preview_side_obj<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
    allow_view: bool,
) -> Result<ScriptPreviewSide, String> {
    let mut ct = optional_str_prop(cx, obj, "contentType")?.and_then(clean_nonempty);
    let note = optional_str_prop(cx, obj, "note")?
        .or(optional_str_prop(cx, obj, "comment")?)
        .and_then(clean_nonempty);
    let extracted = extract_body_or_structured(cx, obj, true)?;
    let body = match extracted {
        Some(ext) => {
            if ct.is_none() {
                ct = ext.expected_ct_prefix.map(str::to_string);
            }
            Some(ext.bytes)
        }
        None => None,
    };
    let view = if allow_view {
        parse_preview_cards(cx, obj)?
    } else {
        Vec::new()
    };
    Ok(ScriptPreviewSide {
        body,
        content_type: ct,
        note,
        view,
    })
}

pub(crate) fn parse_preview_action<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    is_response: bool,
) -> Result<ScriptPreviewAction, String> {
    if val.is_undefined() || val.is_null() {
        return Ok(ScriptPreviewAction::default());
    }
    let Some(obj) = val.into_object() else {
        return Err("onPreview must return an object or void".to_string());
    };
    let cards = parse_preview_cards(cx, &obj)?;
    let side_key = if is_response { "res" } else { "req" };
    let side_val: Value<'js> = obj.get(side_key).map_err(|e| format_js_err(cx, e))?;
    let side = if let Some(side_obj) = side_val.into_object() {
        parse_preview_side_obj(cx, &side_obj, true)?
    } else {
        parse_preview_side_obj(cx, &obj, false)?
    };
    Ok(ScriptPreviewAction { side, cards })
}

pub(crate) fn parse_run_report<'js>(
    cx: &Ctx<'js>,
    val: Value<'js>,
    hook_id: String,
    duration_ms: u64,
) -> Result<ScriptRunReport, String> {
    if val.is_undefined() || val.is_null() {
        return Ok(ScriptRunReport {
            hook_id,
            summary: String::new(),
            cards: Vec::new(),
            duration_ms,
        });
    }
    if let Some(s) = val.as_string() {
        let summary = s.to_string().map_err(|e| format_js_err(cx, e))?;
        return Ok(ScriptRunReport {
            hook_id,
            summary,
            cards: Vec::new(),
            duration_ms,
        });
    }
    let Some(obj) = val.into_object() else {
        return Err("onRun must return an object, string, or void".to_string());
    };
    let summary = optional_str_prop(cx, &obj, "summary")?.unwrap_or_default();
    let cards = parse_preview_cards(cx, &obj)?;
    Ok(ScriptRunReport {
        hook_id,
        summary,
        cards,
        duration_ms,
    })
}

fn clean_nonempty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}
