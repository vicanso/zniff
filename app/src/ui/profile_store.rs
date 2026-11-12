#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct QuickRuleModeSignals {
    /// Kinds `0` (Replace), `6` (Header), `11` (Json): `false` = Req, `true` = Res.
    pub(crate) req_res: Signal<bool>,
    /// Kind `2` (Local): `false` = Local, `true` = Fallback.
    pub(crate) local_fallback: Signal<bool>,
    /// Kind `10` (Cert): `false` = MITM, `true` = mTLS.
    pub(crate) mitm_mtls: Signal<bool>,
    /// Kind `13` (Block): `false` = Status, `true` = Drop.
    pub(crate) status_drop: Signal<bool>,
}

impl QuickRuleModeSignals {
    pub(crate) fn new() -> Self {
        Self {
            req_res: signal(true),
            local_fallback: signal(false),
            mitm_mtls: signal(false),
            status_drop: signal(false),
        }
    }

    pub(crate) fn reset(self) {
        self.req_res.set(true);
        self.local_fallback.set(false);
        self.mitm_mtls.set(false);
        self.status_drop.set(false);
    }

    pub(crate) fn signal_for(self, kind: u8) -> Signal<bool> {
        match kind % 14 {
            2 => self.local_fallback,
            10 => self.mitm_mtls,
            13 => self.status_drop,
            _ => self.req_res,
        }
    }

    pub(crate) fn mode_for(self, kind: u8) -> bool {
        self.signal_for(kind).get()
    }
}

/// Filters `from` and `to` down to the inputs actually rendered for `(kind, mode)`,
/// so hidden leftover text from another rule tab cannot mutate `ready_to_add` or `draft_rules`.
pub(crate) fn effective_quick_inputs<'a>(
    kind: u8,
    mode: bool,
    from: &'a str,
    to: &'a str,
) -> (&'a str, &'a str) {
    let k = kind % 14;
    let from_used = !matches!(k, 4 | 5 | 8 | 12) && !(k == 13 && mode);
    let to_used = matches!(k, 0 | 2 | 6 | 7 | 9 | 10 | 11);
    (
        if from_used { from } else { "" },
        if to_used { to } else { "" },
    )
}

pub(crate) fn effective_quick_op(kind: u8, op: &str) -> &str {
    if quick_kind(kind).supports_graphql_op() {
        op
    } else {
        ""
    }
}

pub(crate) fn clear_rule_draft(
    host: Signal<String>,
    path: Signal<String>,
    op: Signal<String>,
    from: Signal<String>,
    to: Signal<String>,
    modes: QuickRuleModeSignals,
    kind: Signal<u8>,
) {
    host.set(String::new());
    path.set(String::new());
    op.set(String::new());
    from.set(String::new());
    to.set(String::new());
    modes.reset();
    kind.set(0);
}

pub(crate) fn clear_working_setup_config(config: &mut config::Config) {
    config.active_profile.clear();
    config.filter.clear();
    config.bypass_hosts.clear();
    config.rules.clear();
    config.upstream_proxy.clear();
    config.rules_enabled = true;
    config.breaks_enabled = true;
    config.show_tunnel_failures = false;
}

pub(crate) fn deselect_active_profile(profiles: ProfileSignals, controls: &Controls) {
    record_profile_history(
        &profiles.name.get(),
        &profiles.filter.get(),
        &profiles.bypass.get(),
        &profiles.rules.get(),
        &profiles.upstream.get(),
        profiles.rules_on.get(),
        profiles.breaks_on.get(),
        profiles.show_tunnels.get(),
        profiles.history,
    );
    profiles
        .deselect_rev
        .update(|rev| *rev = rev.wrapping_add(1));
    profiles.name.set(String::new());
    profiles.filter.set(String::new());
    profiles.bypass.set(String::new());
    profiles.rules.set(String::new());
    profiles.upstream.set(String::new());
    profiles.rules_on.set(true);
    profiles.breaks_on.set(true);
    profiles.show_tunnels.set(false);
    controls.hub.set_bypass("");
    controls.hub.set_rules("");
    controls.hub.set_upstream("");
    controls.hub.set_rules_enabled(true);
    controls.hub.set_breaks_enabled(true);
    controls.hub.set_show_tunnel_failures(false);
    save_config_or_notify(controls, PrefName::Profiles, clear_working_setup_config);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn blank_working_setup(
    name: Signal<String>,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    tunnels: Signal<bool>,
    draft_host: Signal<String>,
    draft_path: Signal<String>,
    draft_op: Signal<String>,
    draft_from: Signal<String>,
    draft_to: Signal<String>,
    draft_modes: QuickRuleModeSignals,
    draft_kind: Signal<u8>,
    controls: &Controls,
) {
    name.set(String::new());
    filter.set(String::new());
    bypass.set(String::new());
    rules.set(String::new());
    upstream.set(String::new());
    clear_rule_draft(
        draft_host,
        draft_path,
        draft_op,
        draft_from,
        draft_to,
        draft_modes,
        draft_kind,
    );
    rules_on.set(true);
    breaks_on.set(true);
    tunnels.set(false);
    controls.hub.set_bypass("");
    controls.hub.set_rules("");
    controls.hub.set_upstream("");
    controls.hub.set_rules_enabled(true);
    controls.hub.set_breaks_enabled(true);
    controls.hub.set_show_tunnel_failures(false);
    save_config_or_notify(controls, PrefName::Profiles, clear_working_setup_config);
}

#[cfg(test)]
pub(crate) const SAVED_FILTER_CAP: usize = 16;

#[cfg(test)]
pub(crate) fn split_saved_filter(entry: &str) -> (&str, &str) {
    match entry.split_once('\t') {
        Some((name, expr)) => (name.trim(), expr.trim()),
        None => ("", entry.trim()),
    }
}

#[cfg(test)]
pub(crate) fn format_saved_filter(name: &str, expr: &str) -> String {
    let name = name.trim().replace('\t', " ");
    let name = name.trim();
    let expr = expr.trim();
    if name.is_empty() {
        expr.to_string()
    } else {
        format!("{name}\t{expr}")
    }
}

#[cfg(test)]
pub(crate) fn push_saved_filter_named(saved: &[String], name: &str, current: &str) -> Vec<String> {
    let expr = current.trim();
    if expr.is_empty() {
        return saved.to_vec();
    }
    let clean_name = name.trim().replace('\t', " ");
    let clean_name = clean_name.trim();
    let mut next: Vec<String> = saved
        .iter()
        .filter(|item| {
            let (existing_name, existing_expr) = split_saved_filter(item);
            existing_expr != expr
                && (clean_name.is_empty() || !existing_name.eq_ignore_ascii_case(clean_name))
        })
        .cloned()
        .collect();
    next.insert(0, format_saved_filter(clean_name, expr));
    next.truncate(SAVED_FILTER_CAP);
    next
}

#[cfg(test)]
pub(crate) fn push_saved_filter(saved: &[String], current: &str) -> Vec<String> {
    push_saved_filter_named(saved, "", current)
}

pub(crate) fn rule_preset_preview(rules: &str) -> String {
    let first = rules
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("");
    let count = crate::rules::parse_rules(rules).len();
    if count > 1 {
        format!("({count}) {first}")
    } else {
        first.to_string()
    }
}

#[cfg(test)]
pub(crate) fn push_saved_rule(
    saved: &[config::SavedRuleSet],
    name: &str,
    rules: &str,
) -> Vec<config::SavedRuleSet> {
    let rules = rules.trim();
    if rules.is_empty() {
        return saved.to_vec();
    }
    let name = name.trim();
    let resolved_name = if name.is_empty() {
        rules
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or("规则")
            .chars()
            .take(28)
            .collect::<String>()
    } else {
        name.to_string()
    };
    let mut next: Vec<config::SavedRuleSet> = saved
        .iter()
        .filter(|item| {
            item.rules.trim() != rules && !item.name.eq_ignore_ascii_case(&resolved_name)
        })
        .cloned()
        .collect();
    next.insert(
        0,
        config::SavedRuleSet {
            name: resolved_name,
            rules: rules.to_string(),
        },
    );
    next.truncate(SAVED_RULE_CAP);
    next
}

pub(crate) fn profile_preview(profile: &config::SavedProfile) -> String {
    let mut parts = Vec::new();
    let filter = profile.filter.trim();
    if !filter.is_empty() {
        parts.push(filter.to_string());
    }
    let rules = rule_preset_preview(&profile.rules);
    if !rules.is_empty() {
        parts.push(rules);
    }
    let bypass = profile.bypass_hosts.trim();
    if !bypass.is_empty() {
        parts.push(bypass.to_string());
    }
    let upstream = profile.upstream_proxy.trim();
    if !upstream.is_empty() {
        parts.push(upstream.to_string());
    }
    parts.join(" · ")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn store_saved_profile(
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    saved: Signal<Vec<config::SavedProfile>>,
    history: Signal<Vec<config::SavedProfile>>,
) -> std::io::Result<()> {
    let resolved_name = resolve_profile_name(name, filter.trim(), rules.trim());
    if let Some(old) = saved
        .get()
        .into_iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved_name))
    {
        record_profile_history(
            &old.name,
            &old.filter,
            &old.bypass_hosts,
            &old.rules,
            &old.upstream_proxy,
            old.rules_enabled,
            old.breaks_enabled,
            old.show_tunnel_failures,
            history,
        );
    }
    record_profile_history(
        &resolved_name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
        history,
    );
    let next = push_saved_profile(
        &saved.get(),
        name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    let saved_entry = next
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved_name))
        .cloned();
    let changed = next != saved.get();
    if changed {
        saved.set(next.clone());
    }
    let persisted_rules = rules.to_string();
    if let Some(entry) = saved_entry {
        save_config(move |config| {
            config.saved_profiles = next;
            config.active_profile = entry.name;
            config.filter = entry.filter;
            config.bypass_hosts = entry.bypass_hosts;
            config.rules = entry.rules;
            config.upstream_proxy = entry.upstream_proxy;
            config.rules_enabled = entry.rules_enabled;
            config.breaks_enabled = entry.breaks_enabled;
            config.show_tunnel_failures = entry.show_tunnel_failures;
        })?;
    } else if changed {
        save_config(move |config| {
            config.rules = persisted_rules;
            config.saved_profiles = next;
        })?;
    } else {
        save_config(move |config| config.rules = persisted_rules)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_profile_history(
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    history: Signal<Vec<config::SavedProfile>>,
) {
    let next = push_profile_history(
        &history.get(),
        name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    if next == history.get() {
        return;
    }
    history.set(next.clone());
    let _ = save_config(|config| config.profile_history = next);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_profile_history(
    history: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
) -> Vec<config::SavedProfile> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return history.to_vec();
    }
    let resolved_name = resolve_profile_name(name, filter, rules);
    let candidate = config::SavedProfile {
        name: resolved_name,
        filter: filter.to_string(),
        bypass_hosts: bypass.to_string(),
        rules: rules.to_string(),
        upstream_proxy: upstream.to_string(),
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    };
    let mut next: Vec<config::SavedProfile> = history
        .iter()
        .filter(|item| !config::same_profile_content(item, &candidate))
        .cloned()
        .collect();
    next.insert(0, candidate);
    next.truncate(config::PROFILE_HISTORY_CAP);
    next
}

pub(crate) fn apply_saved_profile(
    profile: &config::SavedProfile,
    filter: Signal<String>,
    bypass: Signal<String>,
    rules: Signal<String>,
    upstream: Signal<String>,
    rules_on: Signal<bool>,
    breaks_on: Signal<bool>,
    tunnels: Signal<bool>,
    controls: &Controls,
) -> bool {
    filter.set(profile.filter.clone());
    bypass.set(profile.bypass_hosts.clone());
    rules.set(profile.rules.clone());
    upstream.set(profile.upstream_proxy.clone());
    rules_on.set(profile.rules_enabled);
    breaks_on.set(profile.breaks_enabled);
    tunnels.set(profile.show_tunnel_failures);
    controls.hub.set_bypass(&profile.bypass_hosts);
    controls.hub.set_rules(&profile.rules);
    controls.hub.set_upstream(&profile.upstream_proxy);
    controls.hub.set_rules_enabled(profile.rules_enabled);
    controls.hub.set_breaks_enabled(profile.breaks_enabled);
    controls
        .hub
        .set_show_tunnel_failures(profile.show_tunnel_failures);
    let profile = profile.clone();
    save_config_or_notify(controls, PrefName::Profiles, move |config| {
        config.active_profile = profile.name.clone();
        config.filter = profile.filter.clone();
        config.bypass_hosts = profile.bypass_hosts.clone();
        config.rules = profile.rules.clone();
        config.upstream_proxy = profile.upstream_proxy.clone();
        config.rules_enabled = profile.rules_enabled;
        config.breaks_enabled = profile.breaks_enabled;
        config.show_tunnel_failures = profile.show_tunnel_failures;
    })
}

pub(crate) fn drop_saved_profile(
    target: &config::SavedProfile,
    rule_name: Signal<String>,
    saved: Signal<Vec<config::SavedProfile>>,
    history: Signal<Vec<config::SavedProfile>>,
    controls: &Controls,
) {
    record_profile_history(
        &target.name,
        &target.filter,
        &target.bypass_hosts,
        &target.rules,
        &target.upstream_proxy,
        target.rules_enabled,
        target.breaks_enabled,
        target.show_tunnel_failures,
        history,
    );
    if rule_name.get().trim().eq_ignore_ascii_case(&target.name) {
        rule_name.set(String::new());
    }
    let next: Vec<config::SavedProfile> = saved
        .get()
        .into_iter()
        .filter(|item| item != target)
        .collect();
    saved.set(next.clone());
    let removed_name = target.name.clone();
    save_config_or_notify(controls, PrefName::Profiles, move |config| {
        if config
            .active_profile
            .trim()
            .eq_ignore_ascii_case(&removed_name)
        {
            config.active_profile.clear();
        }
        config.saved_profiles = next;
    });
}

/// Rules after 保存 commits a filled add-rule row. An empty row stays out,
/// so it cannot turn into a global `*`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn draft_rules(
    rules: &str,
    host: &str,
    path: &str,
    from: &str,
    to: &str,
    response: bool,
    kind: u8,
) -> String {
    draft_rules_with_op(rules, host, path, "", from, to, response, kind)
}

pub(crate) fn draft_rules_with_op(
    rules: &str,
    host: &str,
    path: &str,
    op: &str,
    from: &str,
    to: &str,
    response: bool,
    kind: u8,
) -> String {
    let (from, to) = effective_quick_inputs(kind, response, from, to);
    let op = effective_quick_op(kind, op);
    let touched = !host.trim().is_empty()
        || !path.trim().is_empty()
        || !op.trim().is_empty()
        || !from.trim().is_empty()
        || !to.trim().is_empty();
    if !touched {
        return rules.to_string();
    }
    let combined = crate::rules::combine_rule_pattern(host, path);
    if op.trim().is_empty() {
        crate::rules::append_quick_rule(rules, quick_kind(kind), &combined, response, from, to)
            .unwrap_or_else(|| rules.to_string())
    } else {
        crate::rules::append_quick_rule_with_op(
            rules,
            quick_kind(kind),
            &combined,
            response,
            from,
            to,
            op,
        )
        .unwrap_or_else(|| rules.to_string())
    }
}

pub(crate) fn resolve_profile_name(name: &str, filter: &str, rules: &str) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    if !filter.is_empty() {
        return filter.chars().take(28).collect();
    }
    let lang = use_context::<Signal<Option<Lang>>>()
        .map(ui_lang)
        .unwrap_or_else(|| load_lang_pref().unwrap_or_else(|| lang_from_tag(locale().as_str())));
    let fallback = t(lang, "配置", "Profile");
    if !rules.is_empty() {
        return rules
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or(fallback)
            .chars()
            .take(28)
            .collect();
    }
    fallback.to_string()
}

/// True when 保存 would store a different snapshot. The open setup is the
/// saved profile with the same name; an unchanged one stays disabled.
pub(crate) fn profile_save_ready(
    saved: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
    draft_host: &str,
    draft_path: &str,
    draft_op: &str,
    draft_from: &str,
    draft_to: &str,
    draft_response: bool,
    draft_kind: u8,
) -> bool {
    let rules = draft_rules_with_op(
        rules,
        draft_host,
        draft_path,
        draft_op,
        draft_from,
        draft_to,
        draft_response,
        draft_kind,
    );
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return false;
    }
    let resolved = resolve_profile_name(name, filter, rules);
    let Some(preset) = saved
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(&resolved))
    else {
        return true;
    };
    preset.name != resolved
        || preset.filter.trim() != filter
        || preset.bypass_hosts.trim() != bypass
        || preset.rules.trim() != rules
        || preset.upstream_proxy.trim() != upstream
        || preset.rules_enabled != rules_enabled
        || preset.breaks_enabled != breaks_enabled
        || preset.show_tunnel_failures != show_tunnel_failures
}

pub(crate) fn push_saved_profile(
    saved: &[config::SavedProfile],
    name: &str,
    filter: &str,
    bypass: &str,
    rules: &str,
    upstream: &str,
    rules_enabled: bool,
    breaks_enabled: bool,
    show_tunnel_failures: bool,
) -> Vec<config::SavedProfile> {
    let filter = filter.trim();
    let bypass = bypass.trim();
    let rules = rules.trim();
    let upstream = upstream.trim();
    if filter.is_empty() && bypass.is_empty() && rules.is_empty() && upstream.is_empty() {
        return saved.to_vec();
    }
    let resolved_name = resolve_profile_name(name, filter, rules);
    let mut next: Vec<config::SavedProfile> = saved
        .iter()
        .filter(|item| !item.name.eq_ignore_ascii_case(&resolved_name))
        .cloned()
        .collect();
    next.insert(
        0,
        config::SavedProfile {
            name: resolved_name,
            filter: filter.to_string(),
            bypass_hosts: bypass.to_string(),
            rules: rules.to_string(),
            upstream_proxy: upstream.to_string(),
            rules_enabled,
            breaks_enabled,
            show_tunnel_failures,
        },
    );
    next.truncate(SAVED_RULE_CAP);
    next
}

pub(crate) fn clamp_list_px(width: f32) -> f32 {
    let rounded = if width.is_finite() && width > 0.0 {
        width.round().min(u32::MAX as f32) as u32
    } else {
        0
    };
    config::clamp_list_width(rounded) as f32
}

pub(crate) fn clamp_list_px_for_window(width: f32, window_w: f32) -> f32 {
    let base = clamp_list_px(width);
    if window_w.is_finite() && window_w > 0.0 {
        let max_for_win = (window_w - 340.0)
            .round()
            .clamp(config::LIST_WIDTH_MIN as f32, config::LIST_WIDTH_MAX as f32);
        base.min(max_for_win)
    } else {
        base
    }
}

pub(crate) fn adaptive_filter_panel_width(window_w: f32, list_w: f32) -> f32 {
    if window_w.is_finite() && window_w > 0.0 {
        let list_px = clamp_list_px_for_window(list_w, window_w);
        let max_w = (window_w - 20.0).max(280.0);
        (window_w - list_px * 0.7 - 16.0)
            .round()
            .clamp(680.0, 1360.0)
            .min(max_w)
    } else {
        760.0
    }
}

pub(crate) const FILTER_PANEL_CHROME_CLEARANCE: f32 = 80.0;
pub(crate) const FILTER_PANEL_DOCK_HEIGHT: f32 = 54.0;
pub(crate) const FILTER_PANEL_IMPORT_EXTRA_HEIGHT: f32 = 182.0;
pub(crate) const FILTER_PANEL_RIGHT_FIXED_HEIGHT: f32 = 620.0;

pub(crate) fn adaptive_filter_panel_max_height(window_h: f32) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        (window_h - FILTER_PANEL_CHROME_CLEARANCE)
            .round()
            .max(260.0)
    } else {
        672.0
    }
}

pub(crate) fn adaptive_filter_body_max_height(window_h: f32, import_open: bool) -> f32 {
    let import_h = if import_open {
        FILTER_PANEL_IMPORT_EXTRA_HEIGHT
    } else {
        0.0
    };
    (adaptive_filter_panel_max_height(window_h) - FILTER_PANEL_DOCK_HEIGHT - import_h)
        .round()
        .max(if import_open { 24.0 } else { 120.0 })
}

#[cfg(test)]
pub(crate) fn adaptive_rules_editor_height(window_h: f32) -> f32 {
    adaptive_rules_editor_height_for(window_h, false)
}

pub(crate) fn adaptive_rules_editor_height_for(window_h: f32, import_open: bool) -> f32 {
    if window_h.is_finite() && window_h > 0.0 {
        let body_budget = adaptive_filter_body_max_height(window_h, import_open);
        let min_h = if import_open { 56.0 } else { 84.0 };
        let max_h = if import_open { 320.0 } else { 460.0 };
        (body_budget - FILTER_PANEL_RIGHT_FIXED_HEIGHT)
            .round()
            .clamp(min_h, max_h)
    } else {
        105.0
    }
}
