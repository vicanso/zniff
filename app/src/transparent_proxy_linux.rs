//! Linux cgroup v2 + `aya` eBPF transparent proxy host controller.
//!
//! Launches and drives `zniff-ebpf-helper` (either directly when `cap_bpf` +
//! `cap_net_admin` are granted or running as root, or via `pkexec`), keeps
//! `/proc` running applications synchronized, and forwards live config updates
//! over a JSON-RPC stdin/stdout control pipe.

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::transparent_proxy::{TpState, TransparentProxyConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HelperEvent {
    #[serde(rename_all = "camelCase")]
    Ready {
        bridge_port_v4: u16,
        bridge_port_v6: u16,
    },
    Updated,
    Error {
        message: String,
    },
    Pong,
}

/// Parses Linux kernel `major.minor` from `/proc/sys/kernel/osrelease`.
pub fn parse_kernel_version(release: &str) -> Option<(u32, u32)> {
    let clean = release.trim();
    let mut parts = clean.split(['.', '-', '+']);
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    Some((major, minor))
}

/// Returns `true` when the Linux kernel supports `bpf_get_socket_cookie` in
/// `cgroup/connect` (`>= 5.12`) and BPF links (`>= 5.7`).
pub fn kernel_supports_ebpf(release: &str) -> bool {
    parse_kernel_version(release).is_some_and(|v| v >= (5, 12))
}

/// Serializes an `Update` command line for `zniff-ebpf-helper`.
pub fn helper_command_json_update(cfg: &TransparentProxyConfig, zniff_pid: u32) -> String {
    serde_json::json!({
        "cmd": "update",
        "proxyPort": cfg.port,
        "proxyAuth": cfg.proxy_auth,
        "captureAll": cfg.capture_all,
        "targets": cfg.targets,
        "blockQuic": cfg.block_quic,
        "zniffPid": zniff_pid,
    })
    .to_string()
}

pub fn parse_helper_event(line: &str) -> Option<HelperEvent> {
    serde_json::from_str::<HelperEvent>(line.trim()).ok()
}

/// Extracts `(Name, Exec basename)` from an XDG `.desktop` file.
pub fn parse_desktop_entry_name_and_exec(content: &str) -> Option<(String, String)> {
    let mut in_entry = false;
    let mut name: Option<String> = None;
    let mut exec_raw: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            if key == "Name" && name.is_none() && !val.is_empty() {
                name = Some(val.to_string());
            } else if key == "Exec" && exec_raw.is_none() && !val.is_empty() {
                exec_raw = Some(val.to_string());
            }
        }
    }

    let name = name?;
    let exec_line = exec_raw?;
    let mut parts = exec_line.split_whitespace();
    let mut candidate = parts.next()?.trim_matches('"');
    if candidate == "env" || candidate.ends_with("/env") {
        for next in parts {
            let cleaned = next.trim_matches('"');
            if !cleaned.contains('=') && !cleaned.starts_with('-') {
                candidate = cleaned;
                break;
            }
        }
    }
    let basename = std::path::Path::new(candidate)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(candidate)
        .trim();
    (!basename.is_empty()).then(|| (name, basename.to_string()))
}

/// Checks `getcap` output for both `cap_bpf` and `cap_net_admin`.
pub fn has_cap_bpf_and_net_admin(getcap_output: &str) -> bool {
    let lower = getcap_output.to_ascii_lowercase();
    lower.contains("cap_bpf") && lower.contains("cap_net_admin")
}

/// Formats the `sudo setcap` command for granting passwordless eBPF capabilities
/// to `helper_path`, single-quoting the path only when needed for shell safety.
pub fn format_setcap_command(helper_path: &std::path::Path) -> String {
    let raw = helper_path.to_string_lossy();
    let safe_unquoted = !raw.is_empty()
        && raw.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-' | b':' | b'@' | b'+')
        });
    let arg = if safe_unquoted {
        raw.into_owned()
    } else {
        let escaped = raw.replace('\'', "'\\''");
        format!("'{escaped}'")
    };
    format!("sudo setcap cap_bpf,cap_net_admin,cap_sys_resource+ep {arg}")
}

#[allow(dead_code)]
fn runtime_state_slot() -> &'static Mutex<RuntimeStatus> {
    static SLOT: OnceLock<Mutex<RuntimeStatus>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(RuntimeStatus::default()))
}

#[derive(Default, Clone)]
#[allow(dead_code)]
struct RuntimeStatus {
    state: TpState,
    active: bool,
    error: String,
}

#[cfg(all(target_os = "linux", feature = "desktop", not(target_arch = "wasm32")))]
pub mod native {
    use std::collections::{HashMap, HashSet};
    use std::env;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use super::{
        HelperEvent, format_setcap_command, has_cap_bpf_and_net_admin, helper_command_json_update,
        kernel_supports_ebpf, parse_desktop_entry_name_and_exec, parse_helper_event,
        runtime_state_slot,
    };
    use crate::transparent_proxy::{
        RunningApp, StartOutcome, TpState, TransparentProxyConfig, TransparentProxyStatus,
    };

    const HELPER_NAME: &str = "zniff-ebpf-helper";

    struct HelperSession {
        child: Child,
        stdin: ChildStdin,
    }

    fn helper_slot() -> &'static Mutex<Option<HelperSession>> {
        static SLOT: OnceLock<Mutex<Option<HelperSession>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    pub fn find_helper_binary() -> Option<PathBuf> {
        if let Some(overridden) = env::var_os("ZNIFF_EBPF_HELPER").filter(|v| !v.is_empty()) {
            let p = PathBuf::from(overridden);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Ok(exe) = env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let beside = dir.join(HELPER_NAME);
            if beside.is_file() {
                return Some(beside);
            }
        }
        if let Some(paths) = env::var_os("PATH") {
            for dir in env::split_paths(&paths) {
                let candidate = dir.join(HELPER_NAME);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub fn check_linux_prereqs() -> Result<(), String> {
        let release = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        if !kernel_supports_ebpf(&release) {
            return Err(format!(
                "需要 Linux 5.12+ 内核以支持 eBPF 透明代理（当前: {}）",
                release.trim()
            ));
        }
        if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
            return Err("需要开启 cgroup v2 (/sys/fs/cgroup/cgroup.controllers)".to_string());
        }
        Ok(())
    }

    fn has_direct_ebpf_caps(helper: &Path) -> bool {
        if unsafe { libc::geteuid() } == 0 {
            return true;
        }
        for cmd in ["getcap", "/usr/sbin/getcap", "/sbin/getcap"] {
            if let Ok(out) = Command::new(cmd).arg(helper.as_os_str()).output()
                && out.status.success()
            {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if has_cap_bpf_and_net_admin(&stdout) {
                    return true;
                }
            }
        }
        false
    }

    pub fn refresh_status() -> TransparentProxyStatus {
        reap_exited_helper();
        let helper = find_helper_binary();
        let embedded = helper.is_some();
        let passwordless_ready = helper.as_deref().is_some_and(has_direct_ebpf_caps);
        let setcap_command = match helper.as_deref() {
            Some(p) if !passwordless_ready => format_setcap_command(p),
            _ => String::new(),
        };
        let prereq_ok = check_linux_prereqs().is_ok();
        let rt = runtime_state_slot()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        TransparentProxyStatus {
            embedded,
            in_applications: prereq_ok,
            state: rt.state,
            active: rt.active,
            error: rt.error,
            passwordless_ready,
            setcap_command,
        }
    }

    fn reap_exited_helper() {
        let mut slot = helper_slot().lock().unwrap_or_else(|e| e.into_inner());
        let exited = match slot.as_mut() {
            Some(session) => matches!(session.child.try_wait(), Ok(Some(_))),
            None => false,
        };
        if exited {
            *slot = None;
            let mut rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if rt.active || matches!(rt.state, TpState::Running | TpState::Starting) {
                rt.active = false;
                rt.state = TpState::Idle;
            }
        }
    }

    pub fn start(cfg: &TransparentProxyConfig) -> Result<StartOutcome, String> {
        let helper = find_helper_binary().ok_or_else(|| "未找到 zniff-ebpf-helper".to_string())?;
        check_linux_prereqs()?;

        {
            let rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if rt.active || matches!(rt.state, TpState::Running | TpState::Starting) {
                drop(rt);
                update_config(cfg)?;
                return Ok(StartOutcome::Started);
            }
        }

        {
            let mut rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            rt.state = TpState::Starting;
            rt.active = false;
            rt.error.clear();
        }

        let direct_caps = has_direct_ebpf_caps(&helper);
        let setcap_hint = format_setcap_command(&helper);
        let mut cmd = if direct_caps {
            Command::new(&helper)
        } else {
            let mut pk = Command::new("pkexec");
            pk.arg(&helper);
            pk
        };
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                let msg = if direct_caps {
                    format!("启动 eBPF 助手失败: {err}")
                } else {
                    format!("启动 eBPF 助手失败: {err}（若未安装 pkexec，可运行：{setcap_hint}）")
                };
                let mut rt = runtime_state_slot()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                rt.state = TpState::Error;
                rt.error = msg.clone();
                return Err(msg);
            }
        };

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法打开 eBPF 助手控制管道".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法读取 eBPF 助手输出".to_string())?;

        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let early_exit_msg = if direct_caps {
            "eBPF 助手提前退出".to_string()
        } else {
            format!("授权取消或未检测到 Polkit 授权弹窗，可运行以下命令配置免密启动：{setcap_hint}")
        };
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut tx = Some(ready_tx);
            for line in reader.lines() {
                let Ok(raw) = line else { break };
                let Some(evt) = parse_helper_event(&raw) else {
                    continue;
                };
                match evt {
                    HelperEvent::Ready { .. } => {
                        if let Some(sender) = tx.take() {
                            let _ = sender.send(Ok(()));
                        }
                    }
                    HelperEvent::Error { message } => {
                        if let Some(sender) = tx.take() {
                            let _ = sender.send(Err(message.clone()));
                        } else {
                            let mut rt = runtime_state_slot()
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            rt.state = TpState::Error;
                            rt.error = message;
                        }
                    }
                    HelperEvent::Updated | HelperEvent::Pong => {}
                }
            }
            if let Some(sender) = tx.take() {
                let _ = sender.send(Err(early_exit_msg));
            }
        });

        match ready_rx.recv_timeout(Duration::from_secs(45)) {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = child.kill();
                let _ = child.wait();
                let mut rt = runtime_state_slot()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                rt.state = TpState::Error;
                rt.active = false;
                rt.error = err.clone();
                return Err(err);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let msg = "等待 eBPF 助手就绪超时".to_string();
                let mut rt = runtime_state_slot()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                rt.state = TpState::Error;
                rt.active = false;
                rt.error = msg.clone();
                return Err(msg);
            }
        }

        let init_cmd = helper_command_json_update(cfg, std::process::id());
        if let Err(err) = writeln!(stdin, "{init_cmd}").and_then(|()| stdin.flush()) {
            let _ = child.kill();
            let _ = child.wait();
            let msg = format!("写入 eBPF 配置失败: {err}");
            let mut rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            rt.state = TpState::Error;
            rt.active = false;
            rt.error = msg.clone();
            return Err(msg);
        }

        *helper_slot().lock().unwrap_or_else(|e| e.into_inner()) =
            Some(HelperSession { child, stdin });
        {
            let mut rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            rt.state = TpState::Running;
            rt.active = true;
            rt.error.clear();
        }
        Ok(StartOutcome::Started)
    }

    pub fn update_config(cfg: &TransparentProxyConfig) -> Result<(), String> {
        let line = helper_command_json_update(cfg, std::process::id());
        let mut slot = helper_slot().lock().unwrap_or_else(|e| e.into_inner());
        let Some(session) = slot.as_mut() else {
            return Ok(());
        };
        writeln!(session.stdin, "{line}")
            .and_then(|()| session.stdin.flush())
            .map_err(|err| format!("同步 eBPF 配置失败: {err}"))
    }

    pub fn stop() -> Result<(), String> {
        {
            let mut rt = runtime_state_slot()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            rt.state = TpState::Stopping;
        }
        let session = helper_slot()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(mut session) = session {
            let _ = writeln!(session.stdin, r#"{{"cmd":"stop"}}"#);
            let _ = session.stdin.flush();
            drop(session.stdin);
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match session.child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        let _ = session.child.kill();
                        let _ = session.child.wait();
                        break;
                    }
                }
            }
        }
        let mut rt = runtime_state_slot()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        rt.state = TpState::Idle;
        rt.active = false;
        rt.error.clear();
        Ok(())
    }

    fn load_desktop_exec_to_name() -> HashMap<String, String> {
        let mut map = HashMap::new();
        let mut dirs = vec![PathBuf::from("/usr/share/applications")];
        if let Ok(home) = env::var("HOME") {
            dirs.push(Path::new(&home).join(".local/share/applications"));
        }
        if let Ok(xdg) = env::var("XDG_DATA_HOME") {
            dirs.push(Path::new(&xdg).join("applications"));
        }
        for dir in dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(&p)
                    && let Some((name, exec_bin)) = parse_desktop_entry_name_and_exec(&text)
                {
                    map.entry(exec_bin.to_ascii_lowercase()).or_insert(name);
                }
            }
        }
        map
    }

    pub fn list_running_apps() -> Vec<RunningApp> {
        let my_uid = unsafe { libc::getuid() };
        let desktop_names = load_desktop_exec_to_name();
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut apps = Vec::new();

        for entry in entries.flatten() {
            let fname = entry.file_name();
            let Some(pid) = fname.to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            if pid == std::process::id() {
                continue;
            }
            let proc_dir = entry.path();
            if let Ok(meta) = fs::metadata(&proc_dir) {
                use std::os::linux::fs::MetadataExt;
                if meta.st_uid() != my_uid {
                    continue;
                }
            }
            let comm = fs::read_to_string(proc_dir.join("comm"))
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let exe_name = fs::read_link(proc_dir.join("exe"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let exec = if !exe_name.is_empty() { exe_name } else { comm };
            if exec.is_empty() {
                continue;
            }
            let exec_low = exec.to_ascii_lowercase();
            if matches!(
                exec_low.as_str(),
                "zniff"
                    | "zniff-ebpf-helper"
                    | "systemd"
                    | "dbus-daemon"
                    | "dbus-broker"
                    | "pipewire"
                    | "wireplumber"
                    | "bash"
                    | "zsh"
                    | "sh"
            ) {
                continue;
            }
            let display = desktop_names
                .get(&exec_low)
                .cloned()
                .unwrap_or_else(|| exec.clone());
            let key = display.to_ascii_lowercase();
            if seen.insert(key) {
                apps.push(RunningApp {
                    name: display,
                    bundle_id: String::new(),
                    exec_name: exec,
                });
            }
        }
        apps.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
        apps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_version_check_requires_5_12_or_newer() {
        assert_eq!(parse_kernel_version("6.8.0-45-generic"), Some((6, 8)));
        assert_eq!(parse_kernel_version("5.12.0"), Some((5, 12)));
        assert!(kernel_supports_ebpf("6.8.0-45-generic"));
        assert!(kernel_supports_ebpf("5.12.0"));
        assert!(!kernel_supports_ebpf("5.10.220-lts"));
        assert!(!kernel_supports_ebpf("4.19.0"));
    }

    #[test]
    fn desktop_entry_and_getcap_helpers_work() {
        let parsed = parse_desktop_entry_name_and_exec(
            "[Desktop Entry]\nName=Firefox\nExec=env MOZ_ENABLE_WAYLAND=1 /usr/bin/firefox %u\n",
        );
        assert_eq!(parsed, Some(("Firefox".into(), "firefox".into())));
        assert!(has_cap_bpf_and_net_admin(
            "/home/u/.local/bin/zniff-ebpf-helper cap_net_admin,cap_bpf,cap_sys_resource=ep"
        ));
        assert!(!has_cap_bpf_and_net_admin(
            "/home/u/.local/bin/zniff-ebpf-helper cap_net_admin=ep"
        ));
    }

    #[test]
    fn setcap_command_formatting_quotes_when_needed() {
        assert_eq!(
            format_setcap_command(std::path::Path::new("/home/u/.local/bin/zniff-ebpf-helper")),
            "sudo setcap cap_bpf,cap_net_admin,cap_sys_resource+ep /home/u/.local/bin/zniff-ebpf-helper"
        );
        assert_eq!(
            format_setcap_command(std::path::Path::new("/tmp/my 'dir'/zniff-ebpf-helper")),
            "sudo setcap cap_bpf,cap_net_admin,cap_sys_resource+ep '/tmp/my '\\''dir'\\''/zniff-ebpf-helper'"
        );
    }

    #[test]
    fn helper_command_and_event_json_match_protocol() {
        let cfg = TransparentProxyConfig {
            port: 8877,
            proxy_auth: "u:p".into(),
            capture_all: false,
            targets: vec!["firefox".into(), "curl".into()],
            block_quic: true,
        };
        let line = helper_command_json_update(&cfg, 1234);
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["cmd"], "update");
        assert_eq!(parsed["proxyPort"], 8877);
        assert_eq!(parsed["zniffPid"], 1234);

        let evt =
            parse_helper_event(r#"{"event":"ready","bridgePortV4":41001,"bridgePortV6":41002}"#);
        assert_eq!(
            evt,
            Some(HelperEvent::Ready {
                bridge_port_v4: 41001,
                bridge_port_v6: 41002,
            })
        );
    }
}
