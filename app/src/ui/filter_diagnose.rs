//! Empty filtered list diagnosis: identifies which filter token(s) eliminated
//! all flows in the current session so the empty state can guide the user.

use crate::i18n::Lang;
use crate::session::{self, FilterPlan, Session};

const MAX_DIAGNOSED_TOKENS: usize = 16;
const MAX_DISPLAY_TOKENS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct EmptyFilterDiagnosis {
    /// Total flows currently stored in the session.
    pub(crate) total_flows: usize,
    /// Individual tokens that match 0 flows in the session (deduplicated, in order).
    pub(crate) blocking_tokens: Vec<String>,
    /// True when each individual token matches >= 1 flow, so only their combination yields 0.
    pub(crate) combined_mismatch: bool,
    /// True when both primary and secondary filters are active.
    pub(crate) both_filters_active: bool,
    /// True when the active primary filter alone matches 0 flows (only set when both active).
    pub(crate) primary_empty: bool,
    /// True when the active secondary filter alone matches 0 flows (only set when both active).
    pub(crate) secondary_empty: bool,
}

/// Deferred diagnosis plan whose [`FilterDiagnosisPlan::finish`] can run outside
/// the session mutex (for spilled bodies on disk).
pub(crate) struct FilterDiagnosisPlan {
    total_flows: usize,
    both_filters_active: bool,
    token_plans: Vec<(String, FilterPlan)>,
    primary_plan: Option<FilterPlan>,
    secondary_plan: Option<FilterPlan>,
}

impl FilterDiagnosisPlan {
    pub(crate) fn finish(self) -> EmptyFilterDiagnosis {
        if self.total_flows == 0 || self.token_plans.is_empty() {
            return EmptyFilterDiagnosis {
                total_flows: self.total_flows,
                ..EmptyFilterDiagnosis::default()
            };
        }
        let mut blocking_tokens = Vec::new();
        for (token, plan) in self.token_plans {
            if plan.finish().is_empty() {
                blocking_tokens.push(token);
            }
        }
        let primary_empty = self.primary_plan.is_some_and(|p| p.finish().is_empty());
        let secondary_empty = self.secondary_plan.is_some_and(|p| p.finish().is_empty());
        let combined_mismatch = blocking_tokens.is_empty();
        EmptyFilterDiagnosis {
            total_flows: self.total_flows,
            blocking_tokens,
            combined_mismatch,
            both_filters_active: self.both_filters_active,
            primary_empty,
            secondary_empty,
        }
    }
}

/// Splits a filter expression into atomic AND tokens using the same pipe/comma OR
/// joining rules as `session::FlowStage::parse`.
pub(crate) fn split_filter_tokens(filter: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in filter
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let normalized = session::normalize_filter_pipes(line);
        for token in join_comma_or_tokens(&normalized) {
            if !token.is_empty() {
                out.push(token);
            }
        }
    }
    out
}

fn join_comma_or_tokens(normalized: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut join_next = false;
    for raw in session::split_filter_words(normalized) {
        if raw == "," {
            join_next = true;
            continue;
        }
        if let Some(rest) = raw.strip_prefix(',') {
            if let Some(last) = tokens.last_mut() {
                last.push(',');
                last.push_str(rest);
            } else if !rest.is_empty() {
                tokens.push(rest.to_string());
            }
            join_next = raw.ends_with(',');
            continue;
        }
        if join_next {
            if let Some(last) = tokens.last_mut() {
                if !last.ends_with(',') {
                    last.push(',');
                }
                last.push_str(&raw);
            } else {
                tokens.push(raw);
            }
            join_next = tokens.last().is_some_and(|t| t.ends_with(','));
            continue;
        }
        join_next = raw.ends_with(',');
        tokens.push(raw);
    }
    tokens
}

/// Builds a [`FilterDiagnosisPlan`] under the session lock.
pub(crate) fn plan_empty_filter_diagnosis(
    session: &Session,
    primary_filter: &str,
    primary_enabled: bool,
    secondary_filter: &str,
) -> FilterDiagnosisPlan {
    let total_flows = session.len();
    let active_primary = if primary_enabled {
        primary_filter.trim()
    } else {
        ""
    };
    let active_secondary = secondary_filter.trim();
    if total_flows == 0 || (active_primary.is_empty() && active_secondary.is_empty()) {
        return FilterDiagnosisPlan {
            total_flows,
            both_filters_active: false,
            token_plans: Vec::new(),
            primary_plan: None,
            secondary_plan: None,
        };
    }

    let both_filters_active = !active_primary.is_empty() && !active_secondary.is_empty();
    let mut unique_tokens = Vec::new();
    for src in [active_primary, active_secondary] {
        for tok in split_filter_tokens(src) {
            if unique_tokens.len() >= MAX_DIAGNOSED_TOKENS {
                break;
            }
            if !unique_tokens
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&tok))
            {
                unique_tokens.push(tok);
            }
        }
    }

    let token_plans = unique_tokens
        .into_iter()
        .map(|tok| {
            let plan = session.plan_filter(&tok);
            (tok, plan)
        })
        .collect();
    let primary_plan = both_filters_active.then(|| session.plan_filter(active_primary));
    let secondary_plan = both_filters_active.then(|| session.plan_filter(active_secondary));

    FilterDiagnosisPlan {
        total_flows,
        both_filters_active,
        token_plans,
        primary_plan,
        secondary_plan,
    }
}

/// Pure helper that immediately evaluates [`plan_empty_filter_diagnosis`].
#[cfg(test)]
pub(crate) fn diagnose_empty_filter(
    session: &Session,
    primary_filter: &str,
    primary_enabled: bool,
    secondary_filter: &str,
) -> EmptyFilterDiagnosis {
    plan_empty_filter_diagnosis(session, primary_filter, primary_enabled, secondary_filter).finish()
}

/// Formats a concise CN/EN diagnosis sentence for the empty filtered list state.
/// Returns `None` when the session is empty or no filter diagnosis applies.
pub(crate) fn format_empty_filter_diagnosis(
    lang: Lang,
    diag: &EmptyFilterDiagnosis,
) -> Option<String> {
    if diag.total_flows == 0 {
        return None;
    }
    if !diag.blocking_tokens.is_empty() {
        let total = crate::present::format_thousands(diag.total_flows as u64);
        if diag.blocking_tokens.len() == 1 {
            let tok = &diag.blocking_tokens[0];
            return Some(match lang {
                Lang::Zh => format!("共 {total} 条流量，过滤词「{tok}」未匹配到任何记录"),
                Lang::En => format!("{total} flows in session, but \"{tok}\" matched 0"),
            });
        }
        let shown: Vec<&str> = diag
            .blocking_tokens
            .iter()
            .take(MAX_DISPLAY_TOKENS)
            .map(String::as_str)
            .collect();
        let extra = diag.blocking_tokens.len().saturating_sub(shown.len());
        let extra_str = crate::present::format_thousands(extra as u64);
        return Some(match lang {
            Lang::Zh => {
                let joined = shown.join("、");
                if extra > 0 {
                    format!("共 {total} 条流量，以下过滤词未匹配：{joined} 等 {extra_str} 项")
                } else {
                    format!("共 {total} 条流量，以下过滤词未匹配：{joined}")
                }
            }
            Lang::En => {
                let joined = shown.join(", ");
                if extra > 0 {
                    format!("{total} flows in session; 0 matches for: {joined} (+{extra_str} more)")
                } else {
                    format!("{total} flows in session; 0 matches for: {joined}")
                }
            }
        });
    }
    if diag.combined_mismatch {
        let total = crate::present::format_thousands(diag.total_flows as u64);
        if diag.both_filters_active && !diag.primary_empty && !diag.secondary_empty {
            return Some(match lang {
                Lang::Zh => format!("共 {total} 条流量，主过滤与二次过滤叠加后无交集"),
                Lang::En => {
                    format!(
                        "{total} flows in session; primary and secondary filters do not overlap"
                    )
                }
            });
        }
        return Some(match lang {
            Lang::Zh => format!("共 {total} 条流量，单个过滤词均有匹配，但组合条件无交集"),
            Lang::En => format!(
                "{total} flows in session; each token matches individually, but their combination matches 0"
            ),
        });
    }
    None
}
