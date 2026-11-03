//! User settings in one `config.toml`. The system-proxy snapshot stays in its
//! own file: a failed write while restoring the proxy must not take the port
//! and rules with it.

use std::fs;
use std::io::{ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::{parse_pref, Lang};
use crate::present::DEFAULT_PORT;

const FILE_NAME: &str = "config.toml";
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
        }
    }
}

fn default_port() -> u16 {
    DEFAULT_PORT
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
}
