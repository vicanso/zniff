//! Adaptive debounce & throttling scheduler for large-session UI refreshes (H2).
//!
//! Keeps interactive user actions (typing a filter, changing sort/language,
//! opening facets) instantaneous while coalescing high-frequency incoming-flow
//! ticks and secondary aggregations (`facets`, autocomplete catalog, `stats`)
//! when a session grows to thousands or tens of thousands of flows.

use std::time::{Duration, SystemTime};

/// Session size above which incoming-flow-only refreshes begin coalescing.
pub(crate) const LARGE_SESSION_THRESHOLD: usize = 2_000;
/// Session size above which stronger coalescing applies.
pub(crate) const HUGE_SESSION_THRESHOLD: usize = 20_000;

/// Returns the minimum interval between incoming-flow-only list refreshes.
pub(crate) fn list_debounce(flow_count: usize) -> Duration {
    if flow_count >= HUGE_SESSION_THRESHOLD {
        Duration::from_millis(500)
    } else if flow_count >= LARGE_SESSION_THRESHOLD {
        Duration::from_millis(250)
    } else {
        Duration::ZERO
    }
}

/// Returns the minimum interval between incoming-flow-only secondary aggregations
/// (`SessionFacets`, `FilterSuggestionsCatalog`, `FilteredStats`).
pub(crate) fn secondary_debounce(flow_count: usize) -> Duration {
    if flow_count >= HUGE_SESSION_THRESHOLD {
        Duration::from_millis(1_000)
    } else if flow_count >= LARGE_SESSION_THRESHOLD {
        Duration::from_millis(500)
    } else {
        Duration::ZERO
    }
}

/// What the periodic UI tick should recompute on this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RefreshDecision {
    pub(crate) run_list: bool,
    pub(crate) run_secondary: bool,
}

/// Tracks last refresh timestamps to coalesce background ticks on large sessions.
#[derive(Debug, Default)]
pub(crate) struct RefreshThrottle {
    last_list_at: Option<SystemTime>,
    last_secondary_at: Option<SystemTime>,
    secondary_dirty: bool,
}

impl RefreshThrottle {
    /// Decides whether this tick should run list projection and/or secondary aggregations.
    ///
    /// - `interactive_changed`: filter, sort, language, path display, facets opened, or spill GC.
    ///   Always runs immediately without debouncing.
    /// - `gen_changed`: new/updated flows arrived since last rendered generation.
    pub(crate) fn decide(
        &mut self,
        now: SystemTime,
        flow_count: usize,
        interactive_changed: bool,
        gen_changed: bool,
    ) -> RefreshDecision {
        if interactive_changed {
            self.last_list_at = Some(now);
            self.last_secondary_at = Some(now);
            self.secondary_dirty = false;
            return RefreshDecision {
                run_list: true,
                run_secondary: true,
            };
        }
        let sec_min = secondary_debounce(flow_count);
        let sec_due = self
            .last_secondary_at
            .and_then(|at| now.duration_since(at).ok())
            .is_none_or(|elapsed| elapsed >= sec_min);

        if !gen_changed {
            if self.secondary_dirty && sec_due {
                self.last_secondary_at = Some(now);
                self.secondary_dirty = false;
                return RefreshDecision {
                    run_list: true,
                    run_secondary: true,
                };
            }
            return RefreshDecision {
                run_list: false,
                run_secondary: false,
            };
        }
        let list_min = list_debounce(flow_count);
        let list_due = self
            .last_list_at
            .and_then(|at| now.duration_since(at).ok())
            .is_none_or(|elapsed| elapsed >= list_min);
        if !list_due {
            return RefreshDecision {
                run_list: false,
                run_secondary: false,
            };
        }
        self.last_list_at = Some(now);
        if sec_due {
            self.last_secondary_at = Some(now);
            self.secondary_dirty = false;
        } else {
            self.secondary_dirty = true;
        }
        RefreshDecision {
            run_list: true,
            run_secondary: sec_due,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sessions_refresh_every_tick_without_debounce() {
        let mut throttle = RefreshThrottle::default();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let d1 = throttle.decide(t0, 500, false, true);
        assert_eq!(
            d1,
            RefreshDecision {
                run_list: true,
                run_secondary: true,
            }
        );
        let d2 = throttle.decide(t0 + Duration::from_millis(50), 500, false, true);
        assert_eq!(
            d2,
            RefreshDecision {
                run_list: true,
                run_secondary: true,
            }
        );
    }

    #[test]
    fn large_sessions_coalesce_incoming_flows_and_secondary_stats() {
        let mut throttle = RefreshThrottle::default();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);

        // First tick runs both.
        assert_eq!(
            throttle.decide(t0, 5_000, false, true),
            RefreshDecision {
                run_list: true,
                run_secondary: true,
            }
        );
        // 100ms later: deferred (< 250ms).
        assert_eq!(
            throttle.decide(t0 + Duration::from_millis(100), 5_000, false, true),
            RefreshDecision {
                run_list: false,
                run_secondary: false,
            }
        );
        // 260ms later: list runs, secondary still deferred (< 500ms).
        assert_eq!(
            throttle.decide(t0 + Duration::from_millis(260), 5_000, false, true),
            RefreshDecision {
                run_list: true,
                run_secondary: false,
            }
        );
        // 520ms later: both list and secondary run.
        assert_eq!(
            throttle.decide(t0 + Duration::from_millis(520), 5_000, false, true),
            RefreshDecision {
                run_list: true,
                run_secondary: true,
            }
        );
        // Interactive filter edit always bypasses debounce immediately.
        assert_eq!(
            throttle.decide(t0 + Duration::from_millis(530), 100_000, true, false),
            RefreshDecision {
                run_list: true,
                run_secondary: true,
            }
        );
    }
}
