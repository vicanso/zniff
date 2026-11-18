//! Cross-platform local loopback client port -> process name attribution.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_CONCURRENT_PORT_PROBES: usize = 6;
static PROBE_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct ProbeSlotGuard;

impl ProbeSlotGuard {
    fn try_acquire() -> Option<Self> {
        loop {
            let cur = PROBE_IN_FLIGHT.load(Ordering::Acquire);
            if cur >= MAX_CONCURRENT_PORT_PROBES {
                return None;
            }
            if PROBE_IN_FLIGHT
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(Self);
            }
        }
    }
}

impl Drop for ProbeSlotGuard {
    fn drop(&mut self) {
        PROBE_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

fn local_app_cache() -> &'static Mutex<HashMap<u16, (String, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<u16, (String, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn cached_local_app(port: u16) -> Option<String> {
    let mut guard = local_app_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if let Some((app, at)) = guard.get(&port)
        && at.elapsed() < Duration::from_secs(15)
    {
        return (!app.is_empty()).then(|| app.clone());
    }
    if guard.len() > 256 {
        guard.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(15));
    }
    None
}

pub(crate) fn store_cached_local_app(port: u16, app: String) {
    if app.is_empty() {
        return;
    }
    let mut guard = local_app_cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    guard.insert(port, (app, Instant::now()));
}

struct LocalPorts {
    at: Instant,
    map: HashMap<u16, String>,
}

fn snapshot_state() -> &'static Mutex<LocalPorts> {
    static SNAP: OnceLock<Mutex<LocalPorts>> = OnceLock::new();
    SNAP.get_or_init(|| {
        let old = Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now);
        Mutex::new(LocalPorts {
            at: old,
            map: HashMap::new(),
        })
    })
}

fn port_snapshot(peer_port: u16) -> Option<String> {
    let snap = snapshot_state();
    let need_full = {
        let mut guard = snap.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(hit) = guard.map.get(&peer_port) {
            return Some(hit.clone());
        }
        let now = Instant::now();
        let full = now.duration_since(guard.at) >= Duration::from_millis(1500);
        if full {
            guard.at = now;
        }
        full
    };

    if need_full && let Some(map) = scan_local_ports() {
        let hit = map.get(&peer_port).cloned();
        let mut guard = snap.lock().unwrap_or_else(|err| err.into_inner());
        if guard.map.len() > 1024 {
            guard.map.clear();
        }
        guard.map.extend(map);
        guard.at = Instant::now();
        if hit.is_some() {
            return hit;
        }
    }

    let _slot = ProbeSlotGuard::try_acquire()?;
    probe_single_port(peer_port).inspect(|name| {
        let mut guard = snap.lock().unwrap_or_else(|err| err.into_inner());
        if guard.map.len() > 1024 {
            guard.map.clear();
        }
        guard.map.insert(peer_port, name.clone());
    })
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(crate) fn local_endpoint_port(name: &str) -> Option<u16> {
    let local = name.split("->").next()?.trim();
    let port = local.rsplit(':').next()?.trim();
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    port.parse().ok()
}

#[cfg(test)]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    Some(HashMap::new())
}

#[cfg(all(not(test), target_os = "macos"))]
fn parse_lsof_fpcn(stdout: &str) -> HashMap<u16, String> {
    let mut map = HashMap::new();
    let mut current = String::new();
    for line in stdout.lines() {
        if let Some(cmd) = line.strip_prefix('c') {
            current = clean_process_name(cmd.trim());
        } else if let Some(net) = line.strip_prefix('n')
            && !current.is_empty()
            && let Some(port) = local_endpoint_port(net)
        {
            map.entry(port).or_insert_with(|| current.clone());
        }
    }
    map
}

#[cfg(all(not(test), target_os = "macos"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let out = std::process::Command::new("lsof")
        .args(["+c", "0", "-nP", "-iTCP", "-Fpcn"])
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Some(parse_lsof_fpcn(&stdout))
}

#[cfg(all(not(test), target_os = "macos"))]
fn probe_single_port(peer_port: u16) -> Option<String> {
    let filter = format!("-iTCP:{peer_port}");
    let out = std::process::Command::new("lsof")
        .args(["+c", "0", "-nP", &filter, "-Fpcn"])
        .output()
        .ok()?;
    if out.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_lsof_fpcn(&stdout).remove(&peer_port)
}

#[cfg(any(test, not(target_os = "macos")))]
fn probe_single_port(_peer_port: u16) -> Option<String> {
    None
}

#[cfg(all(not(test), target_os = "linux"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let mut inode_port: HashMap<String, u16> = HashMap::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in content.lines().skip(1) {
            let mut cols = line.split_whitespace();
            let _sl = cols.next();
            let Some(local_addr) = cols.next() else {
                continue;
            };
            let Some((_, port_hex)) = local_addr.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = u16::from_str_radix(port_hex, 16) else {
                continue;
            };
            let Some(inode) = cols.nth(7) else {
                continue;
            };
            if inode != "0" {
                inode_port.entry(inode.to_string()).or_insert(port);
            }
        }
    }
    if inode_port.is_empty() {
        return Some(HashMap::new());
    }
    let mut map = HashMap::new();
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(pid_str) = fname.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let fd_dir = entry.path().join("fd");
        let Ok(fds) = std::fs::read_dir(fd_dir) else {
            continue;
        };
        let mut name = None;
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let link = link.to_string_lossy();
            let Some(inode) = link
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            let Some(port) = inode_port.get(inode).copied() else {
                continue;
            };
            if name.is_none() {
                let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
                name = Some(clean_process_name(comm.trim()));
            }
            let Some(name) = name.as_ref() else {
                continue;
            };
            if !name.is_empty() {
                map.entry(port).or_insert_with(|| name.clone());
            }
        }
    }
    Some(map)
}

#[cfg(all(not(test), target_os = "windows"))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    let out = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut port_pid: HashMap<u16, u32> = HashMap::new();
    for line in stdout.lines() {
        let mut cols = line.split_whitespace();
        let Some(proto) = cols.next() else {
            continue;
        };
        if !proto.eq_ignore_ascii_case("TCP") {
            continue;
        }
        let Some(local) = cols.next() else {
            continue;
        };
        let Some(_remote) = cols.next() else {
            continue;
        };
        let Some(state) = cols.next() else {
            continue;
        };
        let Some(pid) = cols.next().and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        if !state.eq_ignore_ascii_case("ESTABLISHED") || pid == 0 {
            continue;
        }
        if let Some(port) = local_endpoint_port(local) {
            port_pid.entry(port).or_insert(pid);
        }
    }
    let tlist = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let t_out = String::from_utf8_lossy(&tlist.stdout);
    let mut pid_name: HashMap<u32, String> = HashMap::new();
    for line in t_out.lines() {
        let mut cols = line.split(',');
        let Some(name) = cols.next() else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        let Some(pid) = cols
            .next()
            .map(|text| text.trim().trim_matches('"'))
            .and_then(|text| text.parse::<u32>().ok())
        else {
            continue;
        };
        if !name.is_empty() && pid > 0 {
            pid_name
                .entry(pid)
                .or_insert_with(|| clean_process_name(name));
        }
    }
    let mut map = HashMap::new();
    for (port, pid) in port_pid {
        if let Some(name) = pid_name.get(&pid)
            && !name.is_empty()
        {
            map.insert(port, name.clone());
        }
    }
    Some(map)
}

#[cfg(all(
    not(test),
    not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
))]
fn scan_local_ports() -> Option<HashMap<u16, String>> {
    None
}

/// Identifies the local process name owning `peer_port` on loopback connections.
pub fn resolve_local_process_name(peer_port: u16) -> Option<String> {
    if peer_port == 0 {
        return None;
    }
    if let Some(name) = cached_local_app(peer_port) {
        return Some(name);
    }
    let name = port_snapshot(peer_port)?;
    if name.is_empty() {
        return None;
    }
    store_cached_local_app(peer_port, name.clone());
    Some(name)
}

/// Decodes lsof hex/escape sequences (`\x20`, multi-byte UTF-8 `\xe5\xbe\xae\xe4\xbf\xa1`, etc.).
pub(crate) fn clean_process_name(raw: &str) -> String {
    let bytes = raw.trim().as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'x' | b'X' if i + 3 < bytes.len() => {
                    if let (Some(hi), Some(lo)) =
                        (hex_nibble(bytes[i + 2]), hex_nibble(bytes[i + 3]))
                    {
                        out.push((hi << 4) | lo);
                        i += 4;
                        continue;
                    }
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                    continue;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8_lossy(&out);
    let trimmed = decoded.trim();
    trimmed
        .strip_suffix(".exe")
        .or_else(|| trimmed.strip_suffix(".EXE"))
        .unwrap_or(trimmed)
        .to_string()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_process_name_decodes_spaces_utf8_escapes_and_exe() {
        assert_eq!(
            clean_process_name(r"Google\x20Chrome\x20Helper"),
            "Google Chrome Helper"
        );
        assert_eq!(clean_process_name(r"\xe5\xbe\xae\xe4\xbf\xa1"), "微信");
        assert_eq!(clean_process_name("curl.exe"), "curl");
        assert_eq!(clean_process_name("  Postman  "), "Postman");
    }

    #[test]
    fn local_endpoint_port_parses_ipv4_ipv6_and_skips_remote() {
        assert_eq!(
            local_endpoint_port("127.0.0.1:54321->127.0.0.1:8878"),
            Some(54321)
        );
        assert_eq!(local_endpoint_port("[::1]:61234->[::1]:8878"), Some(61234));
        assert_eq!(local_endpoint_port("127.0.0.1:54321"), Some(54321));
        assert_eq!(local_endpoint_port("*:8878"), Some(8878));
        assert_eq!(local_endpoint_port("invalid"), None);
    }
}
