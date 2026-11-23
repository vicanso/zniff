//! macOS `NETransparentProxyProvider` System Extension host controller.
//!
//! When zniff is packaged as a Developer ID `.app` containing
//! `dev.zniff.app.transparent-proxy.systemextension`, this module drives
//! `OSSystemExtensionManager` activation and `NETransparentProxyManager`
//! lifecycle via the Objective-C bridge compiled from `macos/transparent-proxy/host_bridge.m`.
//! On non-macOS targets, headless builds, or unsigned local builds without the
//! embedded extension, it safely reports `embedded: false` and remains hidden.

use std::sync::atomic::AtomicBool;
#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TpState {
    #[default]
    Idle,
    NeedsApproval,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransparentProxyStatus {
    pub embedded: bool,
    pub in_applications: bool,
    pub state: TpState,
    pub active: bool,
    pub error: String,
}

impl TransparentProxyStatus {
    pub fn is_on_or_starting(&self) -> bool {
        self.active || matches!(self.state, TpState::Running | TpState::Starting)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningApp {
    pub name: String,
    pub bundle_id: String,
    pub exec_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransparentProxyConfig {
    pub port: u16,
    pub proxy_auth: String,
    pub capture_all: bool,
    pub targets: Vec<String>,
    pub block_quic: bool,
}

/// Splits comma / newline / whitespace-separated target app names or bundle IDs.
pub fn parse_targets(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split([',', '\n', ';']) {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// Checks if `candidate` (or its exact name / bundle ID) is currently selected in `raw`.
pub fn has_target(raw: &str, candidate: &str) -> bool {
    let needle = candidate.trim();
    if needle.is_empty() {
        return false;
    }
    parse_targets(raw)
        .iter()
        .any(|item| item.eq_ignore_ascii_case(needle))
}

/// Toggles `item` inside a comma-separated target list string, preserving order.
#[cfg_attr(not(test), allow(dead_code))]
pub fn toggle_target(raw: &str, item: &str) -> String {
    toggle_target_aliases(raw, item, &[item])
}

/// Toggles a target group represented by `aliases` (keeping `preferred` when adding),
/// removing any matching alias when already present.
pub fn toggle_target_aliases(raw: &str, preferred: &str, aliases: &[&str]) -> String {
    let mut items = parse_targets(raw);
    let any_present = items.iter().any(|existing| {
        existing.eq_ignore_ascii_case(preferred.trim())
            || aliases.iter().any(|alias| {
                !alias.trim().is_empty() && existing.eq_ignore_ascii_case(alias.trim())
            })
    });
    if any_present {
        items.retain(|existing| {
            !existing.eq_ignore_ascii_case(preferred.trim())
                && !aliases.iter().any(|alias| {
                    !alias.trim().is_empty() && existing.eq_ignore_ascii_case(alias.trim())
                })
        });
    } else {
        let trimmed = preferred.trim();
        if !trimmed.is_empty() {
            items.push(trimmed.to_string());
        }
    }
    items.join(", ")
}

fn status_cache() -> &'static Mutex<TransparentProxyStatus> {
    static SLOT: OnceLock<Mutex<TransparentProxyStatus>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(TransparentProxyStatus::default()))
}

fn running_apps_cache() -> &'static Mutex<Vec<RunningApp>> {
    static SLOT: OnceLock<Mutex<Vec<RunningApp>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

#[allow(dead_code)]
static OP_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub fn cached_status() -> TransparentProxyStatus {
    status_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

fn store_status(next: TransparentProxyStatus) {
    *status_cache().lock().unwrap_or_else(|err| err.into_inner()) = next;
}

pub fn cached_running_apps() -> Vec<RunningApp> {
    running_apps_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

#[allow(dead_code)]
fn store_running_apps(apps: Vec<RunningApp>) {
    *running_apps_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = apps;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBundleStatus {
    #[serde(default)]
    embedded: bool,
    #[serde(default)]
    in_applications: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawManagerStatus {
    #[serde(default)]
    state: TpState,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    error: String,
}

#[allow(dead_code)]
fn parse_bundle_and_manager_status(
    bundle_json: &str,
    manager_json: &str,
) -> TransparentProxyStatus {
    let bundle: RawBundleStatus = serde_json::from_str(bundle_json).unwrap_or(RawBundleStatus {
        embedded: false,
        in_applications: false,
    });
    let mgr: RawManagerStatus = serde_json::from_str(manager_json).unwrap_or(RawManagerStatus {
        state: TpState::Idle,
        active: false,
        error: String::new(),
    });
    TransparentProxyStatus {
        embedded: bundle.embedded,
        in_applications: bundle.in_applications,
        state: mgr.state,
        active: mgr.active,
        error: mgr.error,
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    Started,
    NeedsApproval,
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
mod native {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        fn zniff_tp_bundle_status(buf: *mut c_char, len: usize) -> c_int;
        fn zniff_tp_get_status(buf: *mut c_char, len: usize) -> c_int;
        fn zniff_tp_activate_and_start(
            port: u16,
            proxy_auth: *const c_char,
            capture_all: bool,
            targets_json: *const c_char,
            block_quic: bool,
            err_buf: *mut c_char,
            err_len: usize,
        ) -> c_int;
        fn zniff_tp_update_config(
            port: u16,
            proxy_auth: *const c_char,
            capture_all: bool,
            targets_json: *const c_char,
            block_quic: bool,
        ) -> c_int;
        fn zniff_tp_stop() -> c_int;
        fn zniff_tp_list_running_apps(buf: *mut c_char, len: usize) -> c_int;
        #[cfg(test)]
        fn zniff_tp_resolve_targets_json(
            targets_json: *const c_char,
            buf: *mut c_char,
            len: usize,
        ) -> c_int;
        fn zniff_tp_open_system_settings();
    }

    fn read_c_buf<const N: usize>(fill: impl FnOnce(*mut c_char, usize) -> c_int) -> String {
        let mut buf = vec![0 as c_char; N];
        let _ = fill(buf.as_mut_ptr(), N);
        // SAFETY: native bridge always NUL-terminates within N bytes via strlcpy.
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    pub(super) fn query_bundle_status() -> String {
        read_c_buf::<2048>(|ptr, len| unsafe { zniff_tp_bundle_status(ptr, len) })
    }

    pub(super) fn query_manager_status() -> String {
        read_c_buf::<2048>(|ptr, len| unsafe { zniff_tp_get_status(ptr, len) })
    }

    pub(super) fn activate_and_start(
        cfg: &super::TransparentProxyConfig,
    ) -> Result<super::StartOutcome, String> {
        let auth = CString::new(cfg.proxy_auth.as_str()).unwrap_or_default();
        let targets = serde_json::to_string(&cfg.targets).unwrap_or_else(|_| "[]".to_string());
        let targets_c = CString::new(targets).unwrap_or_default();
        let mut err_buf = vec![0 as c_char; 1024];
        let rc = unsafe {
            zniff_tp_activate_and_start(
                cfg.port,
                auth.as_ptr(),
                cfg.capture_all,
                targets_c.as_ptr(),
                cfg.block_quic,
                err_buf.as_mut_ptr(),
                err_buf.len(),
            )
        };
        match rc {
            0 => Ok(super::StartOutcome::Started),
            2 => Ok(super::StartOutcome::NeedsApproval),
            _ => {
                let msg = unsafe { CStr::from_ptr(err_buf.as_ptr()) }
                    .to_string_lossy()
                    .trim()
                    .to_string();
                Err(if msg.is_empty() {
                    "激活透明代理系统扩展失败".to_string()
                } else {
                    msg
                })
            }
        }
    }

    pub(super) fn update_config(cfg: &super::TransparentProxyConfig) -> Result<(), String> {
        let auth = CString::new(cfg.proxy_auth.as_str()).unwrap_or_default();
        let targets = serde_json::to_string(&cfg.targets).unwrap_or_else(|_| "[]".to_string());
        let targets_c = CString::new(targets).unwrap_or_default();
        let rc = unsafe {
            zniff_tp_update_config(
                cfg.port,
                auth.as_ptr(),
                cfg.capture_all,
                targets_c.as_ptr(),
                cfg.block_quic,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err("更新透明代理配置失败".to_string())
        }
    }

    pub(super) fn stop() -> Result<(), String> {
        let rc = unsafe { zniff_tp_stop() };
        if rc == 0 {
            Ok(())
        } else {
            Err("停止透明代理失败".to_string())
        }
    }

    pub(super) fn list_running_apps() -> Vec<super::RunningApp> {
        let json = read_c_buf::<65536>(|ptr, len| unsafe { zniff_tp_list_running_apps(ptr, len) });
        serde_json::from_str(&json).unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn resolve_targets_json(targets: &[&str]) -> String {
        let raw = serde_json::to_string(targets).unwrap_or_else(|_| "[]".to_string());
        let raw_c = CString::new(raw).unwrap_or_default();
        read_c_buf::<65536>(|ptr, len| unsafe {
            zniff_tp_resolve_targets_json(raw_c.as_ptr(), ptr, len)
        })
    }

    pub(super) fn open_system_settings() {
        unsafe { zniff_tp_open_system_settings() };
    }
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn refresh_status() -> TransparentProxyStatus {
    let bundle_json = native::query_bundle_status();
    let bundle: RawBundleStatus = serde_json::from_str(&bundle_json).unwrap_or(RawBundleStatus {
        embedded: false,
        in_applications: false,
    });
    if !bundle.embedded {
        let st = TransparentProxyStatus {
            embedded: false,
            in_applications: bundle.in_applications,
            state: TpState::Idle,
            active: false,
            error: String::new(),
        };
        store_status(st.clone());
        return st;
    }
    let mgr_json = native::query_manager_status();
    let st = parse_bundle_and_manager_status(&bundle_json, &mgr_json);
    store_status(st.clone());
    st
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn refresh_status() -> TransparentProxyStatus {
    let st = TransparentProxyStatus::default();
    store_status(st.clone());
    st
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn refresh_running_apps() -> Vec<RunningApp> {
    let apps = native::list_running_apps();
    store_running_apps(apps.clone());
    apps
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn refresh_running_apps() -> Vec<RunningApp> {
    Vec::new()
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn start(cfg: &TransparentProxyConfig) -> Result<StartOutcome, String> {
    if OP_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Err("透明代理正在切换中".to_string());
    }
    let res = native::activate_and_start(cfg);
    OP_IN_FLIGHT.store(false, Ordering::Release);
    refresh_status();
    res
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn start(_cfg: &TransparentProxyConfig) -> Result<StartOutcome, String> {
    Err("当前构建未内嵌 macOS 透明代理系统扩展".to_string())
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn update_config(cfg: &TransparentProxyConfig) -> Result<(), String> {
    let res = native::update_config(cfg);
    refresh_status();
    res
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn update_config(_cfg: &TransparentProxyConfig) -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn stop() -> Result<(), String> {
    if OP_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Err("透明代理正在切换中".to_string());
    }
    let res = native::stop();
    OP_IN_FLIGHT.store(false, Ordering::Release);
    refresh_status();
    res
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn stop() -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32")))]
pub fn open_system_settings() {
    native::open_system_settings();
}

#[cfg(not(all(target_os = "macos", feature = "desktop", not(target_arch = "wasm32"))))]
pub fn open_system_settings() {}

pub fn cleanup_on_exit() {
    let st = cached_status();
    if st.embedded && st.is_on_or_starting() {
        let _ = stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_targets_splits_trims_and_deduplicates_case_insensitively() {
        let parsed = parse_targets("Safari, curl\nGoogle Chrome ; safari, , node");
        assert_eq!(parsed, vec!["Safari", "curl", "Google Chrome", "node"]);
    }

    #[test]
    fn toggle_target_adds_and_removes_case_insensitively() {
        let added = toggle_target("Safari, curl", "Slack");
        assert_eq!(added, "Safari, curl, Slack");
        assert!(has_target(&added, "slack"));

        let removed = toggle_target(&added, "CURL");
        assert_eq!(removed, "Safari, Slack");
        assert!(!has_target(&removed, "curl"));
    }

    #[test]
    fn parses_bundle_and_manager_status_json() {
        let st = parse_bundle_and_manager_status(
            r#"{"embedded":true,"inApplications":true}"#,
            r#"{"state":"needs_approval","active":false,"error":""}"#,
        );
        assert!(st.embedded);
        assert!(st.in_applications);
        assert_eq!(st.state, TpState::NeedsApproval);
        assert!(!st.active);
        assert!(!st.is_on_or_starting());

        let running = parse_bundle_and_manager_status(
            r#"{"embedded":true,"inApplications":true}"#,
            r#"{"state":"running","active":true,"error":""}"#,
        );
        assert_eq!(running.state, TpState::Running);
        assert!(running.is_on_or_starting());
    }

    #[test]
    fn toggle_target_aliases_removes_bundle_id_when_toggling_display_name() {
        let next = toggle_target_aliases(
            "cn.com.gf.etj, curl",
            "广发易淘金",
            &["广发易淘金", "cn.com.gf.etj", "gf_iphone"],
        );
        assert_eq!(next, "curl");

        let added = toggle_target_aliases(
            &next,
            "广发易淘金",
            &["广发易淘金", "cn.com.gf.etj", "gf_iphone"],
        );
        assert_eq!(added, "curl, 广发易淘金");
    }

    #[cfg(all(target_os = "macos", feature = "desktop"))]
    #[test]
    fn native_resolve_targets_expands_app_names_to_bundle_ids() {
        let json = native::resolve_targets_json(&["Safari", "广发易淘金"]);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let targets = parsed["targets"]
            .as_array()
            .expect("targets array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert!(targets.contains(&"Safari"), "{targets:?}");
        if std::path::Path::new("/Applications/Safari.app").exists() {
            assert!(targets.contains(&"com.apple.Safari"), "{targets:?}");
        }
        if std::path::Path::new("/Applications/广发易淘金.app").exists() {
            assert!(targets.contains(&"cn.com.gf.etj"), "{targets:?}");
            assert_eq!(
                parsed["appLabels"]["cn.com.gf.etj"].as_str(),
                Some("广发易淘金")
            );
        }
    }
}
