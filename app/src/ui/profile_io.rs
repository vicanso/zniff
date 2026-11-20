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

/// Previews Whistle or Charles XML rule conversion counts while pasting in the Import box.
pub(crate) fn preview_rule_import(raw_text: &str, lang: Lang) -> Option<String> {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if import_charles::looks_like_charles_xml(trimmed) {
        let rep = import_charles::convert_charles_xml(trimmed, "Charles XML");
        return Some(match lang {
            Lang::Zh => format!(
                "检测到 Charles XML：预计转换 {} 条规则，保留注释/未映射 {} 条",
                rep.converted,
                rep.skipped_comments + rep.unmapped
            ),
            Lang::En => format!(
                "Charles XML detected: {} rules to convert, {} commented/unmapped",
                rep.converted,
                rep.skipped_comments + rep.unmapped
            ),
        });
    }
    if import_whistle::looks_like_whistle(trimmed) {
        let rep = import_whistle::convert_whistle_rules(trimmed, "Whistle");
        return Some(match lang {
            Lang::Zh => format!(
                "检测到 Whistle 规则：预计转换 {} 条，未映射 {} 条将以 # 注释保留",
                rep.converted, rep.unmapped
            ),
            Lang::En => format!(
                "Whistle rules detected: {} to convert, {} unmapped kept as # comments",
                rep.converted, rep.unmapped
            ),
        });
    }
    None
}

pub(crate) fn apply_rule_import_report(
    controls: &Controls,
    report: import_whistle::RuleImportReport,
    profiles: ProfileSignals,
) -> bool {
    if report.converted == 0
        && report.unmapped == 0
        && report.skipped_comments == 0
        && report.bypass_hosts.is_empty()
    {
        set_status(
            controls,
            StatusMsg::ImportFailed("no convertible rules found".to_string()),
        );
        return false;
    }
    let mut saved_ok = true;
    if !report.rules_text.trim().is_empty() {
        let trimmed_source = report.source_name.trim();
        let group_name = if trimmed_source.is_empty() {
            "Imported".to_string()
        } else {
            trimmed_source.to_string()
        };
        let next_groups = config::upsert_rule_group(
            &profiles.rule_groups.get(),
            &group_name,
            &report.rules_text,
            true,
        );
        let selected_name = next_groups
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&group_name))
            .map(|item| item.name.clone())
            .unwrap_or(group_name);
        profiles.rule_groups.set(next_groups.clone());
        profiles.editing_group.set(Some(selected_name));
        controls.hub.set_rule_groups(&next_groups);
        saved_ok &= save_config_or_notify(controls, PrefName::Rules, |config| {
            config.rule_groups = next_groups;
        });
    }
    if !report.bypass_hosts.is_empty() {
        let next_bypass = merge_bypass_hosts(&profiles.bypass.get(), &report.bypass_hosts);
        profiles.bypass.set(next_bypass.clone());
        controls.hub.set_bypass(&next_bypass);
        saved_ok &= save_config_or_notify(controls, PrefName::Bypass, |config| {
            config.bypass_hosts = next_bypass;
        });
    }
    if saved_ok {
        set_status(
            controls,
            StatusMsg::RulesMigrated {
                source: report.source_name,
                converted: report.converted,
                unmapped: report.unmapped,
            },
        );
    }
    true
}

fn merge_bypass_hosts(existing: &str, hosts: &[String]) -> String {
    let mut items: Vec<String> = existing
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for host in hosts {
        if !items.iter().any(|item| item.eq_ignore_ascii_case(host)) {
            items.push(host.clone());
        }
    }
    items.join(", ")
}

pub(crate) fn import_profiles_text(
    controls: &Controls,
    raw_text: &str,
    profiles: ProfileSignals,
) -> bool {
    let trimmed = raw_text.trim();
    if import_charles::looks_like_charles_xml(trimmed) {
        let report = import_charles::convert_charles_xml(trimmed, "Charles XML");
        return apply_rule_import_report(controls, report, profiles);
    }
    if let Ok(incoming) = parse_profiles_from_text_or_path(raw_text) {
        apply_imported_profiles(controls, incoming, profiles);
        return true;
    }
    if import_whistle::looks_like_whistle(trimmed) {
        let report = import_whistle::convert_whistle_rules(trimmed, "Whistle");
        return apply_rule_import_report(controls, report, profiles);
    }
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
        } else if let Some(path) = dropped_ext_path(uri, "chlsj") {
            found = true;
            import_chlsj_file(controls, &path);
        } else if let Some(path) = dropped_toml_path(uri) {
            found = true;
            import_profiles_file(controls, &path, profiles);
        } else if let Some(path) = dropped_ext_path(uri, "xml") {
            found = true;
            import_charles_xml_file(controls, &path, profiles);
        } else if let Some(path) = dropped_ext_path(uri, "txt") {
            found = true;
            import_whistle_file(controls, &path, profiles);
        }
    }
    if !found {
        set_status(
            controls,
            StatusMsg::ImportFailed(
                t(
                    lang,
                    "仅支持拖入 .har、.chlsj、.toml、Charles .xml 或 Whistle .txt 文件",
                    "Drop a .har, .chlsj, .toml, Charles .xml, or Whistle .txt file",
                )
                .to_string(),
            ),
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prompt_import_migration(controls: Arc<Controls>, profiles: ProfileSignals) {
    spawn_detached(async move {
        let dialog =
            open_files_dialog()
                .multiple(true)
                .filter(nsh_core::file_dialog::FileFilter::new(
                    "HAR / Charles / Whistle / TOML",
                    ["har", "chlsj", "toml", "xml", "txt"],
                ));
        match pick_paths_after_paint(dialog).await {
            DialogOutcome::Picked(paths) => {
                for path in paths {
                    let path_str = path.to_string_lossy();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .unwrap_or_default();
                    match ext.as_str() {
                        "har" => import_har_file(&controls, &path_str),
                        "chlsj" => import_chlsj_file(&controls, &path_str),
                        "toml" => import_profiles_file(&controls, &path_str, profiles),
                        "xml" => import_charles_xml_file(&controls, &path_str, profiles),
                        "txt" => import_whistle_file(&controls, &path_str, profiles),
                        _ => {}
                    }
                }
            }
            DialogOutcome::Cancelled | DialogOutcome::Busy => {}
            DialogOutcome::Failed(err) => set_status(&controls, StatusMsg::ImportFailed(err)),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_charles_xml_file(
    controls: &Controls,
    path_text: &str,
    profiles: ProfileSignals,
) {
    let path = std::path::Path::new(path_text.trim());
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Charles XML");
    let report = import_charles::convert_charles_xml(&content, label);
    let _ = apply_rule_import_report(controls, report, profiles);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_whistle_file(controls: &Controls, path_text: &str, profiles: ProfileSignals) {
    let path = std::path::Path::new(path_text.trim());
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            set_status(controls, StatusMsg::ImportFailed(err.to_string()));
            return;
        }
    };
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Whistle");
    let report = import_whistle::convert_whistle_rules(&content, label);
    let _ = apply_rule_import_report(controls, report, profiles);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import_chlsj_file(controls: &Controls, path_text: &str) {
    let path = std::path::PathBuf::from(path_text.trim());
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
    let items = import_charles::import_charles_chlsj(&value);
    let count = controls.hub.import_flows(items);
    set_status(
        controls,
        StatusMsg::Imported {
            count,
            path: path.display().to_string(),
        },
    );
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
