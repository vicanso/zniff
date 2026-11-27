//! Structured parameter helpers (`query`, `setHeaders`, `json`, `form`)
//! shared across QuickJS hook execution stages.

use rquickjs::{Array, ArrayBuffer, Ctx, Object, Result as JsResult, TypedArray, Value};

use crate::hook_action::{MAX_HOOK_HEADERS, extract_hook_body};
use crate::hook_codec::{encode_form_pairs, extract_form_entries, parse_query_pairs_str};
use crate::script_worker::format_js_err;
use crate::session::BODY_CAP;

const MAX_QUERY_PATH_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct ExtractedHookBody {
    pub bytes: Vec<u8>,
    pub default_content_type: Option<&'static str>,
    pub expected_ct_prefix: Option<&'static str>,
}

/// Decodes query `[key, value]` pairs from an HTTP target `/path?a=1&b=2`.
pub(crate) fn path_query_pairs(path: &str) -> Vec<(String, String)> {
    let before_frag = path.split('#').next().unwrap_or(path);
    let Some((_, q)) = before_frag.split_once('?') else {
        return Vec::new();
    };
    if q.is_empty() {
        return Vec::new();
    }
    parse_query_pairs_str(q, true)
}

/// Converts query pairs from `path` into a QuickJS `HeaderPair[]` array.
pub(crate) fn query_to_js<'js>(cx: &Ctx<'js>, path: &str) -> JsResult<Array<'js>> {
    let arr = Array::new(cx.clone())?;
    for (idx, (k, v)) in path_query_pairs(path).into_iter().enumerate() {
        let pair = Array::new(cx.clone())?;
        pair.set(0, k)?;
        pair.set(1, v)?;
        arr.set(idx, pair)?;
    }
    Ok(arr)
}

pub(crate) fn js_scalar_to_string<'js>(cx: &Ctx<'js>, val: Value<'js>) -> Result<String, String> {
    if let Some(s) = val.as_string() {
        return s.to_string().map_err(|e| format_js_err(cx, e));
    }
    if let Some(n) = val.as_int() {
        return Ok(n.to_string());
    }
    if let Some(n) = val.as_float() {
        if !n.is_finite() {
            return Err("numeric property must be finite".to_string());
        }
        if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
            return Ok((n as i64).to_string());
        }
        return Ok(n.to_string());
    }
    if let Some(b) = val.as_bool() {
        return Ok(if b { "true" } else { "false" }.to_string());
    }
    Err("value must be string, number, or boolean".to_string())
}

fn is_plain_js_object<'js>(val: &Value<'js>) -> bool {
    val.is_object()
        && !val.is_array()
        && TypedArray::<u8>::from_value(val.clone()).is_err()
        && ArrayBuffer::from_value(val.clone()).is_none()
}

/// Applies `query` (full array replacement or object patch/delete) to `current_path`.
pub(crate) fn apply_query_mutation<'js>(
    cx: &Ctx<'js>,
    current_path: &str,
    val: Value<'js>,
) -> Result<String, String> {
    if !current_path.starts_with('/') {
        return Err("request path must start with `/`".to_string());
    }
    let (before_frag, frag) = match current_path.find('#') {
        Some(idx) => (&current_path[..idx], &current_path[idx..]),
        None => (current_path, ""),
    };
    let base_path = before_frag.split_once('?').map_or(before_frag, |(b, _)| b);

    let pairs: Vec<(String, String)> = if val.is_array() {
        let arr = Array::from_value(val).map_err(|e| format_js_err(cx, e))?;
        let mut next = Vec::with_capacity(arr.len().min(MAX_HOOK_HEADERS));
        for item in arr.iter::<Value<'js>>().take(MAX_HOOK_HEADERS) {
            let item = item.map_err(|e| format_js_err(cx, e))?;
            let pair = Array::from_value(item)
                .map_err(|_| "query entry must be [key, value]".to_string())?;
            let key: String = pair
                .get(0)
                .map_err(|_| "query key must be string".to_string())?;
            let raw_v: Value<'js> = pair.get(1).map_err(|e| format_js_err(cx, e))?;
            let v_str = if raw_v.is_null() || raw_v.is_undefined() {
                String::new()
            } else {
                js_scalar_to_string(cx, raw_v)?
            };
            if !key.is_empty() {
                next.push((key, v_str));
            }
        }
        next
    } else if is_plain_js_object(&val) {
        let obj = val
            .into_object()
            .ok_or_else(|| "query must be object or array".to_string())?;
        let mut next = path_query_pairs(before_frag);
        for prop in obj.props::<String, Value<'js>>().take(MAX_HOOK_HEADERS) {
            let (k, v) = prop.map_err(|e| format_js_err(cx, e))?;
            if k.is_empty() {
                continue;
            }
            if v.is_null() || v.is_undefined() {
                next.retain(|(ek, _)| ek != &k);
            } else {
                let text = js_scalar_to_string(cx, v)?;
                upsert_pair(&mut next, k, text, false);
            }
        }
        next
    } else {
        return Err("query must be object or [[key, value]] array".to_string());
    };

    let encoded = encode_form_pairs(&pairs, false)?;
    let out = if encoded.is_empty() {
        format!("{base_path}{frag}")
    } else {
        format!("{base_path}?{encoded}{frag}")
    };
    if out.len() > MAX_QUERY_PATH_BYTES {
        return Err("mutated request path exceeds 64 KB limit".to_string());
    }
    Ok(out)
}

/// Applies incremental `setHeaders` patch/delete (`null`/`undefined` removes) to `current`.
pub(crate) fn apply_set_headers<'js>(
    cx: &Ctx<'js>,
    current: &[(String, String)],
    val: Value<'js>,
) -> Result<(Vec<(String, String)>, bool), String> {
    if !is_plain_js_object(&val) {
        return Err("setHeaders must be an object".to_string());
    }
    let obj = val
        .into_object()
        .ok_or_else(|| "setHeaders must be an object".to_string())?;
    let mut out = current.to_vec();
    let mut touched_ct = false;
    for prop in obj.props::<String, Value<'js>>().take(MAX_HOOK_HEADERS) {
        let (k, v) = prop.map_err(|e| format_js_err(cx, e))?;
        if k.trim().is_empty() {
            continue;
        }
        if k.eq_ignore_ascii_case("content-type") {
            touched_ct = true;
        }
        if v.is_null() || v.is_undefined() {
            out.retain(|(ek, _)| !ek.eq_ignore_ascii_case(&k));
        } else {
            let text = js_scalar_to_string(cx, v)?;
            upsert_pair(&mut out, k, text, true);
        }
    }
    Ok((out, touched_ct))
}

fn upsert_pair(
    pairs: &mut Vec<(String, String)>,
    key: String,
    val: String,
    case_insensitive: bool,
) {
    let matches = |a: &str, b: &str| {
        if case_insensitive {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let mut replaced = false;
    let mut idx = 0usize;
    while idx < pairs.len() {
        if matches(&pairs[idx].0, &key) {
            if !replaced {
                pairs[idx].0 = key.clone();
                pairs[idx].1 = val.clone();
                replaced = true;
                idx += 1;
            } else {
                pairs.remove(idx);
            }
        } else {
            idx += 1;
        }
    }
    if !replaced {
        pairs.push((key, val));
    }
}

/// Ensures `Content-Type` exists and matches `expected_prefix` unless already compatible.
pub(crate) fn ensure_default_content_type(
    headers: &mut Vec<(String, String)>,
    ct: &str,
    expected_prefix: &str,
) {
    if let Some((_, existing)) = headers
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        let trimmed = existing.trim().to_ascii_lowercase();
        if !trimmed.starts_with(expected_prefix) {
            *existing = ct.to_string();
        }
    } else {
        headers.push(("Content-Type".to_string(), ct.to_string()));
    }
}

/// Extracts `body`, `json`, or `form` from a JS hook return object.
pub(crate) fn extract_body_or_structured<'js>(
    cx: &Ctx<'js>,
    obj: &Object<'js>,
    allow_json_obj_in_body: bool,
) -> Result<Option<ExtractedHookBody>, String> {
    let body_val: Value<'js> = obj.get("body").map_err(|e| format_js_err(cx, e))?;
    let json_val: Value<'js> = obj.get("json").map_err(|e| format_js_err(cx, e))?;
    let form_val: Value<'js> = obj.get("form").map_err(|e| format_js_err(cx, e))?;

    let has_body = !body_val.is_undefined() && !body_val.is_null();
    let has_json = !json_val.is_undefined();
    let has_form = !form_val.is_undefined() && !form_val.is_null();
    if u8::from(has_body) + u8::from(has_json) + u8::from(has_form) > 1 {
        return Err("hook result cannot specify more than one of body, json, and form".to_string());
    }
    if has_json {
        let json = cx
            .json_stringify(json_val)
            .map_err(|e| format_js_err(cx, e))?
            .map(|s| s.to_string())
            .transpose()
            .map_err(|e| format_js_err(cx, e))?
            .ok_or_else(|| "cannot JSON-serialize hook `json` field".to_string())?;
        if json.len() > BODY_CAP {
            return Err("hook JSON body exceeds 1 MB limit".to_string());
        }
        return Ok(Some(ExtractedHookBody {
            bytes: json.into_bytes(),
            default_content_type: Some("application/json; charset=utf-8"),
            expected_ct_prefix: Some("application/json"),
        }));
    }
    if has_form {
        let entries = extract_form_entries(cx, form_val).map_err(|e| format_js_err(cx, e))?;
        let encoded = encode_form_pairs(&entries, true)?;
        return Ok(Some(ExtractedHookBody {
            bytes: encoded.into_bytes(),
            default_content_type: Some("application/x-www-form-urlencoded; charset=utf-8"),
            expected_ct_prefix: Some("application/x-www-form-urlencoded"),
        }));
    }
    if has_body {
        let (bytes, is_json) = extract_hook_body(cx, body_val, allow_json_obj_in_body)?;
        return Ok(Some(ExtractedHookBody {
            bytes,
            default_content_type: is_json.then_some("application/json; charset=utf-8"),
            expected_ct_prefix: is_json.then_some("application/json"),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Runtime};

    use super::*;

    #[test]
    fn path_query_pairs_decodes_and_skips_fragments() {
        let pairs = path_query_pairs("/v1/search?q=hello+world&tag=a&tag=b#frag");
        assert_eq!(
            pairs,
            vec![
                ("q".into(), "hello world".into()),
                ("tag".into(), "a".into()),
                ("tag".into(), "b".into()),
            ]
        );
        assert!(path_query_pairs("/plain").is_empty());
    }

    #[test]
    fn query_and_set_headers_helpers_patch_delete_and_replace() {
        let rt = Runtime::new().unwrap();
        let cx = Context::full(&rt).unwrap();
        cx.with(|cx| {
            let patch: Value = cx.eval("({ debug: 1, tag: null, token: 'abc' })").unwrap();
            let updated =
                apply_query_mutation(&cx, "/api/items?tag=old&keep=yes#sec", patch).unwrap();
            assert_eq!(updated, "/api/items?keep=yes&debug=1&token=abc#sec");

            let replaced_val: Value = cx.eval("([['a', '1'], ['a', '2']])").unwrap();
            let replaced = apply_query_mutation(&cx, updated.as_str(), replaced_val).unwrap();
            assert_eq!(replaced, "/api/items?a=1&a=2#sec");

            let cur_hdrs = vec![
                ("Authorization".into(), "Bearer old".into()),
                ("X-Remove".into(), "gone".into()),
            ];
            let sh: Value = cx
                .eval("({ authorization: 'Bearer new', 'x-remove': null, 'X-Added': 42 })")
                .unwrap();
            let (mut next_hdrs, touched_ct) = apply_set_headers(&cx, &cur_hdrs, sh).unwrap();
            assert!(!touched_ct);
            assert_eq!(
                next_hdrs,
                vec![
                    ("authorization".into(), "Bearer new".into()),
                    ("X-Added".into(), "42".into()),
                ]
            );
            ensure_default_content_type(
                &mut next_hdrs,
                "application/json; charset=utf-8",
                "application/json",
            );
            assert!(
                next_hdrs
                    .iter()
                    .any(|(k, v)| k == "Content-Type" && v == "application/json; charset=utf-8")
            );
        });
    }
}
