//! Zero-config target launchers for isolated Chromium browsers, proxied
//! terminal sessions, and Android `adb` devices.

use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::ca::Ca;
use crate::present;

static LAUNCH_SEQ: AtomicU64 = AtomicU64::new(1);
static BROWSER_PROFILES: Mutex<Vec<(Child, PathBuf)>> = Mutex::new(Vec::new());
static ADB_PROXIED: Mutex<Vec<(String, u16)>> = Mutex::new(Vec::new());
static LAST_ADB_BIN: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(windows)]
const ADB_BIN_NAME: &str = "adb.exe";
#[cfg(not(windows))]
const ADB_BIN_NAME: &str = "adb";

/// Outcome reported after configuring connected Android devices via `adb`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdbOutcome {
    Configured {
        count: usize,
        port: u16,
        cert_pushed: bool,
    },
    NoneConnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserCandidate {
    pub(crate) name: &'static str,
    pub(crate) bin: PathBuf,
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && (meta.permissions().mode() & 0o111) != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find_in_path_var(path_var: &OsStr, names: &[&str]) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        for name in names {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    find_in_path_var(&path_var, names)
}

#[cfg(target_os = "macos")]
fn detect_chromium_browser() -> Option<BrowserCandidate> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let specs: [(&str, &str); 5] = [
        (
            "Google Chrome",
            "Google Chrome.app/Contents/MacOS/Google Chrome",
        ),
        (
            "Microsoft Edge",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ),
        (
            "Brave Browser",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
        ),
        ("Chromium", "Chromium.app/Contents/MacOS/Chromium"),
        (
            "Google Chrome Canary",
            "Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        ),
    ];
    for (name, rel) in specs {
        let sys_path = Path::new("/Applications").join(rel);
        if sys_path.is_file() {
            return Some(BrowserCandidate {
                name,
                bin: sys_path,
            });
        }
        if let Some(ref home_dir) = home {
            let user_path = home_dir.join("Applications").join(rel);
            if user_path.is_file() {
                return Some(BrowserCandidate {
                    name,
                    bin: user_path,
                });
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_chromium_browser() -> Option<BrowserCandidate> {
    let mut roots = Vec::new();
    for var in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(val) = std::env::var_os(var) {
            roots.push(PathBuf::from(val));
        }
    }
    let specs: [(&str, &str); 3] = [
        ("Google Chrome", r"Google\Chrome\Application\chrome.exe"),
        ("Microsoft Edge", r"Microsoft\Edge\Application\msedge.exe"),
        (
            "Brave Browser",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
        ),
    ];
    for (name, rel) in specs {
        for root in &roots {
            let path = root.join(rel);
            if path.is_file() {
                return Some(BrowserCandidate { name, bin: path });
            }
        }
    }
    find_in_path(&["chrome.exe", "msedge.exe", "brave.exe"]).map(|bin| BrowserCandidate {
        name: "Chromium",
        bin,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn detect_chromium_browser() -> Option<BrowserCandidate> {
    let specs: [(&str, &[&str]); 4] = [
        ("Google Chrome", &["google-chrome", "google-chrome-stable"]),
        ("Chromium", &["chromium", "chromium-browser"]),
        (
            "Microsoft Edge",
            &["microsoft-edge", "microsoft-edge-stable"],
        ),
        ("Brave Browser", &["brave-browser", "brave"]),
    ];
    for (name, bins) in specs {
        if let Some(bin) = find_in_path(bins) {
            return Some(BrowserCandidate { name, bin });
        }
    }
    None
}

/// Builds Chromium CLI flags for zero-config isolated HTTPS interception,
/// optionally opening a specific HTTP(S) initial URL.
pub fn build_chromium_args(
    port: u16,
    spki_hashes: &[String],
    profile_dir: &Path,
    initial_url: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        format!("--proxy-server=http://127.0.0.1:{port}"),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    if !spki_hashes.is_empty() {
        args.push(format!(
            "--ignore-certificate-errors-spki-list={}",
            spki_hashes.join(",")
        ));
    }
    let start_url = initial_url
        .map(str::trim)
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .map(str::to_string)
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}/zniff"));
    args.push(start_url);
    args
}

/// Removes temporary browser profile directories whose child process has exited.
fn sweep_exited_browser_profiles() {
    let mut guard = BROWSER_PROFILES
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let mut active = Vec::with_capacity(guard.len());
    for (mut child, dir) in guard.drain(..) {
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = fs::remove_dir_all(&dir);
            }
            Ok(None) | Err(_) => active.push((child, dir)),
        }
    }
    *guard = active;
}

/// Launches an isolated Chromium-based browser pointed at the running proxy
/// without modifying the OS proxy or system trust store.
pub fn launch_isolated_browser(ca: &Ca, port: u16, initial_url: Option<&str>) -> Result<String> {
    sweep_exited_browser_profiles();
    let candidate = detect_chromium_browser()
        .context("未找到 Chrome / Edge / Brave / Chromium，请先安装或手动配置代理")?;
    let seq = LAUNCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let profile_dir =
        std::env::temp_dir().join(format!("zniff-chrome-{}-{seq}", std::process::id()));
    fs::create_dir_all(&profile_dir)
        .with_context(|| format!("创建临时浏览器配置目录 {}", profile_dir.display()))?;
    let spki = ca.spki_hashes_base64();
    let args = build_chromium_args(port, &spki, &profile_dir, initial_url);
    let child = Command::new(&candidate.bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("启动 {} ({})", candidate.name, candidate.bin.display()))?;
    BROWSER_PROFILES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .push((child, profile_dir));
    Ok(candidate.name.to_string())
}

#[cfg(any(target_os = "macos", test))]
fn shell_quote_single(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Builds a self-deleting shell bootstrap script for macOS terminal windows.
#[cfg(any(target_os = "macos", test))]
pub fn build_macos_terminal_script(port: u16, cert_path: &Path, proxy_auth: &str) -> String {
    let cert_str = cert_path.to_string_lossy();
    let exports = present::proxy_env_export_with_auth(port, &cert_str, proxy_auth);
    let quoted_cert = shell_quote_single(&cert_str);
    format!(
        "#!/bin/sh\n{exports}\nprintf '\\n[zniff] Proxy active -> http://127.0.0.1:{port}\\n'\nprintf '[zniff] Root CA      -> %s\\n\\n' {quoted_cert}\nrm -f -- \"$0\"\nexec \"${{SHELL:-/bin/zsh}}\" -i\n"
    )
}

#[cfg(target_os = "macos")]
fn launch_terminal_platform(port: u16, cert_path: &Path, proxy_auth: &str) -> Result<String> {
    let seq = LAUNCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let script_path =
        std::env::temp_dir().join(format!("zniff-term-{}-{seq}.command", std::process::id()));
    let body = build_macos_terminal_script(port, cert_path, proxy_auth);
    fs::write(&script_path, body.as_bytes())
        .with_context(|| format!("写入终端启动脚本 {}", script_path.display()))?;
    fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("设置脚本可执行权限 {}", script_path.display()))?;

    let preferred = ["iTerm", "Ghostty", "Warp", "Terminal"];
    let mut launched_name = None;
    for app in preferred {
        let app_dir = Path::new("/Applications").join(format!("{app}.app"));
        let sys_app_dir = Path::new("/System/Applications/Utilities").join(format!("{app}.app"));
        if app != "Terminal" && !app_dir.exists() && !sys_app_dir.exists() {
            continue;
        }
        let status = Command::new("open")
            .args(["-a", app])
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.is_ok_and(|s| s.success()) {
            launched_name = Some(app.to_string());
            break;
        }
    }
    if launched_name.is_none() {
        let out = Command::new("open")
            .arg(&script_path)
            .output()
            .context("打开系统终端失败")?;
        if !out.status.success() {
            let _ = fs::remove_file(&script_path);
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            anyhow::bail!(
                "{}",
                if err.is_empty() {
                    "open terminal failed"
                } else {
                    &err
                }
            );
        }
        launched_name = Some("Terminal".to_string());
    }
    Ok(launched_name.unwrap_or_else(|| "Terminal".to_string()))
}

#[cfg(target_os = "windows")]
fn launch_terminal_platform(port: u16, cert_path: &Path, proxy_auth: &str) -> Result<String> {
    let vars = present::proxy_env_vars(port, &cert_path.to_string_lossy(), proxy_auth);
    let banner = format!("Write-Host '[zniff] Proxy active -> http://127.0.0.1:{port}'");
    if let Some(wt) = find_in_path(&["wt.exe"]) {
        let mut cmd = Command::new(wt);
        cmd.args(["powershell.exe", "-NoExit", "-Command", &banner]);
        for (k, v) in &vars {
            cmd.env(k, v);
        }
        if cmd.spawn().is_ok() {
            return Ok("Windows Terminal".to_string());
        }
    }
    let mut cmd = Command::new("cmd.exe");
    cmd.args([
        "/c",
        "start",
        "zniff proxy",
        "powershell.exe",
        "-NoExit",
        "-Command",
        &banner,
    ]);
    for (k, v) in &vars {
        cmd.env(k, v);
    }
    cmd.spawn().context("启动 PowerShell 终端失败")?;
    Ok("PowerShell".to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch_terminal_platform(port: u16, cert_path: &Path, proxy_auth: &str) -> Result<String> {
    let vars = present::proxy_env_vars(port, &cert_path.to_string_lossy(), proxy_auth);
    let candidates = [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "kitty",
        "alacritty",
    ];
    for term in candidates {
        let Some(bin) = find_in_path(&[term]) else {
            continue;
        };
        let mut cmd = Command::new(&bin);
        for (k, v) in &vars {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if cmd.spawn().is_ok() {
            return Ok(term.to_string());
        }
    }
    anyhow::bail!(
        "未找到受支持的终端程序（x-terminal-emulator/gnome-terminal/konsole/kitty/alacritty）"
    )
}

/// Launches a desktop terminal pre-loaded with zniff proxy environment variables and CA paths.
pub fn launch_proxied_terminal(ca: &Ca, port: u16, proxy_auth: &str) -> Result<String> {
    let cert_path = ca.cert_path();
    if !cert_path.is_file() {
        anyhow::bail!("根证书文件不存在: {}", cert_path.display());
    }
    launch_terminal_platform(port, &cert_path, proxy_auth)
}

/// Parses online device serials from `adb devices` stdout.
pub fn parse_adb_devices(output: &str) -> Vec<String> {
    let mut serials = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("List of devices") || trimmed.starts_with('*')
        {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        if state == "device" && !serials.iter().any(|existing| existing == serial) {
            serials.push(serial.to_string());
        }
    }
    serials
}

pub(crate) fn adb_reverse_args(serial: &str, port: u16) -> Vec<String> {
    let tcp = format!("tcp:{port}");
    vec![
        "-s".into(),
        serial.into(),
        "reverse".into(),
        tcp.clone(),
        tcp,
    ]
}

pub(crate) fn adb_set_proxy_args(serial: &str, port: u16) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "shell".into(),
        "settings".into(),
        "put".into(),
        "global".into(),
        "http_proxy".into(),
        format!("127.0.0.1:{port}"),
    ]
}

pub(crate) fn adb_clear_proxy_args(serial: &str) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "shell".into(),
        "settings".into(),
        "put".into(),
        "global".into(),
        "http_proxy".into(),
        ":0".into(),
    ]
}

pub(crate) fn adb_reverse_remove_args(serial: &str, port: u16) -> Vec<String> {
    vec![
        "-s".into(),
        serial.into(),
        "reverse".into(),
        "--remove".into(),
        format!("tcp:{port}"),
    ]
}

fn env_non_empty_path(key: &str) -> Option<PathBuf> {
    let val = std::env::var_os(key)?;
    if val.is_empty() || val.to_str().is_some_and(|s| s.trim().is_empty()) {
        return None;
    }
    Some(PathBuf::from(val))
}

#[cfg(target_os = "macos")]
fn collect_macos_etc_path_files() -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from("/etc/paths")];
    if let Ok(entries) = fs::read_dir("/etc/paths.d") {
        let mut extra: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        extra.sort();
        files.extend(extra);
    }
    files
}

fn default_extra_path_files() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        collect_macos_etc_path_files()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

fn read_path_dirs_from_files(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for file in files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        for raw in content.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            dirs.push(PathBuf::from(trimmed));
        }
    }
    dirs
}

struct AdbLookupEnv {
    override_bin: Option<PathBuf>,
    path_var: Option<OsString>,
    android_home: Option<PathBuf>,
    android_sdk_root: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    #[cfg(target_os = "windows")]
    local_app_data: Option<PathBuf>,
    extra_path_files: Vec<PathBuf>,
}

impl AdbLookupEnv {
    fn from_process_env() -> Self {
        Self {
            override_bin: env_non_empty_path("ZNIFF_ADB"),
            path_var: std::env::var_os("PATH").filter(|val| !val.is_empty()),
            android_home: env_non_empty_path("ANDROID_HOME"),
            android_sdk_root: env_non_empty_path("ANDROID_SDK_ROOT"),
            home_dir: env_non_empty_path("HOME").or_else(|| env_non_empty_path("USERPROFILE")),
            #[cfg(target_os = "windows")]
            local_app_data: env_non_empty_path("LOCALAPPDATA"),
            extra_path_files: default_extra_path_files(),
        }
    }
}

fn join_rel(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |acc, part| acc.join(part))
}

fn well_known_adb_candidates(env: &AdbLookupEnv) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        if let Some(home) = &env.home_dir {
            dirs.push(join_rel(
                home,
                &["Library", "Android", "sdk", "platform-tools"],
            ));
        }
        dirs.push(PathBuf::from("/opt/local/bin"));
        if let Some(home) = &env.home_dir {
            dirs.push(join_rel(home, &[".local", "bin"]));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = &env.home_dir {
            dirs.push(join_rel(home, &["Android", "Sdk", "platform-tools"]));
        }
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
        if let Some(home) = &env.home_dir {
            dirs.push(join_rel(home, &[".local", "bin"]));
        }
        dirs.push(PathBuf::from("/snap/bin"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = &env.local_app_data {
            dirs.push(join_rel(
                local_app_data,
                &["Android", "Sdk", "platform-tools"],
            ));
        }
        if let Some(home) = &env.home_dir {
            dirs.push(join_rel(
                home,
                &["AppData", "Local", "Android", "Sdk", "platform-tools"],
            ));
            dirs.push(join_rel(home, &["scoop", "shims"]));
        }
        dirs.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
    }

    dirs.extend(read_path_dirs_from_files(&env.extra_path_files));
    dirs.into_iter().map(|dir| dir.join(ADB_BIN_NAME)).collect()
}

fn resolve_adb_from_env(env: &AdbLookupEnv) -> Option<PathBuf> {
    if let Some(override_bin) = &env.override_bin {
        if is_executable_file(override_bin) {
            return Some(override_bin.clone());
        }
        if override_bin.is_dir() {
            let joined = override_bin.join(ADB_BIN_NAME);
            if is_executable_file(&joined) {
                return Some(joined);
            }
        }
    }
    if let Some(found) = env
        .path_var
        .as_deref()
        .and_then(|path_var| find_in_path_var(path_var, &[ADB_BIN_NAME]))
    {
        return Some(found);
    }
    for root in [&env.android_home, &env.android_sdk_root]
        .into_iter()
        .flatten()
    {
        let candidate = root.join("platform-tools").join(ADB_BIN_NAME);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    well_known_adb_candidates(env)
        .into_iter()
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn parse_login_shell_path(stdout: &str) -> Option<PathBuf> {
    stdout.lines().rev().find_map(|raw| {
        let trimmed = raw.trim();
        let path = Path::new(trimmed);
        (path.is_absolute() && is_executable_file(path)).then(|| path.to_path_buf())
    })
}

#[cfg(unix)]
fn probe_adb_in_shell(shell: &Path, timeout: Duration) -> Option<PathBuf> {
    let mut child = Command::new(shell)
        .args(["-lc", "command -v adb"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout {
            let _ = out.take(64 * 1024).read_to_string(&mut text);
        }
        text
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    drop(reader);
                    return None;
                }
                let text = reader.join().unwrap_or_default();
                return parse_login_shell_path(&text);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(reader);
                return None;
            }
        }
    }
}

#[cfg(unix)]
fn find_adb_via_login_shell() -> Option<PathBuf> {
    let mut shells: Vec<PathBuf> = Vec::new();
    if let Some(shell) = env_non_empty_path("SHELL") {
        shells.push(shell);
    }
    for fallback in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        let path = PathBuf::from(fallback);
        if !shells.iter().any(|existing| existing == &path) {
            shells.push(path);
        }
    }
    shells
        .into_iter()
        .filter(|shell| is_executable_file(shell))
        .find_map(|shell| probe_adb_in_shell(&shell, Duration::from_secs(2)))
}

/// Resolves the `adb` binary across overrides, PATH, SDK roots, common install dirs,
/// and (on Unix GUI launches) a login-shell fallback.
pub(crate) fn resolve_adb_bin() -> Option<PathBuf> {
    if let Some(found) = resolve_adb_from_env(&AdbLookupEnv::from_process_env()) {
        return Some(found);
    }
    #[cfg(unix)]
    {
        find_adb_via_login_shell()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn cached_or_resolve_adb_bin() -> Option<PathBuf> {
    {
        let guard = LAST_ADB_BIN.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(cached) = guard.as_ref().filter(|path| is_executable_file(path)) {
            return Some(cached.clone());
        }
    }
    let resolved = resolve_adb_bin()?;
    *LAST_ADB_BIN.lock().unwrap_or_else(|err| err.into_inner()) = Some(resolved.clone());
    Some(resolved)
}

fn run_adb(adb: &Path, args: &[String], action: &str) -> Result<()> {
    let output = Command::new(adb)
        .args(args)
        .output()
        .with_context(|| format!("{action}: 执行 {} 失败", adb.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    if detail.is_empty() {
        anyhow::bail!("{action} 失败（退出码 {}）", output.status);
    }
    anyhow::bail!("{action}: {detail}");
}

fn remember_adb_proxies(items: &[(String, u16)]) {
    if items.is_empty() {
        return;
    }
    let mut kept = ADB_PROXIED.lock().unwrap_or_else(|err| err.into_inner());
    for (serial, port) in items {
        if let Some(existing) = kept.iter_mut().find(|(s, _)| s == serial) {
            existing.1 = *port;
        } else {
            kept.push((serial.clone(), *port));
        }
    }
}

/// Configures `adb reverse` and `global http_proxy` on all online Android devices,
/// and pushes `ca.crt` to `/sdcard/Download/zniff-ca.crt` best-effort.
pub fn configure_android_adb(ca: &Ca, port: u16) -> Result<AdbOutcome> {
    if port == 0 {
        anyhow::bail!("请先启动抓包监听端口");
    }
    let adb = resolve_adb_bin().ok_or_else(|| {
        anyhow::anyhow!(
            "未找到 adb 命令（已检查 PATH、ANDROID_HOME/ANDROID_SDK_ROOT 及常见安装路径；也可通过 ZNIFF_ADB 指定路径）"
        )
    })?;
    *LAST_ADB_BIN.lock().unwrap_or_else(|err| err.into_inner()) = Some(adb.clone());
    let out = Command::new(&adb)
        .arg("devices")
        .output()
        .with_context(|| format!("执行 {} devices 失败", adb.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                "adb devices 执行失败"
            } else {
                &stderr
            }
        );
    }
    let serials = parse_adb_devices(&String::from_utf8_lossy(&out.stdout));
    if serials.is_empty() {
        return Ok(AdbOutcome::NoneConnected);
    }

    let cert_path = ca.cert_path();
    let cert_str = cert_path.to_string_lossy().into_owned();
    let mut configured = Vec::new();
    let mut cert_pushed = false;

    for serial in &serials {
        if let Err(err) = run_adb(
            &adb,
            &adb_reverse_args(serial, port),
            &format!("为 Android 设备 {serial} 配置端口反转"),
        ) {
            remember_adb_proxies(&configured);
            return Err(err);
        }
        configured.push((serial.clone(), port));
        if let Err(err) = run_adb(
            &adb,
            &adb_set_proxy_args(serial, port),
            &format!("为 Android 设备 {serial} 设置全局代理"),
        ) {
            remember_adb_proxies(&configured);
            return Err(err);
        }
        if cert_path.is_file()
            && Command::new(&adb)
                .args([
                    "-s",
                    serial.as_str(),
                    "push",
                    cert_str.as_str(),
                    "/sdcard/Download/zniff-ca.crt",
                ])
                .output()
                .is_ok_and(|res| res.status.success())
        {
            cert_pushed = true;
        }
    }

    remember_adb_proxies(&configured);
    Ok(AdbOutcome::Configured {
        count: serials.len(),
        port,
        cert_pushed,
    })
}

/// Number of Android devices currently configured with adb reverse + global proxy by this process.
pub fn configured_adb_count() -> usize {
    ADB_PROXIED
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .len()
}

/// Restores global HTTP proxy and removes `adb reverse` mappings created by this process.
/// Returns the number of restored devices.
pub fn clear_android_adb_proxies() -> usize {
    let items = {
        let mut kept = ADB_PROXIED.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *kept)
    };
    let count = items.len();
    if items.is_empty() {
        return 0;
    }
    let Some(adb) = cached_or_resolve_adb_bin() else {
        return count;
    };
    for (serial, port) in items {
        let _ = Command::new(&adb)
            .args(adb_clear_proxy_args(&serial))
            .output();
        let _ = Command::new(&adb)
            .args(adb_reverse_remove_args(&serial, port))
            .output();
    }
    count
}

/// Process-exit / panic cleanup hook for Android proxies and exited browser temp profiles.
pub fn cleanup_on_exit() {
    let _ = clear_android_adb_proxies();
    sweep_exited_browser_profiles();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_args_include_proxy_spki_and_isolated_profile() {
        let dir = Path::new("/tmp/zniff-chrome-profile");
        let spki = vec![
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=".to_string(),
        ];
        let args = build_chromium_args(8877, &spki, dir, None);
        assert!(args.contains(&"--proxy-server=http://127.0.0.1:8877".to_string()));
        assert!(args.contains(
            &"--ignore-certificate-errors-spki-list=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=,BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=".to_string()
        ));
        assert!(args.contains(&"--user-data-dir=/tmp/zniff-chrome-profile".to_string()));
        assert!(args.contains(&"--no-first-run".to_string()));
        assert_eq!(
            args.last().map(String::as_str),
            Some("http://127.0.0.1:8877/zniff")
        );

        let custom = build_chromium_args(8877, &spki, dir, Some("https://api.vicanso.dev"));
        assert_eq!(
            custom.last().map(String::as_str),
            Some("https://api.vicanso.dev")
        );

        let fallback = build_chromium_args(8877, &spki, dir, Some("javascript:alert(1)"));
        assert_eq!(
            fallback.last().map(String::as_str),
            Some("http://127.0.0.1:8877/zniff")
        );
    }

    #[test]
    fn macos_terminal_script_exports_proxy_auth_and_self_cleans() {
        let script =
            build_macos_terminal_script(18878, Path::new("/tmp/zniff data/ca.crt"), "alice:s3cr@t");
        assert!(script.contains("export https_proxy=http://alice:s3cr%40t@127.0.0.1:18878"));
        assert!(script.contains("export SSL_CERT_FILE='/tmp/zniff data/ca.crt'"));
        assert!(script.contains("[zniff] Proxy active -> http://127.0.0.1:18878"));
        assert!(script.contains("rm -f -- \"$0\""));
    }

    #[test]
    fn adb_devices_parser_and_arg_builders_skip_offline_devices() {
        let sample = "\
* daemon not running; starting now at tcp:5037
List of devices attached
emulator-5554\tdevice product:sdk_gphone64_arm64
R5CR123456A\toffline
192.168.1.9:5555\tunauthorized
emulator-5556\tdevice
";
        assert_eq!(
            parse_adb_devices(sample),
            vec!["emulator-5554", "emulator-5556"]
        );
        assert_eq!(
            adb_reverse_args("emulator-5554", 8877),
            vec!["-s", "emulator-5554", "reverse", "tcp:8877", "tcp:8877"]
        );
        assert_eq!(
            adb_set_proxy_args("emulator-5554", 8877),
            vec![
                "-s",
                "emulator-5554",
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy",
                "127.0.0.1:8877"
            ]
        );
        assert_eq!(
            adb_clear_proxy_args("emulator-5554"),
            vec![
                "-s",
                "emulator-5554",
                "shell",
                "settings",
                "put",
                "global",
                "http_proxy",
                ":0"
            ]
        );
        assert_eq!(
            adb_reverse_remove_args("emulator-5554", 8877),
            vec!["-s", "emulator-5554", "reverse", "--remove", "tcp:8877"]
        );
    }

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(tag: &str) -> Self {
            let seq = LAUNCH_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "zniff-launcher-test-{}-{}-{seq}",
                tag,
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp test dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_test_file(path: &Path, content: &str, executable: bool) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, content).expect("write test file");
        #[cfg(unix)]
        {
            let mode = if executable { 0o755 } else { 0o644 };
            let mut perms = fs::metadata(path).expect("stat test file").permissions();
            perms.set_mode(mode);
            fs::set_permissions(path, perms).expect("chmod test file");
        }
        let _ = executable;
    }

    #[test]
    fn adb_resolver_follows_layer_priority_and_symlinks() {
        let root = TempDirGuard::new("adb-priority");
        let override_file = root.path().join("override-file").join(ADB_BIN_NAME);
        let override_dir = root.path().join("override-dir");
        let override_dir_bin = override_dir.join(ADB_BIN_NAME);
        let path_non_exec_dir = root.path().join("path-non-exec");
        let path_non_exec_bin = path_non_exec_dir.join(ADB_BIN_NAME);
        let path_exec_dir = root.path().join("path-exec");
        let path_real_target = root.path().join("caskroom").join(ADB_BIN_NAME);
        let path_exec_bin = path_exec_dir.join(ADB_BIN_NAME);
        let android_home = root.path().join("android-home");
        let android_home_bin = android_home.join("platform-tools").join(ADB_BIN_NAME);
        let sdk_root = root.path().join("sdk-root");
        let sdk_root_bin = sdk_root.join("platform-tools").join(ADB_BIN_NAME);
        let extra_dir = root.path().join("extra-paths-bin");
        let extra_bin = extra_dir.join(ADB_BIN_NAME);
        let paths_file = root.path().join("paths");

        write_test_file(&override_file, "#!/bin/sh\n", true);
        write_test_file(&override_dir_bin, "#!/bin/sh\n", true);
        write_test_file(&path_non_exec_bin, "not executable", false);
        write_test_file(&path_real_target, "#!/bin/sh\n", true);
        #[cfg(unix)]
        {
            fs::create_dir_all(&path_exec_dir).expect("create symlink parent");
            std::os::unix::fs::symlink(&path_real_target, &path_exec_bin)
                .expect("create adb symlink");
        }
        #[cfg(not(unix))]
        write_test_file(&path_exec_bin, "exe", true);
        write_test_file(&android_home_bin, "#!/bin/sh\n", true);
        write_test_file(&sdk_root_bin, "#!/bin/sh\n", true);
        write_test_file(&extra_bin, "#!/bin/sh\n", true);
        write_test_file(
            &paths_file,
            &format!("# comment\n\n  {}  \n# trailing\n", extra_dir.display()),
            false,
        );

        let joined_path =
            std::env::join_paths([&path_non_exec_dir, &path_exec_dir]).expect("join PATH");
        let mut env = AdbLookupEnv {
            override_bin: Some(override_file.clone()),
            path_var: Some(joined_path),
            android_home: Some(android_home),
            android_sdk_root: Some(sdk_root),
            home_dir: None,
            #[cfg(target_os = "windows")]
            local_app_data: None,
            extra_path_files: vec![paths_file.clone(), root.path().join("missing-paths")],
        };

        assert_eq!(resolve_adb_from_env(&env), Some(override_file));

        env.override_bin = Some(override_dir);
        assert_eq!(resolve_adb_from_env(&env), Some(override_dir_bin));

        env.override_bin = None;
        assert_eq!(resolve_adb_from_env(&env), Some(path_exec_bin));

        env.path_var = None;
        assert_eq!(resolve_adb_from_env(&env), Some(android_home_bin));

        env.android_home = None;
        assert_eq!(resolve_adb_from_env(&env), Some(sdk_root_bin));

        env.android_sdk_root = None;
        assert_eq!(
            read_path_dirs_from_files(&env.extra_path_files),
            vec![extra_dir]
        );
        assert!(well_known_adb_candidates(&env).contains(&extra_bin));
        let empty_env = AdbLookupEnv {
            override_bin: None,
            path_var: None,
            android_home: None,
            android_sdk_root: None,
            home_dir: None,
            #[cfg(target_os = "windows")]
            local_app_data: None,
            extra_path_files: Vec::new(),
        };
        if !well_known_adb_candidates(&empty_env)
            .into_iter()
            .any(|candidate| is_executable_file(&candidate))
        {
            assert_eq!(resolve_adb_from_env(&env), Some(extra_bin));
        }
    }

    #[cfg(unix)]
    #[test]
    fn adb_login_shell_output_parser_skips_noise_and_non_executables() {
        let root = TempDirGuard::new("adb-shell-parse");
        let valid_bin = root.path().join("bin").join("adb");
        let non_exec = root.path().join("plain").join("adb");
        let missing_bin = root.path().join("missing").join("adb");
        write_test_file(&valid_bin, "#!/bin/sh\n", true);
        write_test_file(&non_exec, "not executable", false);

        let noisy = format!(
            "Welcome banner\nalias adb='echo hi'\nadb\n{}\n{}\n{}\n",
            valid_bin.display(),
            missing_bin.display(),
            non_exec.display(),
        );
        assert_eq!(parse_login_shell_path(&noisy), Some(valid_bin));
        assert_eq!(
            parse_login_shell_path(&format!(
                "alias adb='foo'\n{}\n{}\n",
                missing_bin.display(),
                non_exec.display()
            )),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn adb_login_shell_probe_enforces_timeout() {
        let root = TempDirGuard::new("adb-shell-timeout");
        let slow_shell = root.path().join("slow-sh");
        write_test_file(&slow_shell, "#!/bin/sh\nexec sleep 30\n", true);

        let start = Instant::now();
        let found = probe_adb_in_shell(&slow_shell, Duration::from_millis(120));
        assert_eq!(found, None);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "timed out probe took too long: {:?}",
            start.elapsed()
        );
    }
}
