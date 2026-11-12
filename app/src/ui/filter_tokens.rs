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
        .h(px(28.0))
        .px_2p5()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(px(6.0))
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
        .active(|style| style.opacity(0.8))
        .cursor(Cursor::Pointer)
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
                .text_size(px(11.5))
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
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(host))
    {
        items.remove(pos);
    } else {
        items.push(host.to_string());
    }
    set_host_or(filter, &items.join(" | "))
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
    if let Some(pos) = items
        .iter()
        .position(|item| item.eq_ignore_ascii_case(token))
    {
        items.remove(pos);
    } else {
        items.push(token.to_string());
    }
    set_exclude_or(filter, &items.join(" | "))
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

pub(crate) fn focused_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    multiline: bool,
    on_change: impl Fn(&str) + 'static,
    focus: Option<ElementRef>,
) -> impl IntoElement {
    let field = input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(move || color(fg_muted()))
        .w_full()
        .h(px(height))
        .px_2p5()
        .rounded(px(7.0))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|style| style.border_color(accent_blue()))
        .text_size(px(12.5))
        .text_color(fg_primary())
        .multiline(multiline)
        .on_change(on_change);
    let field = if multiline { field.py_2() } else { field };
    match focus {
        Some(focus) => field.element_ref(&focus),
        None => field,
    }
}
