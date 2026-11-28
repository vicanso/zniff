//! Scoped QuickJS host SDK (`zniff.flows`, `zniff.codec`, `zniff.crypto`) with
//! tiered per-stage read quotas and lock-free `zniff-spill` `pread` access.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rquickjs::function::Opt;
use rquickjs::{Array, Ctx, Exception, Function, Object, Result as JsResult, Value};
use tokio::sync::watch;

use crate::hook_codec_crypto::{build_codec_object, build_crypto_object, to_uint8_array};
use crate::hook_spec::HookStage;
use crate::session::{Direction, Session};

const LIVE_DEFAULT_IDS_LIMIT: usize = 200;
const LIVE_MAX_IDS_LIMIT: usize = 500;
const LIVE_MAX_BODY_READS: usize = 16;
const LIVE_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

const OFFLINE_DEFAULT_IDS_LIMIT: usize = 200;
const OFFLINE_MAX_IDS_LIMIT: usize = 2000;
const OFFLINE_MAX_BODY_READS: usize = 500;
const OFFLINE_MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Cloneable session host handle captured per hook call without retaining `Arc<Hub>`.
#[derive(Clone)]
pub(crate) struct HookSessionHost {
    session: Arc<Mutex<Session>>,
    generation: Arc<AtomicU64>,
    state_tx: watch::Sender<u64>,
}

impl HookSessionHost {
    pub(crate) fn new(
        session: Arc<Mutex<Session>>,
        generation: Arc<AtomicU64>,
        state_tx: watch::Sender<u64>,
    ) -> Self {
        Self {
            session,
            generation,
            state_tx,
        }
    }

    fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.state_tx.send_modify(|v| *v = v.wrapping_add(1));
    }

    fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        let (out, rev_changed) = {
            let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
            let before = session.rev();
            let out = f(&mut session);
            let after = session.rev();
            (out, after != before)
        };
        if rev_changed {
            self.state_tx.send_modify(|v| *v = v.wrapping_add(1));
        }
        out
    }

    fn read_body(&self, id: u64, dir: Direction) -> Option<Vec<u8>> {
        let plan = self.with_session(|session| session.plan_body_read(id, dir))?;
        Some(plan.finish())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HookQuota {
    pub default_ids_limit: usize,
    pub max_ids_limit: usize,
    pub max_body_reads: usize,
    pub max_body_bytes: usize,
}

impl HookQuota {
    pub(crate) fn for_stage(stage: HookStage) -> Self {
        match stage {
            HookStage::Request | HookStage::Response => Self {
                default_ids_limit: LIVE_DEFAULT_IDS_LIMIT,
                max_ids_limit: LIVE_MAX_IDS_LIMIT,
                max_body_reads: LIVE_MAX_BODY_READS,
                max_body_bytes: LIVE_MAX_BODY_BYTES,
            },
            HookStage::Preview | HookStage::Run => Self {
                default_ids_limit: OFFLINE_DEFAULT_IDS_LIMIT,
                max_ids_limit: OFFLINE_MAX_IDS_LIMIT,
                max_body_reads: OFFLINE_MAX_BODY_READS,
                max_body_bytes: OFFLINE_MAX_BODY_BYTES,
            },
        }
    }
}

struct ActiveHookCall {
    host: Option<HookSessionHost>,
    quota: HookQuota,
    body_reads_used: usize,
    body_bytes_used: usize,
}

thread_local! {
    static ACTIVE_HOOK_CALL: RefCell<Option<ActiveHookCall>> = const { RefCell::new(None) };
}

/// RAII scope guard binding session access and per-invocation quotas on the worker thread.
pub(crate) struct HookCallGuard;

impl HookCallGuard {
    pub(crate) fn enter(host: Option<HookSessionHost>, stage: HookStage) -> Self {
        ACTIVE_HOOK_CALL.with(|cell| {
            *cell.borrow_mut() = Some(ActiveHookCall {
                host,
                quota: HookQuota::for_stage(stage),
                body_reads_used: 0,
                body_bytes_used: 0,
            });
        });
        Self
    }
}

impl Drop for HookCallGuard {
    fn drop(&mut self) {
        ACTIVE_HOOK_CALL.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Installs the global `$` and `zniff` SDK bindings (`flows`, `codec`, `crypto`)
/// pointing to the same object before `SANDBOX_PRELUDE_LOCKDOWN` freezes it.
pub(crate) fn install_zniff_globals<'js>(cx: &Ctx<'js>) -> JsResult<()> {
    let sdk = Object::new(cx.clone())?;
    sdk.set("flows", build_flows_object(cx)?)?;
    sdk.set("codec", build_codec_object(cx)?)?;
    sdk.set("crypto", build_crypto_object(cx)?)?;
    let globals = cx.globals();
    globals.set("zniff", sdk.clone())?;
    globals.set("$", sdk)
}

fn require_host<'js>(cx: &Ctx<'js>) -> JsResult<HookSessionHost> {
    let host = ACTIVE_HOOK_CALL.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|active| active.host.clone())
    });
    host.ok_or_else(|| {
        Exception::throw_message(cx, "zniff.flows is unavailable outside an active hook")
    })
}

fn check_and_charge_body_read<'js>(cx: &Ctx<'js>, byte_len: usize) -> JsResult<()> {
    let err_msg = ACTIVE_HOOK_CALL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(active) = borrow.as_mut() else {
            return Some("zniff.flows is unavailable outside an active hook".to_string());
        };
        if active.body_reads_used >= active.quota.max_body_reads {
            return Some(format!(
                "zniff.flows body read count quota exceeded (max {})",
                active.quota.max_body_reads
            ));
        }
        let next_bytes = active.body_bytes_used.saturating_add(byte_len);
        if next_bytes > active.quota.max_body_bytes {
            return Some(format!(
                "zniff.flows body byte quota exceeded (max {} bytes)",
                active.quota.max_body_bytes
            ));
        }
        active.body_reads_used += 1;
        active.body_bytes_used = next_bytes;
        None
    });
    if let Some(msg) = err_msg {
        Err(Exception::throw_range(cx, &msg))
    } else {
        Ok(())
    }
}

fn build_flows_object<'js>(cx: &Ctx<'js>) -> JsResult<Object<'js>> {
    let obj = Object::new(cx.clone())?;

    obj.set(
        "ids",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, filter: Opt<String>, limit: Opt<i64>| -> JsResult<Vec<f64>> {
                let host = require_host(&cx)?;
                let quota = ACTIVE_HOOK_CALL
                    .with(|c| c.borrow().as_ref().map(|a| a.quota))
                    .unwrap_or_else(|| HookQuota::for_stage(HookStage::Request));
                let lim = match limit.0 {
                    Some(n) if n <= 0 => return Ok(Vec::<f64>::new()),
                    Some(n) => (n as usize).clamp(1, quota.max_ids_limit),
                    None => quota.default_ids_limit,
                };
                let filter_str = filter.0.unwrap_or_default();
                let (plan, marker_ids): (_, Vec<u64>) = host.with_session(|session| {
                    let plan = session.plan_filter(&filter_str);
                    (plan, Vec::new())
                });
                let _ = marker_ids;
                let matched = plan.finish();
                let ids: Vec<f64> = host.with_session(|session| {
                    matched
                        .into_iter()
                        .rev()
                        .filter_map(|(id, _)| {
                            let flow = session.get(id)?;
                            (!flow.is_marker()).then_some(id as f64)
                        })
                        .take(lim)
                        .collect()
                });
                Ok(ids)
            },
        )?,
    )?;

    obj.set(
        "url",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, id: u64| -> JsResult<Option<String>> {
                let host = require_host(&cx)?;
                let url = host.with_session(|session| {
                    let flow = session.get(id)?;
                    (!flow.is_marker()).then(|| flow.url.clone())
                });
                Ok(url)
            },
        )?,
    )?;

    obj.set(
        "info",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, id: u64| -> JsResult<Value<'js>> {
                let host = require_host(&cx)?;
                let snapshot = host.with_session(|session| {
                    let flow = session.get(id)?;
                    if flow.is_marker() {
                        return None;
                    }
                    Some((
                        flow.id,
                        flow.protocol.label().to_string(),
                        flow.method.clone(),
                        flow.url.clone(),
                        flow.host.clone(),
                        flow.path.clone(),
                        flow.status,
                        flow.error.clone(),
                        flow.client_ip.clone(),
                        flow.elapsed_ms(),
                        flow.req_bytes(),
                        flow.res_bytes(),
                        flow.comment.clone(),
                        flow.pinned,
                    ))
                });
                let Some((
                    fid,
                    protocol,
                    method,
                    url,
                    host_name,
                    path,
                    status,
                    error,
                    client_ip,
                    elapsed_ms,
                    req_bytes,
                    res_bytes,
                    comment,
                    pinned,
                )) = snapshot
                else {
                    return Ok(Value::new_null(cx.clone()));
                };
                let info = Object::new(cx.clone())?;
                info.set("id", fid as f64)?;
                info.set("protocol", protocol)?;
                info.set("method", method)?;
                info.set("url", url)?;
                info.set("host", host_name)?;
                info.set("path", path)?;
                info.set("status", status.map(u16::into) as Option<u32>)?;
                info.set("error", error)?;
                info.set("clientIp", client_ip)?;
                info.set("elapsedMs", elapsed_ms.map(|v| v as f64))?;
                info.set("reqBytes", req_bytes as f64)?;
                info.set("resBytes", res_bytes as f64)?;
                info.set("comment", comment)?;
                info.set("pinned", pinned)?;
                Ok(info.into_value())
            },
        )?,
    )?;

    register_header_fns(&obj, cx, Direction::Request)?;
    register_header_fns(&obj, cx, Direction::Response)?;
    register_body_fns(&obj, cx, Direction::Request)?;
    register_body_fns(&obj, cx, Direction::Response)?;

    obj.set(
        "setComment",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, id: u64, comment: String| -> JsResult<bool> {
                let host = require_host(&cx)?;
                let ok = host.with_session(|session| session.set_comment(id, comment));
                if ok {
                    host.touch();
                }
                Ok(ok)
            },
        )?,
    )?;
    obj.set(
        "appendComment",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, id: u64, note: String| -> JsResult<bool> {
                let host = require_host(&cx)?;
                let trimmed = note.trim();
                if trimmed.is_empty() {
                    return Ok(false);
                }
                let ok = host.with_session(|session| {
                    let Some(flow) = session.get(id) else {
                        return false;
                    };
                    if flow.comment.contains(trimmed) {
                        return false;
                    }
                    let next = if flow.comment.is_empty() {
                        trimmed.to_string()
                    } else {
                        format!("{} {trimmed}", flow.comment)
                    };
                    session.set_comment(id, next)
                });
                if ok {
                    host.touch();
                }
                Ok(ok)
            },
        )?,
    )?;
    obj.set(
        "setPinned",
        Function::new(
            cx.clone(),
            |cx: Ctx<'js>, id: u64, pinned: bool| -> JsResult<bool> {
                let host = require_host(&cx)?;
                let ok = host.with_session(|session| session.set_pinned(id, pinned));
                if ok {
                    host.touch();
                }
                Ok(ok)
            },
        )?,
    )?;

    Ok(obj)
}

fn register_header_fns<'js>(obj: &Object<'js>, cx: &Ctx<'js>, dir: Direction) -> JsResult<()> {
    let (all_name, one_name) = match dir {
        Direction::Request => ("requestHeaders", "requestHeader"),
        Direction::Response => ("responseHeaders", "responseHeader"),
    };

    obj.set(
        all_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, id: u64| -> JsResult<Value<'js>> {
                let host = require_host(&cx)?;
                let pairs = host.with_session(|session| {
                    let flow = session.get(id)?;
                    if flow.is_marker() {
                        return None;
                    }
                    Some(match dir {
                        Direction::Request => flow.request_headers.clone(),
                        Direction::Response => flow.response_headers.clone(),
                    })
                });
                let Some(pairs) = pairs else {
                    return Ok(Value::new_null(cx.clone()));
                };
                let arr = Array::new(cx.clone())?;
                for (idx, (k, v)) in pairs.into_iter().enumerate() {
                    let pair = Array::new(cx.clone())?;
                    pair.set(0, k)?;
                    pair.set(1, v)?;
                    arr.set(idx, pair)?;
                }
                Ok(arr.into_value())
            },
        )?,
    )?;

    obj.set(
        one_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, id: u64, name: String| -> JsResult<Option<String>> {
                let host = require_host(&cx)?;
                let target = name.trim();
                let val = host.with_session(|session| {
                    let flow = session.get(id)?;
                    if flow.is_marker() {
                        return None;
                    }
                    let hdrs = match dir {
                        Direction::Request => &flow.request_headers,
                        Direction::Response => &flow.response_headers,
                    };
                    hdrs.iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(target))
                        .map(|(_, v)| v.clone())
                });
                Ok(val)
            },
        )?,
    )
}

fn register_body_fns<'js>(obj: &Object<'js>, cx: &Ctx<'js>, dir: Direction) -> JsResult<()> {
    let (bytes_name, text_name) = match dir {
        Direction::Request => ("requestBody", "requestText"),
        Direction::Response => ("responseBody", "responseText"),
    };

    obj.set(
        bytes_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, id: u64| -> JsResult<Value<'js>> {
                let host = require_host(&cx)?;
                let approx_len = host.with_session(|session| {
                    session.get(id).map(|f| match dir {
                        Direction::Request => f.req_bytes(),
                        Direction::Response => f.res_bytes(),
                    })
                });
                let Some(approx) = approx_len else {
                    return Ok(Value::new_null(cx.clone()));
                };
                check_and_charge_body_read(&cx, approx)?;
                let Some(bytes) = host.read_body(id, dir) else {
                    return Ok(Value::new_null(cx.clone()));
                };
                let ta = to_uint8_array(&cx, bytes)?;
                Ok(ta.into_value())
            },
        )?,
    )?;

    obj.set(
        text_name,
        Function::new(
            cx.clone(),
            move |cx: Ctx<'js>, id: u64| -> JsResult<Option<String>> {
                let host = require_host(&cx)?;
                let approx_len = host.with_session(|session| {
                    session.get(id).map(|f| match dir {
                        Direction::Request => f.req_bytes(),
                        Direction::Response => f.res_bytes(),
                    })
                });
                let Some(approx) = approx_len else {
                    return Ok(None);
                };
                check_and_charge_body_read(&cx, approx)?;
                let Some(bytes) = host.read_body(id, dir) else {
                    return Ok(None);
                };
                Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
            },
        )?,
    )
}
