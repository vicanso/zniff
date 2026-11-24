#[allow(unused_imports)]
use super::*;

pub(crate) fn filter_chip(
    filter: Signal<String>,
    zh: &'static str,
    en: &'static str,
    token: &'static str,
    aliases: &'static [&'static str],
) -> impl IntoElement {
    let is_active = computed(move || filter_has_token(&filter.get(), aliases));
    view()
        .role(AccessibilityRole::Button)
        .h(px(ControlSize::Sm.height()))
        .px_2p5()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(move || {
            if is_active.get() {
                accent_blue_border()
            } else {
                border_control()
            }
        })
        .bg(move || {
            if is_active.get() {
                accent_blue_soft()
            } else {
                bg_control()
            }
        })
        .hover(|style| {
            style.bg(move || {
                if is_active.get() {
                    accent_blue_soft_hover()
                } else {
                    bg_control_hover()
                }
            })
        })
        .active(|style| style.bg(bg_control_active()))
        .transition_colors()
        .duration(MOTION_INSTANT_MS)
        .ease_out()
        .focus_visible(apply_focus_ring)
        .on_press(move || filter.set(toggle_filter_chip(&filter.get(), token, aliases)))
        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(move || {
            if is_active.get() {
                accent_blue()
            } else {
                fg_muted()
            }
        }))
        .child(
            text(tr(zh, en))
                .number_of_lines(1)
                .text_size(px(TEXT_SM))
                .text_color(move || {
                    if is_active.get() {
                        accent_blue_text()
                    } else {
                        fg_secondary()
                    }
                }),
        )
}

pub(crate) fn filter_has_token(filter: &str, aliases: &[&str]) -> bool {
    let normalized = crate::session::normalize_filter_pipes(filter);
    normalized.split_whitespace().any(|part| {
        let lower = part.to_ascii_lowercase();
        aliases.iter().any(|alias| lower == *alias)
    })
}

/// Adds `token` as its own word, or removes every alias of that chip.
pub(crate) fn toggle_filter_chip(filter: &str, token: &str, aliases: &[&str]) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut kept = Vec::new();
    let mut found = false;
    for part in normalized.split_whitespace() {
        let lower = part.to_ascii_lowercase();
        if aliases.iter().any(|alias| lower == *alias) {
            found = true;
            continue;
        }
        kept.push(part.to_string());
    }
    if !found {
        kept.push(token.to_string());
    }
    kept.join(" ")
}

pub(crate) fn split_ui_or_items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for item in text
        .split(|ch: char| ch == '|' || ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(item))
        {
            out.push(item.to_string());
        }
    }
    out
}

pub(crate) fn extract_host_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("host:")
            .or_else(|| lower.strip_prefix("domain:"))
        {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

pub(crate) fn set_host_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("host:") || lower.starts_with("domain:") {
            if !inserted && !items.is_empty() {
                out.push(format!("host:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.insert(0, format!("host:{}", items.join("|")));
    }
    out.join(" ")
}

pub(crate) fn extract_path_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("path:") {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

pub(crate) fn set_path_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("path:") {
            if !inserted && !items.is_empty() {
                out.push(format!("path:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("path:{}", items.join("|")));
    }
    out.join(" ")
}

pub(crate) fn extract_exclude_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let rest = lower.strip_prefix("mute:").or_else(|| {
            lower
                .strip_prefix('-')
                .filter(|rest| !rest.is_empty() && !rest.contains(':'))
        });
        if let Some(rest) = rest {
            for part in split_ui_or_items(rest) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

pub(crate) fn set_exclude_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items: Vec<String> = split_ui_or_items(input)
        .into_iter()
        .map(|item| {
            item.strip_prefix("mute:")
                .or_else(|| item.strip_prefix('-'))
                .unwrap_or(&item)
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let is_exclude = lower.starts_with("mute:")
            || lower
                .strip_prefix('-')
                .is_some_and(|rest| !rest.is_empty() && !rest.contains(':'));
        if is_exclude {
            if !inserted && !items.is_empty() {
                out.push(format!("mute:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("mute:{}", items.join("|")));
    }
    out.join(" ")
}

pub(crate) fn extract_app_or(filter: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let mut items = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let rest_len = if lower.starts_with("app:") {
            Some(4)
        } else if lower.starts_with("proc:") {
            Some(5)
        } else if lower.starts_with("ip:") {
            Some(3)
        } else if lower.starts_with("client:") {
            Some(7)
        } else {
            None
        };
        if let Some(prefix_len) = rest_len {
            for part in split_ui_or_items(&token[prefix_len..]) {
                if !items
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&part))
                {
                    items.push(part);
                }
            }
        }
    }
    items.join(" | ")
}

pub(crate) fn set_app_or(filter: &str, input: &str) -> String {
    let normalized = crate::session::normalize_filter_pipes(filter);
    let items = split_ui_or_items(input);
    let mut out = Vec::new();
    let mut inserted = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("app:")
            || lower.starts_with("proc:")
            || lower.starts_with("ip:")
            || lower.starts_with("client:")
        {
            if !inserted && !items.is_empty() {
                out.push(format!("app:{}", items.join("|")));
                inserted = true;
            }
            continue;
        }
        out.push(token.to_string());
    }
    if !inserted && !items.is_empty() {
        out.push(format!("app:{}", items.join("|")));
    }
    out.join(" ")
}

pub(crate) fn host_facet_active(filter: &str, host: &str) -> bool {
    split_ui_or_items(&extract_host_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(host))
}

pub(crate) fn toggle_host_facet(filter: &str, host: &str) -> String {
    let mut items = split_ui_or_items(&extract_host_or(filter));
    let added = if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(host))
    {
        items.remove(pos);
        false
    } else {
        items.push(host.to_string());
        true
    };
    let next = set_host_or(filter, &items.join(" | "));
    if !added {
        return next;
    }
    let mut muted = split_ui_or_items(&extract_exclude_or(&next));
    let before = muted.len();
    muted.retain(|item| !item.eq_ignore_ascii_case(host));
    if muted.len() == before {
        next
    } else {
        set_exclude_or(&next, &muted.join(" | "))
    }
}

pub(crate) fn root_domain_facet_active(filter: &str, root: &str) -> bool {
    let wild = format!("*.{root}");
    let items = split_ui_or_items(&extract_host_or(filter));
    items.iter().any(|item| item.eq_ignore_ascii_case(&wild))
        || items.iter().any(|item| item.eq_ignore_ascii_case(root))
}

pub(crate) fn toggle_root_domain_facet(filter: &str, root: &str) -> String {
    let wild = format!("*.{root}");
    let mut items = split_ui_or_items(&extract_host_or(filter));
    let had_wild = items.iter().any(|item| item.eq_ignore_ascii_case(&wild));
    let had_bare = items.iter().any(|item| item.eq_ignore_ascii_case(root));
    if had_wild || had_bare {
        items.retain(|item| !item.eq_ignore_ascii_case(&wild) && !item.eq_ignore_ascii_case(root));
        set_host_or(filter, &items.join(" | "))
    } else {
        items.push(wild.clone());
        items.push(root.to_string());
        let next = set_host_or(filter, &items.join(" | "));
        let mut muted = split_ui_or_items(&extract_exclude_or(&next));
        let before = muted.len();
        muted.retain(|item| !item.eq_ignore_ascii_case(root) && !item.eq_ignore_ascii_case(&wild));
        if muted.len() == before {
            next
        } else {
            set_exclude_or(&next, &muted.join(" | "))
        }
    }
}

pub(crate) fn app_facet_active(filter: &str, app_token: &str) -> bool {
    split_ui_or_items(&extract_app_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(app_token))
}

pub(crate) fn toggle_app_facet(filter: &str, app_token: &str) -> String {
    let mut items = split_ui_or_items(&extract_app_or(filter));
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(app_token))
    {
        items.remove(pos);
    } else {
        items.push(app_token.to_string());
    }
    set_app_or(filter, &items.join(" | "))
}

pub(crate) fn mute_facet_active(filter: &str, token: &str) -> bool {
    split_ui_or_items(&extract_exclude_or(filter))
        .iter()
        .any(|item| item.eq_ignore_ascii_case(token))
}

pub(crate) fn toggle_mute_facet(filter: &str, token: &str) -> String {
    let mut items = split_ui_or_items(&extract_exclude_or(filter));
    let added = if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(token))
    {
        items.remove(pos);
        false
    } else {
        items.push(token.to_string());
        true
    };
    let next = set_exclude_or(filter, &items.join(" | "));
    if !added {
        return next;
    }
    let wild = format!("*.{token}");
    let mut hosts = split_ui_or_items(&extract_host_or(&next));
    let before = hosts.len();
    hosts.retain(|item| !item.eq_ignore_ascii_case(token) && !item.eq_ignore_ascii_case(&wild));
    if hosts.len() == before {
        next
    } else {
        set_host_or(&next, &hosts.join(" | "))
    }
}

pub(crate) fn side_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    focused_input(value, placeholder, height, multiline, on_change, None)
}

pub(crate) fn submit_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    on_change: impl Fn(&str) + 'static,
    on_submit: impl Fn(&str) + 'static,
) -> impl IntoElement {
    styled_input_box(value, placeholder, height, false, on_change).on_submit(on_submit)
}

pub(crate) fn focused_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
    focus: Option<ElementRef>,
) -> impl IntoElement {
    let field = styled_input_box(value, placeholder, height, multiline, on_change);
    match focus {
        Some(focus) => field.element_ref(&focus),
        None => field,
    }
}

fn styled_input_box(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
) -> nsh_base::Input {
    let field = input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(move || color(fg_muted()))
        .w_full()
        .h(px(height))
        .px_2p5()
        .rounded(px(RADIUS_MD))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(TEXT_SM))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change);
    if multiline { field.py_2() } else { field }
}
