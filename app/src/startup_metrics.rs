//! Cold-start and auto-capture recovery timing instrumentation (H3).
//!
//! Uses `crate::clock::now()` so instrumentation compiles and works cleanly on
//! both desktop and `wasm32` targets, and exposes a JSON-serializable snapshot
//! for [`crate::diagnostics::DiagnosticsBundle`].

use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

/// Cold-start and auto-capture recovery timings exported in diagnostics bundles.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct StartupMetricsSnapshot {
    /// Milliseconds from process start to root `App` component mount.
    pub process_to_app_mount_ms: Option<u64>,
    /// Milliseconds from process start to the first rendered UI frame/tick.
    pub process_to_first_frame_ms: Option<u64>,
    /// Whether `config.toml` had `capturing = true` and triggered auto-recovery.
    pub auto_capture_requested: bool,
    /// Milliseconds spent recovering capture (from spawn to listener bound / error).
    pub auto_capture_recovery_ms: Option<u64>,
    /// Total milliseconds from process start until auto-capture became ready.
    pub process_to_capture_ready_ms: Option<u64>,
    /// Outcome of auto-capture recovery (`Some(true)` = listening, `Some(false)` = failed).
    pub auto_capture_succeeded: Option<bool>,
}

#[derive(Debug, Default)]
pub(crate) struct StartupMetricsState {
    process_start: Option<SystemTime>,
    app_mount: Option<SystemTime>,
    first_frame: Option<SystemTime>,
    auto_capture_requested: bool,
    auto_capture_start: Option<SystemTime>,
    auto_capture_finish: Option<SystemTime>,
    auto_capture_succeeded: Option<bool>,
}

fn elapsed_ms(from: Option<SystemTime>, to: Option<SystemTime>) -> Option<u64> {
    let (start, end) = (from?, to?);
    let dur = end.duration_since(start).unwrap_or_default();
    Some(dur.as_millis().min(u128::from(u64::MAX)) as u64)
}

impl StartupMetricsState {
    pub(crate) fn record_process_start(&mut self, at: SystemTime) {
        self.process_start.get_or_insert(at);
    }

    pub(crate) fn record_app_mount(&mut self, at: SystemTime) {
        self.process_start.get_or_insert(at);
        self.app_mount.get_or_insert(at);
    }

    pub(crate) fn record_first_frame(&mut self, at: SystemTime) {
        self.process_start.get_or_insert(at);
        self.first_frame.get_or_insert(at);
    }

    pub(crate) fn record_auto_capture_start(&mut self, at: SystemTime) {
        self.process_start.get_or_insert(at);
        self.auto_capture_requested = true;
        self.auto_capture_start.get_or_insert(at);
    }

    pub(crate) fn record_auto_capture_finish(&mut self, at: SystemTime, succeeded: bool) {
        if !self.auto_capture_requested || self.auto_capture_finish.is_some() {
            return;
        }
        self.auto_capture_finish = Some(at);
        self.auto_capture_succeeded = Some(succeeded);
    }

    pub(crate) fn snapshot(&self) -> StartupMetricsSnapshot {
        StartupMetricsSnapshot {
            process_to_app_mount_ms: elapsed_ms(self.process_start, self.app_mount),
            process_to_first_frame_ms: elapsed_ms(self.process_start, self.first_frame),
            auto_capture_requested: self.auto_capture_requested,
            auto_capture_recovery_ms: elapsed_ms(self.auto_capture_start, self.auto_capture_finish),
            process_to_capture_ready_ms: if self.auto_capture_succeeded == Some(true) {
                elapsed_ms(self.process_start, self.auto_capture_finish)
            } else {
                None
            },
            auto_capture_succeeded: self.auto_capture_succeeded,
        }
    }
}

fn store() -> &'static Mutex<StartupMetricsState> {
    static STATE: OnceLock<Mutex<StartupMetricsState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(StartupMetricsState::default()))
}

/// Records process start timestamp (idempotent; first call wins).
pub fn mark_process_start() {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_process_start(crate::clock::now());
}

/// Records root `App` component mount timestamp (idempotent).
pub fn mark_app_mount() {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_app_mount(crate::clock::now());
}

/// Records first rendered UI frame/tick timestamp (idempotent).
pub fn mark_first_frame() {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_first_frame(crate::clock::now());
}

/// Records when boot-time auto-capture recovery starts.
#[cfg(not(target_arch = "wasm32"))]
pub fn mark_auto_capture_start() {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_auto_capture_start(crate::clock::now());
}

/// Records when boot-time auto-capture recovery completes.
#[cfg(not(target_arch = "wasm32"))]
pub fn mark_auto_capture_finish(succeeded: bool) {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record_auto_capture_finish(crate::clock::now(), succeeded);
}

/// Returns a point-in-time snapshot of cold-start and auto-capture metrics.
pub fn snapshot() -> StartupMetricsSnapshot {
    store().lock().unwrap_or_else(|e| e.into_inner()).snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn records_cold_start_and_auto_capture_recovery_idempotently() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut state = StartupMetricsState::default();
        state.record_process_start(t0);
        state.record_app_mount(t0 + Duration::from_millis(45));
        state.record_first_frame(t0 + Duration::from_millis(120));
        state.record_auto_capture_start(t0 + Duration::from_millis(50));
        state.record_auto_capture_finish(t0 + Duration::from_millis(185), true);

        // Subsequent calls must not overwrite the initial cold-start milestones.
        state.record_first_frame(t0 + Duration::from_secs(9));
        state.record_auto_capture_finish(t0 + Duration::from_secs(12), false);

        let snap = state.snapshot();
        assert_eq!(snap.process_to_app_mount_ms, Some(45));
        assert_eq!(snap.process_to_first_frame_ms, Some(120));
        assert!(snap.auto_capture_requested);
        assert_eq!(snap.auto_capture_recovery_ms, Some(135));
        assert_eq!(snap.process_to_capture_ready_ms, Some(185));
        assert_eq!(snap.auto_capture_succeeded, Some(true));
    }

    #[test]
    fn failed_auto_capture_omits_capture_ready_time() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
        let mut state = StartupMetricsState::default();
        state.record_process_start(t0);
        state.record_auto_capture_start(t0 + Duration::from_millis(30));
        state.record_auto_capture_finish(t0 + Duration::from_millis(90), false);

        let snap = state.snapshot();
        assert!(snap.auto_capture_requested);
        assert_eq!(snap.auto_capture_recovery_ms, Some(60));
        assert_eq!(snap.process_to_capture_ready_ms, None);
        assert_eq!(snap.auto_capture_succeeded, Some(false));
    }
}
