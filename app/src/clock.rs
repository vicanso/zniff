//! Wall clock for code that also runs in the browser.
//!
//! On `wasm32-unknown-unknown`, `SystemTime::now`, `SystemTime::elapsed`, and
//! `Instant::now` panic (`time not implemented on this platform`). The web
//! runtime exposes `Date.now()` through `nsh_core::time::unix_ms`.

use std::time::{Duration, SystemTime};

/// Current wall time. On native this is `SystemTime::now`.
pub fn now() -> SystemTime {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ms = nsh_core::time::unix_ms();
        let ms = if ms.is_finite() && ms > 0.0 {
            ms as u64
        } else {
            0
        };
        SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
    }
}

/// Time since `at`. On native this matches `at.elapsed().unwrap_or_default()`.
pub fn elapsed(at: SystemTime) -> Duration {
    now().duration_since(at).unwrap_or_default()
}
