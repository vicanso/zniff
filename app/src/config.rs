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

static PROXY_AUTH_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

/// Overrides `proxy_auth` for the current process (used by `--proxy-auth`).
pub fn set_proxy_auth_override(auth: impl Into<String>) {
    *PROXY_AUTH_OVERRIDE
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(auth.into().trim().to_string());
}

pub fn resolve_proxy_auth(cli: Option<&str>, env: Option<&str>, saved: &str) -> String {
    if let Some(cli) = cli {
        return cli.trim().to_string();
    }
    if let Some(env) = env {
        return env.trim().to_string();
    }
    saved.trim().to_string()
}

pub fn runtime_proxy_auth(saved: &str) -> String {
    let cli = PROXY_AUTH_OVERRIDE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let env = std::env::var("ZNIFF_PROXY_AUTH").ok();
    resolve_proxy_auth(cli.as_deref(), env.as_deref(), saved)
}

/// Left pane width when `config.toml` has no `list_width`.
pub const LIST_WIDTH_DEFAULT: u32 = 660;
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
#[serde(from = "RawConfig", into = "RawConfig")]
pub struct Config {
    pub listen_port: u16,
    pub listen_all: bool,
    pub use_system_proxy: bool,
    /// `None` follows the system locale. Only `zh` and `en` are stored.
    pub language: Option<String>,
    /// Name of the active entry in `saved_profiles`, or empty when using an unnamed setup.
    pub active_profile: String,
    pub bypass_hosts: String,
    pub rules: String,
    pub upstream_proxy: String,
    pub rules_enabled: bool,
    pub breaks_enabled: bool,
    /// On unless the file says otherwise, so an older `config.toml` keeps today's list.
    pub show_id: bool,
    pub show_method: bool,
    pub show_status: bool,
    pub show_protocol: bool,
    pub show_path: bool,
    pub show_gap: bool,
    /// On unless the file says otherwise. Time from the request to the full response.
    pub show_elapsed: bool,
    pub show_host: bool,
    pub show_client_ip: bool,
    pub show_size: bool,
    /// False shows only `flow.path` in the Path column; true shows `flow.url`.
    pub show_full_url: bool,
    /// Where to ellipsize overflowing Path/URL and Host cells (`head`, `middle`, `tail`).
    pub ellipsize: ListEllipsize,
    pub list_width: u32,
    pub window_width: u32,
    pub window_height: u32,
    /// Pretty is the body default unless the file says otherwise.
    pub default_pretty: bool,
    /// Origin TLS probe failures stay out of the list unless this is on.
    pub show_tunnel_failures: bool,
    /// Restored into the filter box on the next launch.
    pub filter: String,
    /// The last window was capturing. The next launch presses Start itself.
    pub capturing: bool,
    /// Filters the user saved on purpose. Newest first.
    pub saved_filters: Vec<String>,
    /// Named rule presets the user saved on purpose. Newest first.
    pub saved_rules: Vec<SavedRuleSet>,
    /// Filter, bypass, rules, and upstream saved as one preset. Newest first.
    pub saved_profiles: Vec<SavedProfile>,
    /// Recent setup snapshots (up to 10, newest first) for recovering previous versions.
    pub profile_history: Vec<SavedProfile>,
    /// Dragged list-column widths, in points, keyed by `method`, `path`, and the rest.
    pub column_widths: HashMap<String, u32>,
    /// Custom list-column order when reordered from [`DEFAULT_COLUMN_ORDER`].
    pub column_order: Vec<String>,
    /// Sanitize sensitive headers, query params, and JSON/form fields on HAR/HTML export and live share.
    pub redact_export: bool,
    /// When `redact_export` is enabled, replace binary or unstructured bodies with a placeholder instead of exporting raw bytes.
    pub redact_strict: bool,
    /// Extra comma-separated sensitive header or field names to redact on export/share.
    pub redact_fields: String,
    /// Base URL of a `zniff --relay` process, such as `http://10.0.0.1:8877`.
    /// Empty keeps the share link on this computer.
    pub relay_url: String,
    /// Bearer token sent with share uploads. The relay process expects the same value.
    pub relay_token: String,
    /// Accept relay uploads on the capture port. Off until the user turns it on.
    pub relay_accept: bool,
    /// Optional HTTP Basic credential (`user:pass`) required on the capture port.
    /// Belongs to this machine; omitted from saved profiles.
    pub proxy_auth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawConfig {
    #[serde(default)]
    listen_port: u16,
    #[serde(default)]
    listen_all: bool,
    #[serde(default)]
    use_system_proxy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bypass_hosts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upstream_proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    breaks_enabled: Option<bool>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_id: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_method: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_status: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    show_protocol: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_path: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_gap: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_elapsed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    show_host: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    show_client_ip: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    show_size: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    show_full_url: bool,
    #[serde(default, skip_serializing_if = "ListEllipsize::is_default")]
    ellipsize: ListEllipsize,
    #[serde(
        default = "default_list_width",
        skip_serializing_if = "is_default_list_width"
    )]
    list_width: u32,
    #[serde(
        default = "default_window_width",
        skip_serializing_if = "is_default_window_width"
    )]
    window_width: u32,
    #[serde(
        default = "default_window_height",
        skip_serializing_if = "is_default_window_height"
    )]
    window_height: u32,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    default_pretty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_tunnel_failures: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filter: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    capturing: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    column_order: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    redact_export: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    redact_strict: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    redact_fields: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    relay_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    relay_token: String,
    #[serde(default, skip_serializing_if = "is_false")]
    relay_accept: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    proxy_auth: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    saved_filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    saved_rules: Vec<SavedRuleSet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    saved_profiles: Vec<SavedProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    profile_history: Vec<SavedProfile>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    column_widths: HashMap<String, u32>,
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        let mut saved_filters = raw.saved_filters;
        let mut saved_rules = raw.saved_rules;
        let mut saved_profiles = raw.saved_profiles;
        let mut profile_history = raw.profile_history;
        clamp_saved_filters(&mut saved_filters);
        clamp_saved_rules(&mut saved_rules);
        migrate_saved_profiles_from(&saved_filters, &saved_rules, &mut saved_profiles);
        clamp_profile_history(&mut profile_history);

        let explicit_active = raw
            .active_profile
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());

        let (
            active_profile,
            filter,
            bypass_hosts,
            rules,
            upstream_proxy,
            rules_enabled,
            breaks_enabled,
            show_tunnel_failures,
        ) = if let Some(preset) = explicit_active.and_then(|name| {
            saved_profiles
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(name))
        }) {
            (
                preset.name.clone(),
                raw.filter.unwrap_or_else(|| preset.filter.clone()),
                raw.bypass_hosts
                    .unwrap_or_else(|| preset.bypass_hosts.clone()),
                raw.rules.unwrap_or_else(|| preset.rules.clone()),
                raw.upstream_proxy
                    .unwrap_or_else(|| preset.upstream_proxy.clone()),
                raw.rules_enabled.unwrap_or(preset.rules_enabled),
                raw.breaks_enabled.unwrap_or(preset.breaks_enabled),
                raw.show_tunnel_failures
                    .unwrap_or(preset.show_tunnel_failures),
            )
        } else {
            let filter = raw.filter.unwrap_or_default();
            let bypass_hosts = raw.bypass_hosts.unwrap_or_default();
            let rules = raw.rules.unwrap_or_default();
            let upstream_proxy = raw.upstream_proxy.unwrap_or_default();
            let mut rules_enabled = raw.rules_enabled.unwrap_or(true);
            let mut breaks_enabled = raw.breaks_enabled.unwrap_or(true);
            let show_tunnel_failures = raw.show_tunnel_failures.unwrap_or(false);
            let has_setup_content = !filter.trim().is_empty()
                || !bypass_hosts.trim().is_empty()
                || !rules.trim().is_empty()
                || !upstream_proxy.trim().is_empty();
            let inferred_active = if raw.active_profile.is_none() && has_setup_content {
                let probe = SavedProfile {
                    name: String::new(),
                    filter: filter.clone(),
                    bypass_hosts: bypass_hosts.clone(),
                    rules: rules.clone(),
                    upstream_proxy: upstream_proxy.clone(),
                    rules_enabled,
                    breaks_enabled,
                    show_tunnel_failures,
                };
                if let Some(preset) = saved_profiles
                    .iter()
                    .find(|item| same_profile_content(item, &probe))
                {
                    preset.name.clone()
                } else if raw.rules_enabled.is_none()
                    && raw.breaks_enabled.is_none()
                    && let Some(preset) = saved_profiles.iter().find(|item| {
                        item.filter.trim() == filter.trim()
                            && item.bypass_hosts.trim() == bypass_hosts.trim()
                            && item.rules.trim() == rules.trim()
                            && item.upstream_proxy.trim() == upstream_proxy.trim()
                            && item.show_tunnel_failures == show_tunnel_failures
                    })
                {
                    rules_enabled = preset.rules_enabled;
                    breaks_enabled = preset.breaks_enabled;
                    preset.name.clone()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            (
                inferred_active,
                filter,
                bypass_hosts,
                rules,
                upstream_proxy,
                rules_enabled,
                breaks_enabled,
                show_tunnel_failures,
            )
        };

        Self {
            listen_port: raw.listen_port,
            listen_all: raw.listen_all,
            use_system_proxy: raw.use_system_proxy,
            language: raw.language,
            active_profile,
            bypass_hosts,
            rules,
            upstream_proxy,
            rules_enabled,
            breaks_enabled,
            show_id: raw.show_id,
            show_method: raw.show_method,
            show_status: raw.show_status,
            show_protocol: raw.show_protocol,
            show_path: raw.show_path,
            show_gap: raw.show_gap,
            show_elapsed: raw.show_elapsed,
            show_host: raw.show_host,
            show_client_ip: raw.show_client_ip,
            show_size: raw.show_size,
            show_full_url: raw.show_full_url,
            ellipsize: raw.ellipsize,
            list_width: raw.list_width,
            window_width: raw.window_width,
            window_height: raw.window_height,
            default_pretty: raw.default_pretty,
            show_tunnel_failures,
            filter,
            capturing: raw.capturing,
            saved_filters,
            saved_rules,
            saved_profiles,
            profile_history,
            column_widths: raw.column_widths,
            column_order: raw.column_order,
            redact_export: raw.redact_export,
            redact_strict: raw.redact_strict,
            redact_fields: raw.redact_fields,
            relay_url: raw.relay_url,
            relay_token: raw.relay_token,
            relay_accept: raw.relay_accept,
            proxy_auth: raw.proxy_auth,
        }
    }
}

impl From<Config> for RawConfig {
    fn from(config: Config) -> Self {
        let active_name = config.active_profile.trim();
        let matched = (!active_name.is_empty())
            .then(|| {
                config
                    .saved_profiles
                    .iter()
                    .find(|item| item.name.eq_ignore_ascii_case(active_name))
            })
            .flatten();

        let (
            active_profile,
            filter,
            bypass_hosts,
            rules,
            upstream_proxy,
            rules_enabled,
            breaks_enabled,
            show_tunnel_failures,
        ) = if let Some(preset) = matched {
            (
                Some(preset.name.clone()),
                (config.filter.trim() != preset.filter.trim()).then(|| config.filter.clone()),
                (config.bypass_hosts.trim() != preset.bypass_hosts.trim())
                    .then(|| config.bypass_hosts.clone()),
                (config.rules.trim() != preset.rules.trim()).then(|| config.rules.clone()),
                (config.upstream_proxy.trim() != preset.upstream_proxy.trim())
                    .then(|| config.upstream_proxy.clone()),
                (config.rules_enabled != preset.rules_enabled).then_some(config.rules_enabled),
                (config.breaks_enabled != preset.breaks_enabled).then_some(config.breaks_enabled),
                (config.show_tunnel_failures != preset.show_tunnel_failures)
                    .then_some(config.show_tunnel_failures),
            )
        } else {
            (
                None,
                (!config.filter.is_empty()).then_some(config.filter),
                (!config.bypass_hosts.is_empty()).then_some(config.bypass_hosts),
                (!config.rules.is_empty()).then_some(config.rules),
                (!config.upstream_proxy.is_empty()).then_some(config.upstream_proxy),
                (!config.rules_enabled).then_some(false),
                (!config.breaks_enabled).then_some(false),
                config.show_tunnel_failures.then_some(true),
            )
        };

        Self {
            listen_port: config.listen_port,
            listen_all: config.listen_all,
            use_system_proxy: config.use_system_proxy,
            language: config.language,
            active_profile,
            bypass_hosts,
            rules,
            upstream_proxy,
            rules_enabled,
            breaks_enabled,
            show_id: config.show_id,
            show_method: config.show_method,
            show_status: config.show_status,
            show_protocol: config.show_protocol,
            show_path: config.show_path,
            show_gap: config.show_gap,
            show_elapsed: config.show_elapsed,
            show_host: config.show_host,
            show_client_ip: config.show_client_ip,
            show_size: config.show_size,
            show_full_url: config.show_full_url,
            ellipsize: config.ellipsize,
            list_width: config.list_width,
            window_width: config.window_width,
            window_height: config.window_height,
            default_pretty: config.default_pretty,
            show_tunnel_failures,
            filter,
            capturing: config.capturing,
            column_order: config.column_order,
            redact_export: config.redact_export,
            redact_strict: config.redact_strict,
            redact_fields: config.redact_fields,
            relay_url: config.relay_url,
            relay_token: config.relay_token,
            relay_accept: config.relay_accept,
            proxy_auth: config.proxy_auth,
            saved_filters: config.saved_filters,
            saved_rules: config.saved_rules,
            saved_profiles: config.saved_profiles,
            profile_history: config.profile_history,
            column_widths: config.column_widths,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_port: default_port(),
            listen_all: false,
            use_system_proxy: false,
            language: None,
            active_profile: String::new(),
            bypass_hosts: String::new(),
            rules: String::new(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
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
            profile_history: Vec::new(),
            column_widths: HashMap::new(),
            column_order: Vec::new(),
            redact_export: false,
            redact_strict: false,
            redact_fields: String::new(),
            relay_url: String::new(),
            relay_token: String::new(),
            relay_accept: false,
            proxy_auth: String::new(),
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
            default: 74,
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
    Ready {
        config: Box<Config>,
        migrated_active: bool,
    },
    Missing,
    Broken(std::io::Error),
}

/// Reads `config.toml`, or the old one-file-per-setting layout the first time.
/// A broken file stays on disk and this returns the defaults.
#[cfg_attr(not(test), allow(dead_code))]
static LAST_LOAD_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[allow(dead_code)]
pub fn last_load_error() -> Option<String> {
    LAST_LOAD_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn take_last_load_error() -> Option<String> {
    LAST_LOAD_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn load(dir: &Path) -> Config {
    load_mode(dir, false)
}

pub fn load_mode_ext(dir: &Path, dev: bool) -> (Config, Option<String>) {
    let _guard = lock();
    match read_config(dir, dev) {
        Read::Ready {
            config,
            migrated_active,
        } => {
            if migrated_active {
                let _ = store_unlocked(dir, dev, &config);
            }
            *LAST_LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
            (*config, None)
        }
        Read::Broken(err) => {
            let msg = err.to_string();
            *LAST_LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg.clone());
            (Config::default_for(dev), Some(msg))
        }
        Read::Missing if dev => {
            let config = Config::default_for(true);
            let _ = store_unlocked(dir, true, &config);
            *LAST_LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
            (config, None)
        }
        Read::Missing => {
            let config = migrate_legacy(dir);
            if store_unlocked(dir, false, &config).is_ok() {
                remove_legacy(dir);
            }
            *LAST_LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
            (config, None)
        }
    }
}

/// Reads `config.toml` (`dev == false`) or `config.dev.toml` (`dev == true`).
/// Dev mode never touches `config.toml` or legacy single-setting files.
pub fn load_mode(dir: &Path, dev: bool) -> Config {
    load_mode_ext(dir, dev).0
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
        Read::Ready { config, .. } => *config,
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
    *LAST_LOAD_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

fn read_config(dir: &Path, dev: bool) -> Read {
    let path = dir.join(config_file_name(dev));
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<RawConfig>(&text) {
            Ok(raw) => {
                let had_active = raw.active_profile.is_some();
                let config = normalize_for(Config::from(raw), dev);
                let migrated_active = !had_active && !config.active_profile.is_empty();
                Read::Ready {
                    config: Box::new(config),
                    migrated_active,
                }
            }
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
    config.relay_url = config.relay_url.trim().to_string();
    config.relay_token = config.relay_token.trim().to_string();
    config.proxy_auth = config.proxy_auth.trim().to_string();
    clamp_saved_filters(&mut config.saved_filters);
    clamp_saved_rules(&mut config.saved_rules);
    migrate_saved_profiles(&mut config);
    clamp_profile_history(&mut config.profile_history);
    let active = config.active_profile.trim();
    config.active_profile = if active.is_empty() {
        String::new()
    } else {
        config
            .saved_profiles
            .iter()
            .find(|preset| preset.name.eq_ignore_ascii_case(active))
            .map(|preset| preset.name.clone())
            .unwrap_or_default()
    };
    config
}

fn migrate_saved_profiles(config: &mut Config) {
    migrate_saved_profiles_from(
        &config.saved_filters,
        &config.saved_rules,
        &mut config.saved_profiles,
    );
}

fn migrate_saved_profiles_from(
    saved_filters: &[String],
    saved_rules: &[SavedRuleSet],
    saved_profiles: &mut Vec<SavedProfile>,
) {
    if !saved_profiles.is_empty() {
        clamp_saved_profiles(saved_profiles);
        return;
    }
    for entry in saved_filters {
        let (name, expr) = entry.split_once('\t').unwrap_or(("", entry));
        let expr = expr.trim();
        if expr.is_empty() {
            continue;
        }
        let name = name.trim();
        saved_profiles.push(SavedProfile {
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
    for rule in saved_rules {
        let rules = rule.rules.trim();
        if rules.is_empty() {
            continue;
        }
        let name = rule.name.trim();
        saved_profiles.push(SavedProfile {
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
    clamp_saved_profiles(saved_profiles);
}

const SAVED_PROFILE_CAP: usize = 16;
pub const PROFILE_HISTORY_CAP: usize = 10;

pub fn same_profile_content(a: &SavedProfile, b: &SavedProfile) -> bool {
    a.filter.trim() == b.filter.trim()
        && a.bypass_hosts.trim() == b.bypass_hosts.trim()
        && a.rules.trim() == b.rules.trim()
        && a.upstream_proxy.trim() == b.upstream_proxy.trim()
        && a.rules_enabled == b.rules_enabled
        && a.breaks_enabled == b.breaks_enabled
        && a.show_tunnel_failures == b.show_tunnel_failures
}

fn clamp_profile_history(items: &mut Vec<SavedProfile>) {
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
        let name = if name.is_empty() {
            "配置".to_string()
        } else {
            name
        };
        let normalized = SavedProfile {
            name,
            filter: item.filter.trim().to_string(),
            bypass_hosts: item.bypass_hosts.trim().to_string(),
            rules: item.rules.trim().to_string(),
            upstream_proxy: item.upstream_proxy.trim().to_string(),
            rules_enabled: item.rules_enabled,
            breaks_enabled: item.breaks_enabled,
            show_tunnel_failures: item.show_tunnel_failures,
        };
        if kept
            .iter()
            .any(|existing: &SavedProfile| same_profile_content(existing, &normalized))
        {
            continue;
        }
        kept.push(normalized);
        if kept.len() == PROFILE_HISTORY_CAP {
            break;
        }
    }
    *items = kept;
}

fn clamp_saved_profiles(items: &mut Vec<SavedProfile>) {
    let mut kept = Vec::new();
    for item in items.drain(..) {
        let name = item.name.trim().to_string();
        let filter = item.filter.trim().to_string();
        let bypass_hosts = item.bypass_hosts.trim().to_string();
        let rules = item.rules.trim().to_string();
        let upstream_proxy = item.upstream_proxy.trim().to_string();
        if filter.is_empty()
            && bypass_hosts.is_empty()
            && rules.is_empty()
            && upstream_proxy.is_empty()
        {
            continue;
        }
        let name = if name.is_empty() {
            "配置".to_string()
        } else {
            name
        };
        if kept
            .iter()
            .any(|existing: &SavedProfile| existing.name.eq_ignore_ascii_case(&name))
        {
            continue;
        }
        kept.push(SavedProfile {
            name,
            filter,
            bypass_hosts,
            rules,
            upstream_proxy,
            rules_enabled: item.rules_enabled,
            breaks_enabled: item.breaks_enabled,
            show_tunnel_failures: item.show_tunnel_failures,
        });
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
    fn size_column_width_default_is_74() {
        let limit = column_width_limit("size").unwrap();
        assert_eq!(limit.default, 74);
        assert_eq!(limit.min, 48);
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
    fn relay_settings_default_empty_and_trim_on_write() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 9\n").unwrap();
        let config = load(dir.path());
        assert_eq!(config.relay_url, "");
        assert_eq!(config.relay_token, "");
        assert!(!config.relay_accept);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("relay_"), "{text}");

        update(dir.path(), |config| {
            config.relay_url = "  http://10.1.2.3:9/  ".into();
            config.relay_token = "\tsecret\t".into();
            config.relay_accept = true;
        })
        .unwrap();
        let config = load(dir.path());
        assert_eq!(config.relay_url, "http://10.1.2.3:9/");
        assert_eq!(config.relay_token, "secret");
        assert!(config.relay_accept);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("relay_accept = true"), "{text}");

        update(dir.path(), |config| {
            config.relay_url.clear();
            config.relay_token.clear();
            config.relay_accept = false;
        })
        .unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("relay_"), "{text}");
    }

    #[test]
    fn proxy_auth_persists_at_machine_level_and_resolves_overrides() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 9\n").unwrap();
        let config = load(dir.path());
        assert_eq!(config.proxy_auth, "");
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("proxy_auth"), "{text}");

        update(dir.path(), |config| {
            config.proxy_auth = "  alice:s3cr3t  ".into();
        })
        .unwrap();
        let config = load(dir.path());
        assert_eq!(config.proxy_auth, "alice:s3cr3t");
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("proxy_auth = \"alice:s3cr3t\""), "{text}");

        update(dir.path(), |config| config.proxy_auth.clear()).unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("proxy_auth"), "{text}");

        assert_eq!(
            resolve_proxy_auth(Some("cli:pw"), Some("env:pw"), "cfg:pw"),
            "cli:pw"
        );
        assert_eq!(
            resolve_proxy_auth(None, Some("  env:pw  "), "cfg:pw"),
            "env:pw"
        );
        assert_eq!(resolve_proxy_auth(None, Some(""), "cfg:pw"), "");
        assert_eq!(resolve_proxy_auth(None, None, " cfg:pw "), "cfg:pw");
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

    #[test]
    fn load_mode_ext_captures_broken_toml_syntax_error() {
        let dir = Scratch::new();
        fs::write(
            dir.path().join("config.toml"),
            "listen_port = \"unclosed string",
        )
        .unwrap();
        let (config, err) = load_mode_ext(dir.path(), false);
        assert_eq!(config.listen_port, DEFAULT_PORT);
        assert!(err.is_some());
        let err_msg = err.unwrap();
        assert!(
            err_msg.contains("TOML parse error") || err_msg.contains("invalid data"),
            "{err_msg}"
        );
        assert!(last_load_error().is_some());
        assert_eq!(take_last_load_error().as_deref(), Some(err_msg.as_str()));
        assert!(last_load_error().is_none());
    }

    #[test]
    fn update_mode_preserves_broken_file_without_clobbering() {
        let dir = Scratch::new();
        let broken_content = "listen_port = \"broken string\n";
        fs::write(dir.path().join("config.toml"), broken_content).unwrap();

        let res = update_mode(dir.path(), false, |cfg| cfg.listen_port = 8899);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), ErrorKind::InvalidData);

        let file_after = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert_eq!(file_after, broken_content);
    }

    #[test]
    fn load_mode_ext_clears_error_on_valid_config() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8899\n").unwrap();
        let (config, err) = load_mode_ext(dir.path(), false);
        assert_eq!(config.listen_port, 8899);
        assert!(err.is_none());
        assert!(last_load_error().is_none());
    }

    #[test]
    fn active_profile_omits_duplicate_top_level_setup_fields_and_restores_preset() {
        let dir = Scratch::new();
        let test_profile = SavedProfile {
            name: "test".into(),
            filter: "host:test.gf.com.cn path:/finance-api".into(),
            bypass_hosts: String::new(),
            rules: "replace */configs req supportCrypto=true -> supportCrypto=false".into(),
            upstream_proxy: String::new(),
            rules_enabled: false,
            breaks_enabled: false,
            show_tunnel_failures: true,
        };
        let google_profile = SavedProfile {
            name: "google".into(),
            filter: String::new(),
            bypass_hosts: "google.com".into(),
            rules: String::new(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        };
        update(dir.path(), |config| {
            config.saved_profiles = vec![test_profile.clone(), google_profile];
            config.active_profile = "TEST".into();
            config.filter = test_profile.filter.clone();
            config.bypass_hosts = test_profile.bypass_hosts.clone();
            config.rules = test_profile.rules.clone();
            config.upstream_proxy = test_profile.upstream_proxy.clone();
            config.rules_enabled = test_profile.rules_enabled;
            config.breaks_enabled = test_profile.breaks_enabled;
            config.show_tunnel_failures = test_profile.show_tunnel_failures;
            config.column_widths.insert("path".into(), 280);
            config.relay_accept = true;
        })
        .unwrap();

        let raw_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let before_tables = raw_toml.split("[[").next().unwrap();
        assert!(
            before_tables.contains("active_profile = \"test\""),
            "{raw_toml}"
        );
        assert!(!before_tables.contains("\nfilter ="), "{raw_toml}");
        assert!(!before_tables.contains("\nrules ="), "{raw_toml}");
        assert!(!before_tables.contains("bypass_hosts"), "{raw_toml}");
        assert!(!before_tables.contains("upstream_proxy"), "{raw_toml}");
        assert!(!before_tables.contains("rules_enabled"), "{raw_toml}");
        assert!(!before_tables.contains("breaks_enabled"), "{raw_toml}");
        assert!(
            !before_tables.contains("show_tunnel_failures"),
            "{raw_toml}"
        );

        let reloaded = load(dir.path());
        assert_eq!(reloaded.active_profile, "test");
        assert_eq!(reloaded.filter, test_profile.filter);
        assert_eq!(reloaded.rules, test_profile.rules);
        assert!(reloaded.bypass_hosts.is_empty());
        assert!(reloaded.upstream_proxy.is_empty());
        assert!(!reloaded.rules_enabled);
        assert!(!reloaded.breaks_enabled);
        assert!(reloaded.show_tunnel_failures);
        assert!(reloaded.relay_accept);
        assert_eq!(reloaded.column_widths.get("path").copied(), Some(280));
    }

    #[test]
    fn active_profile_persists_only_unsaved_overrides_including_empty_clear() {
        let dir = Scratch::new();
        let preset = SavedProfile {
            name: "test".into(),
            filter: "host:api.example.com".into(),
            bypass_hosts: String::new(),
            rules: "delay api.example.com/* 100ms".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        };
        update(dir.path(), |config| {
            config.saved_profiles = vec![preset.clone()];
            config.active_profile = "test".into();
            config.filter = preset.filter.clone();
            config.rules = preset.rules.clone();
        })
        .unwrap();

        // User temporarily clears `filter` without clicking Save yet.
        update(dir.path(), |config| {
            config.filter.clear();
            config.breaks_enabled = false;
        })
        .unwrap();

        let raw_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let before_tables = raw_toml.split("[[").next().unwrap();
        assert!(
            before_tables.contains("active_profile = \"test\""),
            "{raw_toml}"
        );
        assert!(before_tables.contains("filter = \"\""), "{raw_toml}");
        assert!(
            before_tables.contains("breaks_enabled = false"),
            "{raw_toml}"
        );
        assert!(!before_tables.contains("\nrules ="), "{raw_toml}");

        let draft_loaded = load(dir.path());
        assert_eq!(draft_loaded.active_profile, "test");
        assert_eq!(draft_loaded.filter, "");
        assert_eq!(draft_loaded.rules, preset.rules);
        assert!(!draft_loaded.breaks_enabled);

        // User clicks Save to commit the changes into `test`.
        update(dir.path(), |config| {
            config.saved_profiles[0].filter.clear();
            config.saved_profiles[0].breaks_enabled = false;
        })
        .unwrap();

        let saved_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let saved_top = saved_toml.split("[[").next().unwrap();
        assert!(
            saved_top.contains("active_profile = \"test\""),
            "{saved_toml}"
        );
        assert!(!saved_top.contains("filter ="), "{saved_toml}");
        assert!(!saved_top.contains("breaks_enabled ="), "{saved_toml}");
    }

    #[test]
    fn legacy_config_with_matching_saved_profile_migrates_to_active_profile() {
        let dir = Scratch::new();
        let legacy = r#"listen_port = 8877
listen_all = true
use_system_proxy = false
language = "zh"
bypass_hosts = ""
rules = "replace */finance-api/online-hall-v2/configs req supportCrypto=true -> supportCrypto=false"
upstream_proxy = ""
window_width = 1845
window_height = 1018
filter = "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall|/super|/gfhall"
capturing = true

[[saved_profiles]]
name = "test"
filter = "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall|/super|/gfhall"
rules = "replace */finance-api/online-hall-v2/configs req supportCrypto=true -> supportCrypto=false"

[[saved_profiles]]
name = "google"
bypass_hosts = "google.com"

[column_widths]
method = 73
path = 280
"#;
        fs::write(dir.path().join("config.toml"), legacy).unwrap();

        let config = load(dir.path());
        assert_eq!(config.active_profile, "test");
        assert_eq!(
            config.filter,
            "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall|/super|/gfhall"
        );
        assert_eq!(
            config.rules,
            "replace */finance-api/online-hall-v2/configs req supportCrypto=true -> supportCrypto=false"
        );

        let migrated_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let top = migrated_toml.split("[[").next().unwrap();
        assert!(top.contains("active_profile = \"test\""), "{migrated_toml}");
        assert!(!top.contains("\nrules ="), "{migrated_toml}");
        assert!(!top.contains("\nfilter ="), "{migrated_toml}");
        assert!(!top.contains("bypass_hosts"), "{migrated_toml}");
        assert!(!top.contains("upstream_proxy"), "{migrated_toml}");
    }

    #[test]
    fn deleting_active_profile_clears_reference_and_keeps_live_setup() {
        let dir = Scratch::new();
        let preset = SavedProfile {
            name: "test".into(),
            filter: "host:a.test".into(),
            bypass_hosts: String::new(),
            rules: "break a.test /api".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        };
        update(dir.path(), |config| {
            config.saved_profiles = vec![preset.clone()];
            config.active_profile = "test".into();
            config.filter = preset.filter.clone();
            config.rules = preset.rules.clone();
        })
        .unwrap();

        update(dir.path(), |config| {
            config.saved_profiles.clear();
        })
        .unwrap();

        let raw_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!raw_toml.contains("active_profile"), "{raw_toml}");
        assert!(raw_toml.contains("filter = \"host:a.test\""), "{raw_toml}");
        assert!(
            raw_toml.contains("rules = \"break a.test /api\""),
            "{raw_toml}"
        );

        let reloaded = load(dir.path());
        assert!(reloaded.active_profile.is_empty());
        assert_eq!(reloaded.filter, "host:a.test");
        assert_eq!(reloaded.rules, "break a.test /api");
    }

    #[test]
    fn clamp_saved_profiles_trims_fields_and_deduplicates_default_name() {
        let dir = Scratch::new();
        update(dir.path(), |config| {
            config.saved_profiles = vec![
                SavedProfile {
                    name: "   ".into(),
                    filter: "  host:first.test  ".into(),
                    bypass_hosts: "  a.internal  ".into(),
                    rules: "  cors first.test  ".into(),
                    upstream_proxy: "  127.0.0.1:7890  ".into(),
                    rules_enabled: true,
                    breaks_enabled: true,
                    show_tunnel_failures: false,
                },
                SavedProfile {
                    name: String::new(),
                    filter: "host:duplicate-empty-name.test".into(),
                    bypass_hosts: String::new(),
                    rules: String::new(),
                    upstream_proxy: String::new(),
                    rules_enabled: true,
                    breaks_enabled: true,
                    show_tunnel_failures: false,
                },
                SavedProfile {
                    name: "配置".into(),
                    filter: "host:explicit-default-name.test".into(),
                    bypass_hosts: String::new(),
                    rules: String::new(),
                    upstream_proxy: String::new(),
                    rules_enabled: true,
                    breaks_enabled: true,
                    show_tunnel_failures: false,
                },
            ];
        })
        .unwrap();

        let reloaded = load(dir.path());
        assert_eq!(reloaded.saved_profiles.len(), 1);
        let p = &reloaded.saved_profiles[0];
        assert_eq!(p.name, "配置");
        assert_eq!(p.filter, "host:first.test");
        assert_eq!(p.bypass_hosts, "a.internal");
        assert_eq!(p.rules, "cors first.test");
        assert_eq!(p.upstream_proxy, "127.0.0.1:7890");
    }

    #[test]
    fn redact_strict_persists_when_true_and_omits_default_false() {
        let dir = Scratch::new();
        assert!(!load(dir.path()).redact_strict);

        update(dir.path(), |config| {
            config.redact_export = true;
            config.redact_strict = true;
            config.redact_fields = "phone, id_card".into();
        })
        .unwrap();

        let raw_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(raw_toml.contains("redact_strict = true"), "{raw_toml}");

        let reloaded = load(dir.path());
        assert!(reloaded.redact_export);
        assert!(reloaded.redact_strict);
        assert_eq!(reloaded.redact_fields, "phone, id_card");

        update(dir.path(), |config| {
            config.redact_strict = false;
        })
        .unwrap();
        let raw_toml = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!raw_toml.contains("redact_strict"), "{raw_toml}");
        assert!(!load(dir.path()).redact_strict);
    }
}
