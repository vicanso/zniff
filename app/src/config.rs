//! User settings in one `config.toml`. The system-proxy snapshot stays in its
//! own file: a failed write while restoring the proxy must not take the port
//! and rules with it.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::{Lang, parse_pref};
use crate::present::DEFAULT_PORT;

const FILE_NAME: &str = "config.toml";
const DEV_FILE_NAME: &str = "config.dev.toml";
pub const DEV_DEFAULT_PORT: u16 = 8878;

/// True in debug builds (outside `cargo test`) or when `ZNIFF_DEV` is enabled.
/// Dev mode isolates `config.dev.toml` and `proxy-snapshot.dev.toml` while
/// sharing `ca.crt` and `ca.key`.
pub fn is_dev_mode() -> bool {
    cfg!(all(debug_assertions, not(test))) || is_dev_env(std::env::var("ZNIFF_DEV").ok().as_deref())
}

pub fn is_dev_env(val: Option<&str>) -> bool {
    val.is_some_and(|text| {
        let trimmed = text.trim();
        trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
    })
}

pub fn config_file_name(dev: bool) -> &'static str {
    if dev { DEV_FILE_NAME } else { FILE_NAME }
}

pub fn default_listen_port(dev: bool) -> u16 {
    if dev { DEV_DEFAULT_PORT } else { DEFAULT_PORT }
}

/// Left pane width when `config.toml` has no `list_width`.
pub const LIST_WIDTH_DEFAULT: u32 = 560;
pub const LIST_WIDTH_MIN: u32 = 280;
pub const LIST_WIDTH_MAX: u32 = 1100;

/// Window size when `config.toml` has no `window_width` / `window_height`.
pub const WINDOW_WIDTH_DEFAULT: u32 = 1200;
pub const WINDOW_WIDTH_MIN: u32 = 720;
pub const WINDOW_WIDTH_MAX: u32 = 7680;
pub const WINDOW_HEIGHT_DEFAULT: u32 = 760;
pub const WINDOW_HEIGHT_MIN: u32 = 480;
pub const WINDOW_HEIGHT_MAX: u32 = 4320;

pub fn clamp_list_width(width: u32) -> u32 {
    width.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX)
}

pub fn clamp_window_width(width: u32) -> u32 {
    width.clamp(WINDOW_WIDTH_MIN, WINDOW_WIDTH_MAX)
}

pub fn clamp_window_height(height: u32) -> u32 {
    height.clamp(WINDOW_HEIGHT_MIN, WINDOW_HEIGHT_MAX)
}

const LEGACY: &[&str] = &[
    "listen-port",
    "listen-all",
    "use-system-proxy",
    "language",
    "bypass-hosts",
    "rules",
    "upstream-proxy",
];

/// How long paths/URLs and hosts are truncated in the flow list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListEllipsize {
    Head,
    #[default]
    Middle,
    Tail,
}

impl ListEllipsize {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Middle => "middle",
            Self::Tail => "tail",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "head" | "start" => Self::Head,
            "tail" | "end" => Self::Tail,
            _ => Self::Middle,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::Middle
    }
}

impl Serialize for ListEllipsize {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ListEllipsize {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self::parse(&text))
    }
}

/// One named set of proxy rules saved by the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRuleSet {
    pub name: String,
    pub rules: String,
}

/// One named snapshot of the filter panel and the rules panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedProfile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bypass_hosts: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rules: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub upstream_proxy: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub rules_enabled: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub breaks_enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_tunnel_failures: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileExport {
    #[serde(default, alias = "saved_profiles")]
    pub profiles: Vec<SavedProfile>,
}

fn sanitize_profile(mut profile: SavedProfile) -> Option<SavedProfile> {
    profile.name = profile.name.trim().to_string();
    profile.filter = profile.filter.trim().to_string();
    profile.bypass_hosts = profile.bypass_hosts.trim().to_string();
    profile.rules = profile.rules.trim().to_string();
    profile.upstream_proxy = profile.upstream_proxy.trim().to_string();
    if profile.name.is_empty() {
        return None;
    }
    if profile.filter.is_empty()
        && profile.bypass_hosts.is_empty()
        && profile.rules.is_empty()
        && profile.upstream_proxy.is_empty()
    {
        return None;
    }
    Some(profile)
}

pub fn export_profiles_toml(profiles: &[SavedProfile]) -> std::result::Result<String, String> {
    let cleaned: Vec<SavedProfile> = profiles
        .iter()
        .cloned()
        .filter_map(sanitize_profile)
        .collect();
    if cleaned.is_empty() {
        return Err("no saved setups to export".to_string());
    }
    toml::to_string_pretty(&ProfileExport { profiles: cleaned }).map_err(|err| err.to_string())
}

pub fn parse_profiles_toml(text: &str) -> std::result::Result<Vec<SavedProfile>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty TOML".to_string());
    }
    if let Ok(bundle) = toml::from_str::<ProfileExport>(trimmed) {
        let cleaned: Vec<SavedProfile> = bundle
            .profiles
            .into_iter()
            .filter_map(sanitize_profile)
            .collect();
        if !cleaned.is_empty() {
            return Ok(cleaned);
        }
    }
    let single = toml::from_str::<SavedProfile>(trimmed).map_err(|err| err.to_string())?;
    sanitize_profile(single)
        .map(|profile| vec![profile])
        .ok_or_else(|| "no valid setup found in TOML".to_string())
}

pub fn merge_imported_profiles(
    existing: &[SavedProfile],
    incoming: &[SavedProfile],
    cap: usize,
) -> Vec<SavedProfile> {
    let mut out: Vec<SavedProfile> = Vec::new();
    for item in incoming.iter().cloned().filter_map(sanitize_profile) {
        if !out
            .iter()
            .any(|seen| seen.name.eq_ignore_ascii_case(&item.name))
        {
            out.push(item);
        }
    }
    for item in existing.iter().cloned() {
        if !out
            .iter()
            .any(|seen| seen.name.eq_ignore_ascii_case(&item.name))
        {
            out.push(item);
        }
    }
    out.truncate(cap);
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub listen_port: u16,
    #[serde(default)]
    pub listen_all: bool,
    #[serde(default)]
    pub use_system_proxy: bool,
    /// `None` follows the system locale. Only `zh` and `en` are stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub bypass_hosts: String,
    #[serde(default)]
    pub rules: String,
    #[serde(default)]
    pub upstream_proxy: String,
    /// On unless the file says otherwise, so an older `config.toml` keeps today's list.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_id: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_method: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_status: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_protocol: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_path: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_gap: bool,
    /// On unless the file says otherwise. Time from the request to the full response.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_elapsed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_host: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_client_ip: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_size: bool,
    /// False shows only `flow.path` in the Path column; true shows `flow.url`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_full_url: bool,
    /// Where to ellipsize overflowing Path/URL and Host cells (`head`, `middle`, `tail`).
    #[serde(default, skip_serializing_if = "ListEllipsize::is_default")]
    pub ellipsize: ListEllipsize,
    #[serde(
        default = "default_list_width",
        skip_serializing_if = "is_default_list_width"
    )]
    pub list_width: u32,
    #[serde(
        default = "default_window_width",
        skip_serializing_if = "is_default_window_width"
    )]
    pub window_width: u32,
    #[serde(
        default = "default_window_height",
        skip_serializing_if = "is_default_window_height"
    )]
    pub window_height: u32,
    /// Pretty is the body default unless the file says otherwise.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub default_pretty: bool,
    /// Origin TLS probe failures stay out of the list unless this is on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_tunnel_failures: bool,
    /// Restored into the filter box on the next launch.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filter: String,
    /// The last window was capturing. The next launch presses Start itself.
    #[serde(default, skip_serializing_if = "is_false")]
    pub capturing: bool,
    /// Filters the user saved on purpose. Newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_filters: Vec<String>,
    /// Named rule presets the user saved on purpose. Newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_rules: Vec<SavedRuleSet>,
    /// Filter, bypass, rules, and upstream saved as one preset. Newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_profiles: Vec<SavedProfile>,
    /// Dragged list-column widths, in points, keyed by `method`, `path`, and the rest.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub column_widths: HashMap<String, u32>,
    /// Custom list-column order when reordered from [`DEFAULT_COLUMN_ORDER`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_order: Vec<String>,
    /// Sanitize sensitive headers, query params, and JSON/form fields on HAR/HTML export and live share.
    #[serde(default, skip_serializing_if = "is_false")]
    pub redact_export: bool,
    /// Extra comma-separated sensitive header or field names to redact on export/share.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redact_fields: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_port: default_port(),
            listen_all: false,
            use_system_proxy: false,
            language: None,
            bypass_hosts: String::new(),
            rules: String::new(),
            upstream_proxy: String::new(),
            show_id: true,
            show_method: true,
            show_status: true,
            show_protocol: false,
            show_path: true,
            show_gap: true,
            show_elapsed: true,
            show_host: false,
            show_client_ip: false,
            show_size: true,
            show_full_url: false,
            ellipsize: ListEllipsize::Middle,
            list_width: default_list_width(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            default_pretty: true,
            show_tunnel_failures: false,
            filter: String::new(),
            capturing: false,
            saved_filters: Vec::new(),
            saved_rules: Vec::new(),
            saved_profiles: Vec::new(),
            column_widths: HashMap::new(),
            column_order: Vec::new(),
            redact_export: false,
            redact_fields: String::new(),
        }
    }
}

impl Config {
    pub fn default_for(dev: bool) -> Self {
        Self {
            listen_port: default_listen_port(dev),
            ..Self::default()
        }
    }
}

/// Default, narrowest, and widest width of one list column, in points.
pub struct ColumnWidth {
    pub default: u32,
    pub min: u32,
    pub max: u32,
}

/// `None` when `key` is not a list column.
pub fn column_width_limit(key: &str) -> Option<ColumnWidth> {
    Some(match key {
        "method" => ColumnWidth {
            default: 76,
            min: 56,
            max: 220,
        },
        "id" => ColumnWidth {
            default: 38,
            min: 34,
            max: 120,
        },
        "path" => ColumnWidth {
            default: 220,
            min: 80,
            max: 800,
        },
        "status" => ColumnWidth {
            default: 46,
            min: 40,
            max: 160,
        },
        "protocol" => ColumnWidth {
            default: 78,
            min: 48,
            max: 200,
        },
        "gap" => ColumnWidth {
            default: 60,
            min: 48,
            max: 200,
        },
        "elapsed" => ColumnWidth {
            default: 54,
            min: 48,
            max: 200,
        },
        "host" => ColumnWidth {
            default: 88,
            min: 64,
            max: 480,
        },
        "client_ip" => ColumnWidth {
            default: 100,
            min: 64,
            max: 260,
        },
        "size" => ColumnWidth {
            default: 66,
            min: 48,
            max: 180,
        },
        _ => return None,
    })
}

/// Default order of all list columns.
pub const DEFAULT_COLUMN_ORDER: [&str; 10] = [
    "id",
    "method",
    "path",
    "status",
    "gap",
    "elapsed",
    "size",
    "protocol",
    "host",
    "client_ip",
];

pub fn is_fixed_column_key(key: &str) -> bool {
    matches!(key, "id" | "method" | "path")
}

/// Resolves `order` into all 10 column keys while keeping fixed columns
/// (`id`, `method`, `path`) in the leading fixed zone and scrolling columns
/// in the trailing scrolling zone.
pub fn resolve_column_order(order: &[String]) -> Vec<String> {
    let mut fixed: Vec<String> = Vec::with_capacity(3);
    let mut scroll: Vec<String> = Vec::with_capacity(7);
    for raw in order {
        let key = raw.trim();
        let Some(&canonical) = DEFAULT_COLUMN_ORDER.iter().find(|&&k| k == key) else {
            continue;
        };
        let target = if is_fixed_column_key(canonical) {
            &mut fixed
        } else {
            &mut scroll
        };
        if !target.iter().any(|seen| seen == canonical) {
            target.push(canonical.to_string());
        }
    }
    for canonical in DEFAULT_COLUMN_ORDER {
        let target = if is_fixed_column_key(canonical) {
            &mut fixed
        } else {
            &mut scroll
        };
        if !target.iter().any(|seen| seen == canonical) {
            target.push(canonical.to_string());
        }
    }
    fixed.extend(scroll);
    fixed
}

/// Returns an empty `Vec` when `order` resolves to [`DEFAULT_COLUMN_ORDER`]
/// so default `config.toml` stays compact.
pub fn normalize_column_order(order: &[String]) -> Vec<String> {
    let resolved = resolve_column_order(order);
    if resolved.iter().map(String::as_str).eq(DEFAULT_COLUMN_ORDER) {
        Vec::new()
    } else {
        resolved
    }
}

/// Drops unknown keys and clamps the rest to [`column_width_limit`].
pub fn clamp_column_widths(widths: &mut HashMap<String, u32>) {
    widths.retain(|key, width| match column_width_limit(key) {
        Some(limit) => {
            *width = (*width).clamp(limit.min, limit.max);
            true
        }
        None => false,
    });
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_true() -> bool {
    true
}

fn default_list_width() -> u32 {
    LIST_WIDTH_DEFAULT
}

fn default_window_width() -> u32 {
    WINDOW_WIDTH_DEFAULT
}

fn default_window_height() -> u32 {
    WINDOW_HEIGHT_DEFAULT
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default_list_width(value: &u32) -> bool {
    *value == LIST_WIDTH_DEFAULT
}

fn is_default_window_width(value: &u32) -> bool {
    *value == WINDOW_WIDTH_DEFAULT
}

fn is_default_window_height(value: &u32) -> bool {
    *value == WINDOW_HEIGHT_DEFAULT
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

enum Read {
    Ready(Box<Config>),
    Missing,
    Broken(std::io::Error),
}

/// Reads `config.toml`, or the old one-file-per-setting layout the first time.
/// A broken file stays on disk and this returns the defaults.
#[cfg_attr(not(test), allow(dead_code))]
pub fn load(dir: &Path) -> Config {
    load_mode(dir, false)
}

/// Reads `config.toml` (`dev == false`) or `config.dev.toml` (`dev == true`).
/// Dev mode never touches `config.toml` or legacy single-setting files.
pub fn load_mode(dir: &Path, dev: bool) -> Config {
    let _guard = lock();
    match read_config(dir, dev) {
        Read::Ready(config) => *config,
        Read::Broken(_) => Config::default_for(dev),
        Read::Missing if dev => {
            let config = Config::default_for(true);
            let _ = store_unlocked(dir, true, &config);
            config
        }
        Read::Missing => {
            let config = migrate_legacy(dir);
            if store_unlocked(dir, false, &config).is_ok() {
                remove_legacy(dir);
            }
            config
        }
    }
}

/// Changes one or more fields and writes the whole file back.
/// A broken `config.toml` is left untouched.
#[cfg_attr(not(test), allow(dead_code))]
pub fn update(dir: &Path, change: impl FnOnce(&mut Config)) -> Result<()> {
    update_mode(dir, false, change)
}

pub fn update_mode(dir: &Path, dev: bool, change: impl FnOnce(&mut Config)) -> Result<()> {
    let _guard = lock();
    let mut config = match read_config(dir, dev) {
        Read::Ready(config) => *config,
        Read::Missing if dev => Config::default_for(true),
        Read::Missing => migrate_legacy(dir),
        Read::Broken(err) => return Err(err),
    };
    change(&mut config);
    config = normalize_for(config, dev);
    store_unlocked(dir, dev, &config)?;
    if !dev {
        remove_legacy(dir);
    }
    Ok(())
}

fn read_config(dir: &Path, dev: bool) -> Read {
    let path = dir.join(config_file_name(dev));
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => Read::Ready(Box::new(normalize_for(config, dev))),
            Err(err) => Read::Broken(std::io::Error::new(ErrorKind::InvalidData, err)),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Read::Missing,
        Err(err) => Read::Broken(err),
    }
}

fn migrate_legacy(dir: &Path) -> Config {
    let mut config = Config::default();
    if let Some(text) = read_legacy(dir, "listen-port")
        && let Some(port) = crate::present::parse_port(&text)
    {
        config.listen_port = port;
    }
    if let Some(text) = read_legacy(dir, "listen-all") {
        config.listen_all = text.trim() == "1";
    }
    if let Some(text) = read_legacy(dir, "use-system-proxy") {
        config.use_system_proxy = text.trim() == "1";
    }
    if let Some(text) = read_legacy(dir, "language") {
        config.language = language_name(parse_pref(&text));
    }
    if let Some(text) = read_legacy(dir, "bypass-hosts") {
        config.bypass_hosts = text;
    }
    if let Some(text) = read_legacy(dir, "rules") {
        config.rules = text;
    }
    if let Some(text) = read_legacy(dir, "upstream-proxy") {
        config.upstream_proxy = text;
    }
    config
}

pub fn language_name(lang: Option<Lang>) -> Option<String> {
    lang.map(|lang| {
        match lang {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
        .to_string()
    })
}

fn normalize_for(mut config: Config, dev: bool) -> Config {
    if config.listen_port == 0 {
        config.listen_port = default_listen_port(dev);
    }
    config.language = language_name(config.language.as_deref().and_then(parse_pref));
    config.list_width = clamp_list_width(config.list_width);
    config.window_width = clamp_window_width(config.window_width);
    config.window_height = clamp_window_height(config.window_height);
    clamp_column_widths(&mut config.column_widths);
    config.column_order = normalize_column_order(&config.column_order);
    clamp_saved_filters(&mut config.saved_filters);
    clamp_saved_rules(&mut config.saved_rules);
    migrate_saved_profiles(&mut config);
    config
}

fn migrate_saved_profiles(config: &mut Config) {
    if !config.saved_profiles.is_empty() {
        clamp_saved_profiles(&mut config.saved_profiles);
        return;
    }
    for entry in &config.saved_filters {
        let (name, expr) = entry.split_once('\t').unwrap_or(("", entry));
        let expr = expr.trim();
        if expr.is_empty() {
            continue;
        }
        let name = name.trim();
        config.saved_profiles.push(SavedProfile {
            name: if name.is_empty() {
                expr.chars().take(28).collect()
            } else {
                name.to_string()
            },
            filter: expr.to_string(),
            bypass_hosts: String::new(),
            rules: String::new(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        });
    }
    for rule in &config.saved_rules {
        let rules = rule.rules.trim();
        if rules.is_empty() {
            continue;
        }
        let name = rule.name.trim();
        config.saved_profiles.push(SavedProfile {
            name: if name.is_empty() {
                rules.lines().next().unwrap_or("规则").trim().to_string()
            } else {
                name.to_string()
            },
            filter: String::new(),
            bypass_hosts: String::new(),
            rules: rules.to_string(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        });
    }
    clamp_saved_profiles(&mut config.saved_profiles);
}

const SAVED_PROFILE_CAP: usize = 16;

fn clamp_saved_profiles(items: &mut Vec<SavedProfile>) {
    let mut kept = Vec::new();
    for item in items.drain(..) {
        let name = item.name.trim().to_string();
        if item.filter.trim().is_empty()
            && item.bypass_hosts.trim().is_empty()
            && item.rules.trim().is_empty()
            && item.upstream_proxy.trim().is_empty()
        {
            continue;
        }
        if kept
            .iter()
            .any(|existing: &SavedProfile| existing.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let name = if name.is_empty() {
            "配置".to_string()
        } else {
            name
        };
        kept.push(SavedProfile { name, ..item });
        if kept.len() == SAVED_PROFILE_CAP {
            break;
        }
    }
    *items = kept;
}

const SAVED_FILTER_CAP: usize = 16;
const SAVED_RULE_CAP: usize = 16;

fn clamp_saved_filters(items: &mut Vec<String>) {
    let mut kept = Vec::new();
    for item in items.drain(..) {
        let item = item.trim().to_string();
        if item.is_empty() || kept.iter().any(|existing: &String| existing == &item) {
            continue;
        }
        kept.push(item);
        if kept.len() == SAVED_FILTER_CAP {
            break;
        }
    }
    *items = kept;
}

fn clamp_saved_rules(items: &mut Vec<SavedRuleSet>) {
    let mut kept = Vec::new();
    for item in items.drain(..) {
        let name = item.name.trim().to_string();
        let rules = item.rules.trim().to_string();
        if rules.is_empty()
            || kept
                .iter()
                .any(|existing: &SavedRuleSet| existing.name == name && existing.rules == rules)
        {
            continue;
        }
        let name = if name.is_empty() {
            rules.lines().next().unwrap_or("规则").trim().to_string()
        } else {
            name
        };
        kept.push(SavedRuleSet { name, rules });
        if kept.len() == SAVED_RULE_CAP {
            break;
        }
    }
    *items = kept;
}

fn read_legacy(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok()
}

fn remove_legacy(dir: &Path) {
    for name in LEGACY {
        let _ = fs::remove_file(dir.join(name));
    }
}

fn store_unlocked(dir: &Path, dev: bool, config: &Config) -> Result<()> {
    let text = toml::to_string_pretty(config)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))?;
    write_atomic(&dir.join(config_file_name(dev)), &text)
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    fs::write(&tmp, text)?;
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "zniff-config-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn system_proxy_is_on_only_for_the_old_one_file() {
        for (text, on) in [
            ("", false),
            ("0\n", false),
            ("true", false),
            ("1", true),
            ("1\n", true),
        ] {
            let dir = Scratch::new();
            fs::write(dir.path().join("use-system-proxy"), text).unwrap();
            assert_eq!(load(dir.path()).use_system_proxy, on, "{text:?}");
        }
        let empty = Scratch::new();
        assert!(!load(empty.path()).use_system_proxy);
    }

    #[test]
    fn legacy_files_become_one_toml() {
        let dir = Scratch::new();
        fs::write(dir.path().join("listen-port"), "8877\n").unwrap();
        fs::write(dir.path().join("listen-all"), "1\n").unwrap();
        fs::write(dir.path().join("language"), "zh\n").unwrap();
        fs::write(dir.path().join("rules"), "break example.com /api\n").unwrap();
        fs::write(dir.path().join("upstream-proxy"), "127.0.0.1:9").unwrap();

        let config = load(dir.path());
        assert_eq!(config.listen_port, 8877);
        assert!(config.listen_all);
        assert!(!config.use_system_proxy);
        assert_eq!(config.language.as_deref(), Some("zh"));
        assert_eq!(config.rules, "break example.com /api\n");
        assert_eq!(config.upstream_proxy, "127.0.0.1:9");
        assert!(dir.path().join("config.toml").is_file());
        assert!(!dir.path().join("listen-port").exists());
        assert!(!dir.path().join("language").exists());
        assert!(!dir.path().join("rules").exists());

        let again = load(dir.path());
        assert_eq!(again, config);
    }

    #[test]
    fn update_keeps_the_other_fields() {
        let dir = Scratch::new();
        fs::write(dir.path().join("bypass-hosts"), "*.pin.dev\n").unwrap();
        load(dir.path());
        update(dir.path(), |config| {
            config.listen_port = 8080;
            config.rules = "map-local / \"quoted\"\n# keep\n".to_string();
        })
        .unwrap();
        update(dir.path(), |config| {
            config.language = Some("en".to_string())
        })
        .unwrap();
        update(dir.path(), |config| config.language = None).unwrap();

        let config = load(dir.path());
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.bypass_hosts, "*.pin.dev\n");
        assert_eq!(config.rules, "map-local / \"quoted\"\n# keep\n");
        assert_eq!(config.language, None);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("language"));
        assert!(text.contains("quoted"));
    }

    #[test]
    fn zero_port_and_a_broken_file_do_not_wipe_settings() {
        let dir = Scratch::new();
        fs::write(
            dir.path().join("config.toml"),
            "listen_port = 0\nlisten_all = true\n",
        )
        .unwrap();
        assert_eq!(load(dir.path()).listen_port, DEFAULT_PORT);
        assert!(load(dir.path()).listen_all);

        fs::write(dir.path().join("config.toml"), "listen_port = \"").unwrap();
        let broken = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert_eq!(load(dir.path()).listen_port, DEFAULT_PORT);
        let err = update(dir.path(), |config| config.listen_port = 9).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(
            fs::read_to_string(dir.path().join("config.toml")).unwrap(),
            broken
        );
    }

    #[test]
    fn list_layout_keeps_today_columns_and_clamps_width() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        let config = load(dir.path());
        assert!(
            config.show_id
                && config.show_method
                && config.show_status
                && config.show_path
                && config.show_gap
                && config.show_elapsed
                && config.show_size
        );
        assert!(!config.show_protocol && !config.show_host && !config.show_client_ip);
        assert_eq!(config.list_width, LIST_WIDTH_DEFAULT);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("show_status"), "{text}");
        assert!(!text.contains("show_method"), "{text}");
        assert!(!text.contains("show_client_ip"), "{text}");
        assert!(!text.contains("list_width"), "{text}");

        update(dir.path(), |config| {
            config.show_protocol = true;
            config.show_client_ip = true;
            config.show_size = false;
            config.list_width = 9_999;
        })
        .unwrap();
        let config = load(dir.path());
        assert!(config.show_protocol);
        assert!(config.show_client_ip);
        assert!(!config.show_size);
        assert!(config.show_status);
        assert_eq!(config.list_width, LIST_WIDTH_MAX);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("show_protocol = true"), "{text}");
        assert!(text.contains("show_client_ip = true"), "{text}");
        assert!(text.contains("show_size = false"), "{text}");
        assert!(!text.contains("show_status"), "{text}");
        assert!(!text.contains("show_host"), "{text}");
        assert!(
            text.contains(&format!("list_width = {LIST_WIDTH_MAX}")),
            "{text}"
        );

        fs::write(
            dir.path().join("config.toml"),
            "list_width = 10\nshow_path = false\n",
        )
        .unwrap();
        let config = load(dir.path());
        assert_eq!(config.list_width, LIST_WIDTH_MIN);
        assert!(!config.show_path);
        assert!(config.show_gap);
        assert!(!config.show_protocol);
        assert!(config.default_pretty);
    }

    #[test]
    fn column_widths_clamp_and_drop_unknown_keys() {
        let dir = Scratch::new();
        update(dir.path(), |config| {
            config.column_widths.insert("path".into(), 9_999);
            config.column_widths.insert("method".into(), 1);
            config.column_widths.insert("nope".into(), 80);
        })
        .unwrap();
        let config = load(dir.path());
        assert_eq!(config.column_widths.get("path").copied(), Some(800));
        assert_eq!(config.column_widths.get("method").copied(), Some(56));
        assert!(!config.column_widths.contains_key("nope"));
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("path = 800"), "{text}");
        assert!(!text.contains("nope"), "{text}");
    }

    #[test]
    fn default_pretty_stays_on_unless_the_file_turns_it_off() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        assert!(load(dir.path()).default_pretty);
        update(dir.path(), |config| config.default_pretty = false).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("default_pretty = false"), "{text}");
        assert!(!load(dir.path()).default_pretty);
        update(dir.path(), |config| config.default_pretty = true).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("default_pretty"), "{text}");
    }

    #[test]
    fn tunnel_failures_stay_hidden_unless_the_file_shows_them() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        assert!(!load(dir.path()).show_tunnel_failures);
        update(dir.path(), |config| config.show_tunnel_failures = true).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("show_tunnel_failures = true"), "{text}");
        assert!(load(dir.path()).show_tunnel_failures);
        update(dir.path(), |config| config.show_tunnel_failures = false).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("show_tunnel_failures"), "{text}");
    }

    #[test]
    fn filter_text_is_kept_and_absent_until_set() {
        let dir = Scratch::new();
        fs::write(
            dir.path().join("config.toml"),
            "listen_port = 8877\nrecent_filters = [\"api.test\"]\n",
        )
        .unwrap();
        let config = load(dir.path());
        assert!(config.filter.is_empty());
        update(dir.path(), |config| {
            config.filter = "api.test".into();
        })
        .unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.filter, "api.test");
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("recent_filters"), "{text}");
        update(dir.path(), |config| {
            config.filter.clear();
        })
        .unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("filter"), "{text}");
    }

    #[test]
    fn capturing_stays_off_unless_the_file_says_so() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        assert!(!load(dir.path()).capturing);
        update(dir.path(), |config| config.capturing = true).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("capturing = true"), "{text}");
        assert!(load(dir.path()).capturing);
        update(dir.path(), |config| config.capturing = false).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("capturing"), "{text}");
    }

    #[test]
    fn dev_mode_isolates_config_file_and_default_port() {
        let dir = Scratch::new();
        fs::write(
            dir.path().join("config.toml"),
            "listen_port = 8877\ncapturing = true\nrules = \"break prod.test /\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("listen-port"), "9000\n").unwrap();

        let dev = load_mode(dir.path(), true);
        assert_eq!(dev.listen_port, DEV_DEFAULT_PORT);
        assert!(!dev.capturing);
        assert!(dev.rules.is_empty());
        assert!(dir.path().join("config.dev.toml").is_file());
        assert!(dir.path().join("listen-port").is_file());

        update_mode(dir.path(), true, |config| {
            config.rules = "delay dev.test / 100ms".into();
            config.listen_port = 0;
        })
        .unwrap();

        let dev_again = load_mode(dir.path(), true);
        assert_eq!(dev_again.listen_port, DEV_DEFAULT_PORT);
        assert_eq!(dev_again.rules, "delay dev.test / 100ms");

        let prod = load_mode(dir.path(), false);
        assert_eq!(prod.listen_port, 8877);
        assert!(prod.capturing);
        assert_eq!(prod.rules, "break prod.test /");

        assert!(is_dev_env(Some("1")));
        assert!(is_dev_env(Some(" true ")));
        assert!(!is_dev_env(Some("0")));
        assert!(!is_dev_env(None));
    }

    #[test]
    fn full_url_and_ellipsize_persist_and_default_cleanly() {
        let dir = Scratch::new();
        let initial = load(dir.path());
        assert!(!initial.show_full_url);
        assert_eq!(initial.ellipsize, ListEllipsize::Middle);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("show_full_url"), "{text}");
        assert!(!text.contains("ellipsize"), "{text}");

        update(dir.path(), |config| {
            config.show_full_url = true;
            config.ellipsize = ListEllipsize::Head;
        })
        .unwrap();
        let updated = load(dir.path());
        assert!(updated.show_full_url);
        assert_eq!(updated.ellipsize, ListEllipsize::Head);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("show_full_url = true"), "{text}");
        assert!(text.contains("ellipsize = \"head\""), "{text}");

        fs::write(
            dir.path().join("config.toml"),
            "ellipsize = \"tail\"\nshow_full_url = false\n",
        )
        .unwrap();
        let tail = load(dir.path());
        assert!(!tail.show_full_url);
        assert_eq!(tail.ellipsize, ListEllipsize::Tail);

        fs::write(dir.path().join("config.toml"), "ellipsize = \"invalid\"\n").unwrap();
        assert_eq!(load(dir.path()).ellipsize, ListEllipsize::Middle);
    }

    #[test]
    fn saved_profiles_export_import_and_merge_case_insensitively() {
        let p1 = SavedProfile {
            name: "Staging".into(),
            filter: "host:staging.test".into(),
            bypass_hosts: String::new(),
            rules: "delay staging.test/api 200ms".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: false,
            show_tunnel_failures: true,
        };
        let p2 = SavedProfile {
            name: "Local".into(),
            filter: String::new(),
            bypass_hosts: "*.internal".into(),
            rules: String::new(),
            upstream_proxy: "127.0.0.1:9090".into(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        };
        let toml_text = export_profiles_toml(&[p1.clone(), p2.clone()]).unwrap();
        assert!(toml_text.contains("[[profiles]]"), "{toml_text}");
        let parsed = parse_profiles_toml(&toml_text).unwrap();
        assert_eq!(parsed, vec![p1.clone(), p2.clone()]);

        let single_toml = "name = \"staging\"\nfilter = \"host:new.staging.test\"\n";
        let single_parsed = parse_profiles_toml(single_toml).unwrap();
        assert_eq!(single_parsed.len(), 1);
        assert_eq!(single_parsed[0].name, "staging");

        let merged = merge_imported_profiles(&[p1, p2.clone()], &single_parsed, 16);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "staging");
        assert_eq!(merged[0].filter, "host:new.staging.test");
        assert_eq!(merged[1], p2);

        assert!(parse_profiles_toml("").is_err());
        assert!(parse_profiles_toml("name = \"empty_only\"\n").is_err());
    }

    #[test]
    fn window_size_defaults_cleanly_and_clamps_bounds() {
        let dir = Scratch::new();
        let initial = load(dir.path());
        assert_eq!(initial.window_width, WINDOW_WIDTH_DEFAULT);
        assert_eq!(initial.window_height, WINDOW_HEIGHT_DEFAULT);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("window_width"), "{text}");
        assert!(!text.contains("window_height"), "{text}");

        update(dir.path(), |config| {
            config.window_width = 1440;
            config.window_height = 900;
        })
        .unwrap();
        let resized = load(dir.path());
        assert_eq!(resized.window_width, 1440);
        assert_eq!(resized.window_height, 900);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("window_width = 1440"), "{text}");
        assert!(text.contains("window_height = 900"), "{text}");

        update(dir.path(), |config| {
            config.window_width = 100;
            config.window_height = 99_999;
        })
        .unwrap();
        let clamped = load(dir.path());
        assert_eq!(clamped.window_width, WINDOW_WIDTH_MIN);
        assert_eq!(clamped.window_height, WINDOW_HEIGHT_MAX);
    }

    #[test]
    fn column_order_normalizes_zones_and_omits_default_order() {
        let dir = Scratch::new();
        let initial = load(dir.path());
        assert!(initial.column_order.is_empty());
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("column_order"), "{text}");

        update(dir.path(), |config| {
            config.column_order = vec![
                "method".into(),
                "host".into(),
                "id".into(),
                "unknown".into(),
                "client_ip".into(),
                "status".into(),
            ];
        })
        .unwrap();
        let loaded = load(dir.path());
        assert_eq!(
            loaded.column_order,
            vec![
                "method",
                "id",
                "path",
                "host",
                "client_ip",
                "status",
                "gap",
                "elapsed",
                "size",
                "protocol",
            ]
        );
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("column_order"), "{text}");

        update(dir.path(), |config| {
            config.column_order = DEFAULT_COLUMN_ORDER.iter().map(|s| s.to_string()).collect();
        })
        .unwrap();
        let reset = load(dir.path());
        assert!(reset.column_order.is_empty());
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("column_order"), "{text}");
    }
}
