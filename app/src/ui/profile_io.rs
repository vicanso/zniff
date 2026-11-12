#[allow(unused_imports)]
use super::*;

pub(crate) fn profiles_for_export(
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
    let effective_name = if !name.trim().is_empty() {
        name.trim().to_string()
    } else {
        matched_saved_profile(saved, filter, bypass, rules, upstream).unwrap_or_default()
    };
    let current = push_saved_profile(
        &[],
        &effective_name,
        filter,
        bypass,
        rules,
        upstream,
        rules_enabled,
        breaks_enabled,
        show_tunnel_failures,
    );
    if !current.is_empty() {
        return current;
    }
    if !effective_name.is_empty()
        && let Some(matched) = saved
            .iter()
            .find(|preset| preset.name.eq_ignore_ascii_case(&effective_name))
    {
        return vec![matched.clone()];
    }
    saved.to_vec()
}

#[cfg(test)]
pub(crate) fn profile_export_filename(
    preferred_name: &str,
    profiles: &[config::SavedProfile],
    secs: u64,
) -> String {
    let raw = if !preferred_name.trim().is_empty() {
        preferred_name.trim()
    } else {
        profiles
            .first()
            .map(|preset| preset.name.trim())
            .unwrap_or_default()
    };
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '-'
            } else {
                ch
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches(|ch: char| ch.is_whitespace() || ch == '.' || ch == '-');
    if sanitized.is_empty() {
        format!("zniff-profiles-{secs}.toml")
    } else {
        format!("zniff-{sanitized}.toml")
    }
}

pub(crate) fn export_profiles_clipboard(controls: &Controls, profiles: &[config::SavedProfile]) {
    match config::export_profiles_toml(profiles) {
        Ok(toml_text) => {
            let count = profiles.len();
            set_clipboard(toml_text);
            set_status(controls, StatusMsg::ProfilesExported { count });
        }
        Err(err) => {
            set_status(controls, StatusMsg::ExportFailed(err));
        }
    }
}

pub(crate) fn parse_profiles_from_text_or_path(
    input: &str,
) -> std::result::Result<Vec<config::SavedProfile>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty TOML".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let candidate_str = dropped_toml_path(trimmed).unwrap_or_else(|| trimmed.to_string());
        let candidate = std::path::Path::new(&candidate_str);
        if candidate
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            && candidate.is_file()
        {
            let content = std::fs::read_to_string(candidate).map_err(|err| err.to_string())?;
            return config::parse_profiles_toml(&content);
        }
    }
    config::parse_profiles_toml(trimmed)
}

pub(crate) fn import_profiles_text(
    controls: &Controls,
    raw_text: &str,
    profiles: ProfileSignals,
) -> bool {
    match parse_profiles_from_text_or_path(raw_text) {
        Ok(incoming) => {
            apply_imported_profiles(controls, incoming, profiles);
            true
        }
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err));
            false
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_profiles_file(controls: &Controls, path_text: &str, profiles: ProfileSignals) {
    let content = match std::fs::read_to_string(path_text.trim()) {
        Ok(content) => content,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let incoming = match config::parse_profiles_toml(&content) {
        Ok(incoming) => incoming,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err));
            return;
        }
    };
    apply_imported_profiles(controls, incoming, profiles);
}

pub(crate) fn apply_imported_profiles(
    controls: &Controls,
    incoming: Vec<config::SavedProfile>,
    profiles: ProfileSignals,
) {
    let count = incoming.len();
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
    for item in incoming.iter().rev() {
        record_profile_history(
            &item.name,
            &item.filter,
            &item.bypass_hosts,
            &item.rules,
            &item.upstream_proxy,
            item.rules_enabled,
            item.breaks_enabled,
            item.show_tunnel_failures,
            profiles.history,
        );
    }
    let merged = config::merge_imported_profiles(&profiles.saved.get(), &incoming, SAVED_RULE_CAP);
    profiles.saved.set(merged.clone());
    let mut saved_ok = save_config_or_notify(controls, PrefName::Profiles, |config| {
        config.saved_profiles = merged
    });
    if let Some(first) = incoming.first() {
        profiles.name.set(first.name.clone());
        saved_ok &= apply_saved_profile(
            first,
            profiles.filter,
            profiles.bypass,
            profiles.rules,
            profiles.upstream,
            profiles.rules_on,
            profiles.breaks_on,
            profiles.show_tunnels,
            controls,
        );
    }
    if saved_ok {
        set_status(controls, StatusMsg::ProfilesImported { count });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_dropped(
    controls: &Controls,
    event: &FileDropEvent,
    lang: Lang,
    profiles: ProfileSignals,
) {
    let mut found = false;
    for uri in &event.uris {
        if let Some(path) = dropped_har_path(uri) {
            found = true;
            import_har_file(controls, &path);
        } else if let Some(path) = dropped_toml_path(uri) {
            found = true;
            import_profiles_file(controls, &path, profiles);
        }
    }
    if !found {
        set_status(
            controls,
            StatusMsg::ImportFailed(
                t(
                    lang,
                    "放下的文件不是 HAR 或 TOML 配置",
                    "Dropped file is not a HAR or TOML setup",
                )
                .to_string(),
            ),
        );
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn import_dropped(
    controls: &Controls,
    _event: &FileDropEvent,
    lang: Lang,
    _profiles: ProfileSignals,
) {
    set_status(
        controls,
        StatusMsg::ImportFailed(
            t(
                lang,
                "Web 模式请打开「过滤 → 导入」粘贴 TOML 配置",
                "In Web mode, open Filter → Import to paste TOML setup",
            )
            .to_string(),
        ),
    );
}

/// `file:` URL of a `.har`, or nothing. Encoded names are decoded. A Windows
/// `file:///C:/...` drops the extra slash. Other schemes are ignored.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn dropped_har_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "har")
}

/// `file:` URL of a `.toml` profile file, or nothing.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn dropped_toml_path(uri: &str) -> Option<String> {
    dropped_ext_path(uri, "toml")
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn dropped_ext_path(uri: &str, ext_name: &str) -> Option<String> {
    let rest = uri.strip_prefix("file:")?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let rest = rest
        .strip_prefix("localhost")
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let path = file_url_path(rest);
    let matches = std::path::Path::new(&path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(ext_name));
    matches.then_some(path)
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn file_url_path(path: &str) -> String {
    let decoded = percent_decode_path(path);
    let bytes = decoded.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes[3] == b'/' || bytes[3] == b'\\')
    {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_har_file(controls: &Controls, path_text: &str) {
    let path = if path_text.trim().is_empty() {
        ca::data_dir().join("import.har")
    } else {
        std::path::PathBuf::from(path_text.trim())
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let count = controls.hub.import_har(&value);
    set_status(
        controls,
        StatusMsg::Imported {
            count,
            path: path.display().to_string(),
        },
    );
}
