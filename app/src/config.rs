//! User settings in one `config.toml`. The system-proxy snapshot stays in its
//! own file: a failed write while restoring the proxy must not take the port
//! and rules with it.

use std::collections::HashMap;
use std::fs;
use std::io::{ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::{parse_pref, Lang};
use crate::present::DEFAULT_PORT;

const FILE_NAME: &str = "config.toml";

/// Left pane width when `config.toml` has no `list_width`.
pub const LIST_WIDTH_DEFAULT: u32 = 560;
pub const LIST_WIDTH_MIN: u32 = 280;
pub const LIST_WIDTH_MAX: u32 = 1100;

pub fn clamp_list_width(width: u32) -> u32 {
    width.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
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
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show_host: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_size: bool,
    #[serde(default = "default_list_width", skip_serializing_if = "is_default_list_width")]
    pub list_width: u32,
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
    /// Newest first. The filter menu offers these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_filters: Vec<String>,
    /// Dragged list-column widths, in points, keyed by `method`, `path`, and the rest.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub column_widths: HashMap<String, u32>,
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
            show_status: true,
            show_protocol: false,
            show_path: true,
            show_gap: true,
            show_elapsed: true,
            show_host: true,
            show_size: false,
            list_width: default_list_width(),
            default_pretty: true,
            show_tunnel_failures: false,
            filter: String::new(),
            capturing: false,
            recent_filters: Vec::new(),
            column_widths: HashMap::new(),
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
            default: 72,
            min: 56,
            max: 220,
        },
        "path" => ColumnWidth {
            default: 240,
            min: 80,
            max: 800,
        },
        "status" => ColumnWidth {
            default: 56,
            min: 40,
            max: 160,
        },
        "protocol" => ColumnWidth {
            default: 84,
            min: 52,
            max: 200,
        },
        "gap" => ColumnWidth {
            default: 72,
            min: 52,
            max: 200,
        },
        "elapsed" => ColumnWidth {
            default: 76,
            min: 52,
            max: 200,
        },
        "host" => ColumnWidth {
            default: 120,
            min: 64,
            max: 480,
        },
        "size" => ColumnWidth {
            default: 72,
            min: 48,
            max: 180,
        },
        _ => return None,
    })
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

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default_list_width(value: &u32) -> bool {
    *value == LIST_WIDTH_DEFAULT
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

enum Read {
    Ready(Config),
    Missing,
    Broken(std::io::Error),
}

/// Reads `config.toml`, or the old one-file-per-setting layout the first time.
/// A broken file stays on disk and this returns the defaults.
pub fn load(dir: &Path) -> Config {
    let _guard = lock();
    match read_config(dir) {
        Read::Ready(config) => config,
        Read::Broken(_) => Config::default(),
        Read::Missing => {
            let config = migrate_legacy(dir);
            if store_unlocked(dir, &config).is_ok() {
                remove_legacy(dir);
            }
            config
        }
    }
}

/// Changes one or more fields and writes the whole file back.
/// A broken `config.toml` is left untouched.
pub fn update(dir: &Path, change: impl FnOnce(&mut Config)) -> Result<()> {
    let _guard = lock();
    let mut config = match read_config(dir) {
        Read::Ready(config) => config,
        Read::Missing => migrate_legacy(dir),
        Read::Broken(err) => return Err(err),
    };
    change(&mut config);
    config = normalize(config);
    store_unlocked(dir, &config)?;
    remove_legacy(dir);
    Ok(())
}

fn read_config(dir: &Path) -> Read {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => Read::Ready(normalize(config)),
            Err(err) => Read::Broken(std::io::Error::new(ErrorKind::InvalidData, err)),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Read::Missing,
        Err(err) => Read::Broken(err),
    }
}

fn migrate_legacy(dir: &Path) -> Config {
    let mut config = Config::default();
    if let Some(text) = read_legacy(dir, "listen-port") {
        if let Some(port) = crate::present::parse_port(&text) {
            config.listen_port = port;
        }
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

fn normalize(mut config: Config) -> Config {
    if config.listen_port == 0 {
        config.listen_port = default_port();
    }
    config.language = language_name(config.language.as_deref().and_then(parse_pref));
    config.list_width = clamp_list_width(config.list_width);
    clamp_column_widths(&mut config.column_widths);
    config
}

fn read_legacy(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok()
}

fn remove_legacy(dir: &Path) {
    for name in LEGACY {
        let _ = fs::remove_file(dir.join(name));
    }
}

fn store_unlocked(dir: &Path, config: &Config) -> Result<()> {
    let text = toml::to_string_pretty(config)
        .map_err(|err| std::io::Error::new(ErrorKind::InvalidData, err))?;
    write_atomic(&dir.join(FILE_NAME), &text)
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
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
            let dir = std::env::temp_dir().join(format!(
                "zniff-config-{}-{}",
                std::process::id(),
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
        for (text, on) in [("", false), ("0\n", false), ("true", false), ("1", true), ("1\n", true)]
        {
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
        update(dir.path(), |config| config.language = Some("en".to_string())).unwrap();
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
        assert_eq!(fs::read_to_string(dir.path().join("config.toml")).unwrap(), broken);
    }

    #[test]
    fn list_layout_keeps_today_columns_and_clamps_width() {
        let dir = Scratch::new();
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        let config = load(dir.path());
        assert!(
            config.show_status
                && config.show_path
                && config.show_gap
                && config.show_elapsed
                && config.show_host
        );
        assert!(!config.show_protocol && !config.show_size);
        assert_eq!(config.list_width, LIST_WIDTH_DEFAULT);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("show_status"), "{text}");
        assert!(!text.contains("list_width"), "{text}");

        update(dir.path(), |config| {
            config.show_protocol = true;
            config.show_host = false;
            config.list_width = 9_999;
        })
        .unwrap();
        let config = load(dir.path());
        assert!(config.show_protocol);
        assert!(!config.show_host);
        assert!(config.show_status);
        assert_eq!(config.list_width, LIST_WIDTH_MAX);
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("show_protocol = true"), "{text}");
        assert!(text.contains("show_host = false"), "{text}");
        assert!(!text.contains("show_status"), "{text}");
        assert!(!text.contains("show_size"), "{text}");
        assert!(text.contains(&format!("list_width = {LIST_WIDTH_MAX}")), "{text}");

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
        fs::write(dir.path().join("config.toml"), "listen_port = 8877\n").unwrap();
        let config = load(dir.path());
        assert!(config.filter.is_empty());
        assert!(config.recent_filters.is_empty());
        update(dir.path(), |config| {
            config.filter = "api.test".into();
            config.recent_filters = vec!["api.test".into(), "pin:".into()];
        })
        .unwrap();
        let loaded = load(dir.path());
        assert_eq!(loaded.filter, "api.test");
        assert_eq!(loaded.recent_filters, vec!["api.test".to_string(), "pin:".to_string()]);
        update(dir.path(), |config| {
            config.filter.clear();
            config.recent_filters.clear();
        })
        .unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("filter"), "{text}");
        assert!(!text.contains("recent_filters"), "{text}");
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
}
