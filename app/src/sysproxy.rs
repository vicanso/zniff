//! System proxy: remember the previous settings, point them at zniff when
//! capture asks, and put them back on stop, panic, or the next launch.
//! The window decides whether to ask. Launch itself does not call `enable`.

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::ca::data_dir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub active: bool,
    #[serde(default)]
    pub macos: Vec<MacService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<WindowsProxy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gnome: Option<GnomeProxy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacService {
    pub name: String,
    pub web: Endpoint,
    pub secure: Endpoint,
    pub socks: Endpoint,
    pub pac_enabled: bool,
    pub pac_url: String,
    pub bypass: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsProxy {
    pub enable: u32,
    pub server: String,
    pub override_list: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GnomeProxy {
    pub mode: String,
    pub http_host: String,
    pub http_port: i32,
    pub https_host: String,
    pub https_port: i32,
    pub ignore_hosts: String,
}

pub enum EnableOutcome {
    Applied,
    /// The listener is still useful; the user points a browser at it.
    #[allow(dead_code)]
    Manual {
        reason: String,
    },
}

fn snapshot_path(dir: &Path, dev: bool) -> PathBuf {
    dir.join(if dev {
        "proxy-snapshot.dev.toml"
    } else {
        "proxy-snapshot.toml"
    })
}

pub fn restore_if_dirty() -> Result<bool> {
    let dev = crate::config::is_dev_mode();
    let Some(snapshot) = read_snapshot_in_mode(&data_dir(), dev)? else {
        return Ok(false);
    };
    if !snapshot.active {
        return Ok(false);
    }
    apply_snapshot(&snapshot)?;
    let mut cleared = snapshot;
    cleared.active = false;
    write_snapshot(&cleared)?;
    Ok(true)
}

pub fn enable(port: u16) -> Result<EnableOutcome> {
    let dev = crate::config::is_dev_mode();
    if let Some(existing) = read_snapshot_in_mode(&data_dir(), dev)?
        && existing.active
    {
        apply_snoop(port, &existing)?;
        return Ok(EnableOutcome::Applied);
    }
    let mut snapshot = capture_current()?;
    snapshot.active = true;
    write_snapshot(&snapshot)?;
    match apply_snoop(port, &snapshot) {
        Ok(()) => Ok(EnableOutcome::Applied),
        Err(err) => {
            let _ = apply_snapshot(&snapshot);
            let mut cleared = snapshot;
            cleared.active = false;
            let _ = write_snapshot(&cleared);
            Err(err)
        }
    }
}

pub fn disable() -> Result<()> {
    restore_if_dirty().map(|_| ())
}

fn capture_current() -> Result<Snapshot> {
    Ok(Snapshot {
        active: false,
        macos: capture_macos()?,
        windows: capture_windows()?,
        gnome: capture_gnome()?,
    })
}

fn apply_snapshot(snapshot: &Snapshot) -> Result<()> {
    #[cfg(target_os = "macos")]
    restore_macos(&snapshot.macos)?;
    #[cfg(target_os = "windows")]
    if let Some(windows) = &snapshot.windows {
        write_windows(windows)?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(gnome) = &snapshot.gnome {
        write_gnome(gnome)?;
    }
    let _ = snapshot;
    Ok(())
}

fn apply_snoop(port: u16, snapshot: &Snapshot) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if snapshot.macos.is_empty() {
            anyhow::bail!("没有找到已启用的网络服务");
        }
        enable_macos(port, &snapshot.macos)?;
    }
    #[cfg(target_os = "windows")]
    {
        write_windows(&WindowsProxy {
            enable: 1,
            server: format!("127.0.0.1:{port}"),
            override_list: String::new(),
        })?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if snapshot.gnome.is_none() {
            anyhow::bail!("没有 GNOME 系统代理，请把浏览器代理设为 127.0.0.1:{port}");
        }
        write_gnome(&GnomeProxy {
            mode: "manual".into(),
            http_host: "127.0.0.1".into(),
            http_port: port as i32,
            https_host: "127.0.0.1".into(),
            https_port: port as i32,
            ignore_hosts: "@as []".into(),
        })?;
    }
    let _ = (port, snapshot);
    Ok(())
}

#[cfg(test)]
fn read_snapshot_in(dir: &Path) -> Result<Option<Snapshot>> {
    read_snapshot_in_mode(dir, false)
}

fn read_snapshot_in_mode(dir: &Path, dev: bool) -> Result<Option<Snapshot>> {
    let toml_path = snapshot_path(dir, dev);
    let json_path = dir.join("proxy-snapshot.json");
    if toml_path.is_file() {
        let text = fs::read_to_string(&toml_path)
            .with_context(|| format!("读取 {}", toml_path.display()))?;
        let snapshot: Snapshot =
            toml::from_str(&text).with_context(|| format!("解析 {}", toml_path.display()))?;
        if !dev && json_path.is_file() {
            let _ = fs::remove_file(&json_path);
        }
        return Ok(Some(snapshot));
    }
    if dev || !json_path.is_file() {
        return Ok(None);
    }
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(&json_path)?)
        .with_context(|| format!("解析 {}", json_path.display()))?;
    write_snapshot_in_mode(dir, false, &snapshot)?;
    let _ = fs::remove_file(&json_path);
    Ok(Some(snapshot))
}

fn write_snapshot(snapshot: &Snapshot) -> Result<()> {
    write_snapshot_in_mode(&data_dir(), crate::config::is_dev_mode(), snapshot)
}

#[cfg(test)]
fn write_snapshot_in(dir: &Path, snapshot: &Snapshot) -> Result<()> {
    write_snapshot_in_mode(dir, false, snapshot)
}

fn write_snapshot_in_mode(dir: &Path, dev: bool, snapshot: &Snapshot) -> Result<()> {
    let text = toml::to_string_pretty(snapshot).context("编码系统代理快照")?;
    let path = snapshot_path(dir, dev);
    write_atomic(&path, &text).with_context(|| format!("写入 {}", path.display()))?;
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    fs::write(&tmp, text)?;
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn capture_macos() -> Result<Vec<MacService>> {
    let mut services = Vec::new();
    for name in enabled_services()? {
        services.push(MacService {
            web: read_endpoint(&name, "-getwebproxy")?,
            secure: read_endpoint(&name, "-getsecurewebproxy")?,
            socks: read_endpoint(&name, "-getsocksfirewallproxy")?,
            pac_enabled: read_flag(&name, "-getautoproxyurl", "Enabled")?,
            pac_url: read_field(&name, "-getautoproxyurl", "URL").unwrap_or_default(),
            bypass: read_bypass(&name)?,
            name,
        });
    }
    Ok(services)
}

#[cfg(not(target_os = "macos"))]
fn capture_macos() -> Result<Vec<MacService>> {
    Ok(Vec::new())
}

#[cfg(target_os = "macos")]
fn enable_macos(port: u16, services: &[MacService]) -> Result<()> {
    for service in services {
        run_setup(&["-setautoproxystate", &service.name, "off"])?;
        run_setup(&["-setsocksfirewallproxystate", &service.name, "off"])?;
        run_setup(&[
            "-setwebproxy",
            &service.name,
            "127.0.0.1",
            &port.to_string(),
            "off",
            "",
            "",
        ])?;
        run_setup(&[
            "-setsecurewebproxy",
            &service.name,
            "127.0.0.1",
            &port.to_string(),
            "off",
            "",
            "",
        ])?;
        run_setup(&["-setwebproxystate", &service.name, "on"])?;
        run_setup(&["-setsecurewebproxystate", &service.name, "on"])?;
        run_setup(&["-setproxybypassdomains", &service.name, "Empty"])?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_macos(services: &[MacService]) -> Result<()> {
    for service in services {
        write_endpoint(
            &service.name,
            "-setwebproxy",
            "-setwebproxystate",
            &service.web,
        )?;
        write_endpoint(
            &service.name,
            "-setsecurewebproxy",
            "-setsecurewebproxystate",
            &service.secure,
        )?;
        write_endpoint(
            &service.name,
            "-setsocksfirewallproxy",
            "-setsocksfirewallproxystate",
            &service.socks,
        )?;
        if service.pac_url.is_empty() {
            run_setup(&["-setautoproxystate", &service.name, "off"])?;
        } else {
            run_setup(&["-setautoproxyurl", &service.name, &service.pac_url])?;
            run_setup(&[
                "-setautoproxystate",
                &service.name,
                if service.pac_enabled { "on" } else { "off" },
            ])?;
        }
        let mut args = vec!["-setproxybypassdomains".to_string(), service.name.clone()];
        if service.bypass.is_empty() {
            args.push("Empty".into());
        } else {
            args.extend(service.bypass.iter().cloned());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_setup(&refs)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_endpoint(service: &str, set: &str, state: &str, endpoint: &Endpoint) -> Result<()> {
    let server = if endpoint.server.is_empty() {
        "0.0.0.0"
    } else {
        &endpoint.server
    };
    let port = if endpoint.port == 0 {
        "0".to_string()
    } else {
        endpoint.port.to_string()
    };
    run_setup(&[set, service, server, &port, "off", "", ""])?;
    run_setup(&[state, service, if endpoint.enabled { "on" } else { "off" }])?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn enabled_services() -> Result<Vec<String>> {
    let text = setup_output(&["-listallnetworkservices"])?;
    let mut names = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        names.push(line.to_string());
    }
    Ok(names)
}

#[cfg(target_os = "macos")]
fn read_endpoint(service: &str, flag: &str) -> Result<Endpoint> {
    let text = setup_output(&[flag, service])?;
    Ok(parse_endpoint(&text))
}

#[cfg(target_os = "macos")]
fn read_bypass(service: &str) -> Result<Vec<String>> {
    let text = setup_output(&["-getproxybypassdomains", service])?;
    Ok(parse_bypass(&text))
}

#[cfg(target_os = "macos")]
fn read_field(service: &str, flag: &str, key: &str) -> Result<String> {
    let text = setup_output(&[flag, service])?;
    Ok(parse_field(&text, key).unwrap_or_default())
}

#[cfg(target_os = "macos")]
fn read_flag(service: &str, flag: &str, key: &str) -> Result<bool> {
    let text = setup_output(&[flag, service])?;
    Ok(parse_yes(parse_field(&text, key).unwrap_or_default()))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_endpoint(text: &str) -> Endpoint {
    Endpoint {
        enabled: parse_yes(parse_field(text, "Enabled").unwrap_or_default()),
        server: parse_field(text, "Server").unwrap_or_default(),
        port: parse_field(text, "Port")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_bypass(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("aren't any") || lower.contains("there are no") {
        return Vec::new();
    }
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_field(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(&prefix)
            .map(|value| value.trim().to_string())
    })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_yes(value: impl AsRef<str>) -> bool {
    matches!(
        value.as_ref().trim().to_ascii_lowercase().as_str(),
        "yes" | "on" | "1" | "true"
    )
}

#[cfg(target_os = "macos")]
fn setup_output(args: &[&str]) -> Result<String> {
    let output = Command::new("networksetup")
        .args(args)
        .output()
        .context("运行 networksetup")?;
    if !output.status.success() {
        anyhow::bail!(
            "networksetup {} 失败: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn run_setup(args: &[&str]) -> Result<()> {
    let _ = setup_output(args)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture_windows() -> Result<Option<WindowsProxy>> {
    use winreg::RegKey;
    use winreg::enums::*;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key =
        hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")?;
    Ok(Some(WindowsProxy {
        enable: key.get_value("ProxyEnable").unwrap_or(0),
        server: key.get_value("ProxyServer").unwrap_or_default(),
        override_list: key.get_value("ProxyOverride").unwrap_or_default(),
    }))
}

#[cfg(not(target_os = "windows"))]
fn capture_windows() -> Result<Option<WindowsProxy>> {
    Ok(None)
}

#[cfg(target_os = "windows")]
fn write_windows(proxy: &WindowsProxy) -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::*;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
        KEY_SET_VALUE,
    )?;
    key.set_value("ProxyEnable", &proxy.enable)?;
    key.set_value("ProxyServer", &proxy.server)?;
    key.set_value("ProxyOverride", &proxy.override_list)?;
    refresh_wininet();
    Ok(())
}

#[cfg(target_os = "windows")]
fn refresh_wininet() {
    use std::ptr::null_mut;
    use windows_sys::Win32::Networking::WinInet::{
        INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
    };
    unsafe {
        InternetSetOptionW(null_mut(), INTERNET_OPTION_SETTINGS_CHANGED, null_mut(), 0);
        InternetSetOptionW(null_mut(), INTERNET_OPTION_REFRESH, null_mut(), 0);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn capture_gnome() -> Result<Option<GnomeProxy>> {
    if Command::new("gsettings").arg("--version").output().is_err() {
        return Ok(None);
    }
    let mode = gsettings_get("org.gnome.system.proxy", "mode").unwrap_or_else(|_| "none".into());
    Ok(Some(GnomeProxy {
        mode,
        http_host: gsettings_get("org.gnome.system.proxy.http", "host").unwrap_or_default(),
        http_port: gsettings_get("org.gnome.system.proxy.http", "port")
            .unwrap_or_else(|_| "0".into())
            .parse()
            .unwrap_or(0),
        https_host: gsettings_get("org.gnome.system.proxy.https", "host").unwrap_or_default(),
        https_port: gsettings_get("org.gnome.system.proxy.https", "port")
            .unwrap_or_else(|_| "0".into())
            .parse()
            .unwrap_or(0),
        ignore_hosts: gsettings_get("org.gnome.system.proxy", "ignore-hosts")
            .unwrap_or_else(|_| "@as []".into()),
    }))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn capture_gnome() -> Result<Option<GnomeProxy>> {
    Ok(None)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn write_gnome(proxy: &GnomeProxy) -> Result<()> {
    gsettings_set("org.gnome.system.proxy", "mode", &quote(&proxy.mode))?;
    gsettings_set(
        "org.gnome.system.proxy.http",
        "host",
        &quote(&proxy.http_host),
    )?;
    gsettings_set(
        "org.gnome.system.proxy.http",
        "port",
        &proxy.http_port.to_string(),
    )?;
    gsettings_set(
        "org.gnome.system.proxy.https",
        "host",
        &quote(&proxy.https_host),
    )?;
    gsettings_set(
        "org.gnome.system.proxy.https",
        "port",
        &proxy.https_port.to_string(),
    )?;
    gsettings_set(
        "org.gnome.system.proxy",
        "ignore-hosts",
        &proxy.ignore_hosts,
    )?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn gsettings_get(schema: &str, key: &str) -> Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .context("运行 gsettings")?;
    if !output.status.success() {
        anyhow::bail!("gsettings get {schema} {key} 失败");
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(unquote_gsettings(&text))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<()> {
    let status = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
        .context("运行 gsettings")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("gsettings set {schema} {key} 失败")
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unquote_gsettings(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
        text[1..text.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_networksetup_endpoint() {
        let text = "Enabled: Yes\nServer: 127.0.0.1\nPort: 12334\nAuthenticated Proxy Enabled: 0\n";
        let endpoint = parse_endpoint(text);
        assert!(endpoint.enabled);
        assert_eq!(endpoint.server, "127.0.0.1");
        assert_eq!(endpoint.port, 12334);
    }

    #[test]
    fn parses_bypass_domains() {
        let text = "localhost\n127.0.0.0/8\n::1\n";
        assert_eq!(parse_bypass(text), vec!["localhost", "127.0.0.0/8", "::1"]);
    }

    #[test]
    fn json_snapshot_moves_to_toml_without_touching_the_system() {
        let dir = std::env::temp_dir().join(format!(
            "zniff-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let json = r#"{
            "active": true,
            "macos": [{
                "name": "Wi-Fi",
                "web": {"enabled": false, "server": "127.0.0.1", "port": 12334},
                "secure": {"enabled": false, "server": "127.0.0.1", "port": 12334},
                "socks": {"enabled": false, "server": "0.0.0.0", "port": 0},
                "pac_enabled": false,
                "pac_url": "(null)",
                "bypass": ["*.local", "169.254/16"]
            }],
            "windows": null,
            "gnome": null
        }"#;
        fs::write(dir.join("proxy-snapshot.json"), json).unwrap();

        let loaded = read_snapshot_in(&dir).unwrap().expect("snapshot");
        assert!(loaded.active);
        assert_eq!(loaded.macos[0].name, "Wi-Fi");
        assert_eq!(loaded.macos[0].web.port, 12334);
        assert_eq!(loaded.macos[0].bypass, vec!["*.local", "169.254/16"]);
        assert!(loaded.windows.is_none());
        assert!(!dir.join("proxy-snapshot.json").exists());
        assert!(dir.join("proxy-snapshot.toml").is_file());

        let again = read_snapshot_in(&dir).unwrap().expect("toml");
        assert!(again.active);
        assert_eq!(again.macos[0].pac_url, "(null)");

        let mut idle = again;
        idle.active = false;
        write_snapshot_in(&dir, &idle).unwrap();
        assert!(!read_snapshot_in(&dir).unwrap().expect("idle").active);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_snapshot_stays_separate_from_prod_snapshot() {
        let dir = std::env::temp_dir().join(format!(
            "zniff-dev-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let prod = Snapshot {
            active: true,
            macos: Vec::new(),
            windows: None,
            gnome: None,
        };
        write_snapshot_in_mode(&dir, false, &prod).unwrap();
        assert!(read_snapshot_in_mode(&dir, true).unwrap().is_none());

        let dev = Snapshot {
            active: false,
            macos: Vec::new(),
            windows: None,
            gnome: None,
        };
        write_snapshot_in_mode(&dir, true, &dev).unwrap();
        assert!(dir.join("proxy-snapshot.dev.toml").is_file());
        assert!(read_snapshot_in_mode(&dir, false).unwrap().unwrap().active);
        assert!(!read_snapshot_in_mode(&dir, true).unwrap().unwrap().active);
        let _ = fs::remove_dir_all(&dir);
    }
}
