use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use zniff_ebpf_common::CommKey;

const EXCLUDED_SYSTEM_NAMES: &[&str] = &[
    "zniff",
    "zniff-ebpf-helper",
    "systemd-resolved",
    "systemd-resolve",
    "systemd-networkd",
    "systemd-network",
    "networkmanager",
    "dnsmasq",
    "avahi-daemon",
];

#[derive(Debug, Clone)]
pub struct ProcEntry {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub exe_name: String,
    pub cmdline: String,
}

#[derive(Debug, Clone, Default)]
pub struct DesktopMap {
    /// Lowercased executable basename -> friendly `.desktop` display name.
    pub exec_to_name: HashMap<String, String>,
    /// Lowercased friendly `.desktop` display name -> executable basenames.
    pub name_to_execs: HashMap<String, Vec<String>>,
}

pub fn load_desktop_map() -> DesktopMap {
    let mut dirs = vec![PathBuf::from("/usr/share/applications")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(Path::new(&home).join(".local/share/applications"));
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        dirs.push(Path::new(&xdg_data).join("applications"));
    }
    load_desktop_map_from_dirs(&dirs)
}

pub fn load_desktop_map_from_dirs(dirs: &[PathBuf]) -> DesktopMap {
    let mut map = DesktopMap::default();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some((name, exec_bin)) = parse_desktop_entry(&text) {
                let exec_low = exec_bin.to_ascii_lowercase();
                let name_low = name.to_ascii_lowercase();
                map.exec_to_name.entry(exec_low.clone()).or_insert(name);
                let list = map.name_to_execs.entry(name_low).or_default();
                if !list.contains(&exec_low) {
                    list.push(exec_low);
                }
            }
        }
    }
    map
}

pub fn parse_desktop_entry(content: &str) -> Option<(String, String)> {
    let mut in_desktop_entry = false;
    let mut name: Option<String> = None;
    let mut exec_raw: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_desktop_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
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
    let exec_bin = extract_exec_basename(&exec_line)?;
    Some((name, exec_bin))
}

fn extract_exec_basename(exec_line: &str) -> Option<String> {
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
    let basename = Path::new(candidate)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(candidate)
        .trim();
    (!basename.is_empty()).then(|| basename.to_string())
}

#[cfg(target_os = "linux")]
pub fn list_proc_entries() -> Vec<ProcEntry> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let proc_dir = entry.path();
        let comm = fs::read_to_string(proc_dir.join("comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let stat = fs::read_to_string(proc_dir.join("stat")).unwrap_or_default();
        let ppid = parse_ppid_from_stat(&stat).unwrap_or(0);
        let exe_name = fs::read_link(proc_dir.join("exe"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let cmdline = fs::read(proc_dir.join("cmdline"))
            .map(|raw| {
                raw.split(|&b| b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        out.push(ProcEntry {
            pid,
            ppid,
            comm,
            exe_name,
            cmdline,
        });
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn list_proc_entries() -> Vec<ProcEntry> {
    Vec::new()
}

pub fn parse_ppid_from_stat(stat: &str) -> Option<u32> {
    // Format: "<pid> (<comm with spaces>) <state> <ppid> ..."
    let close_paren = stat.rfind(')')?;
    let after = stat.get(close_paren + 1..)?.trim_start();
    let mut fields = after.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse::<u32>().ok()
}

pub fn expand_targets_with_desktop(targets: &[String], desktop: &DesktopMap) -> Vec<String> {
    let mut expanded = Vec::new();
    for raw in targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let low = trimmed.to_ascii_lowercase();
        if !expanded.contains(&low) {
            expanded.push(low.clone());
        }
        if let Some(execs) = desktop.name_to_execs.get(&low) {
            for exec in execs {
                if !expanded.contains(exec) {
                    expanded.push(exec.clone());
                }
            }
        }
    }
    expanded
}

pub fn is_system_excluded_proc(entry: &ProcEntry) -> bool {
    let comm_low = entry.comm.to_ascii_lowercase();
    let exe_low = entry.exe_name.to_ascii_lowercase();
    EXCLUDED_SYSTEM_NAMES
        .iter()
        .any(|name| comm_low == *name || exe_low == *name)
}

pub fn detect_excluded_system_pids(procs: &[ProcEntry], self_pid: u32, zniff_pid: u32) -> Vec<u32> {
    let mut pids = Vec::new();
    if self_pid > 0 {
        pids.push(self_pid);
    }
    if zniff_pid > 0 && !pids.contains(&zniff_pid) {
        pids.push(zniff_pid);
    }
    for proc_entry in procs {
        if proc_entry.pid > 0
            && is_system_excluded_proc(proc_entry)
            && !pids.contains(&proc_entry.pid)
        {
            pids.push(proc_entry.pid);
        }
    }
    pids
}

pub fn scan_matching_pids(
    procs: &[ProcEntry],
    expanded_targets: &[String],
    excluded_pids: &[u32],
) -> (Vec<u32>, Vec<CommKey>) {
    let excluded: HashSet<u32> = excluded_pids.iter().copied().collect();
    let mut matched_pids: HashSet<u32> = HashSet::new();
    let mut target_comms: Vec<CommKey> = Vec::new();

    for target in expanded_targets {
        if target.parse::<u32>().is_err() && !target.is_empty() {
            let key = CommKey::from_str_lowercased(target);
            if !target_comms.contains(&key) {
                target_comms.push(key);
            }
        }
    }

    for entry in procs {
        if entry.pid == 0 || excluded.contains(&entry.pid) {
            continue;
        }
        if entry_matches_targets(entry, expanded_targets) {
            matched_pids.insert(entry.pid);
        }
    }

    // Propagate matches down to descendant processes (e.g. Chromium network/renderer helpers).
    let mut children_by_ppid: HashMap<u32, Vec<u32>> = HashMap::new();
    for entry in procs {
        if entry.ppid > 0 && entry.pid != entry.ppid {
            children_by_ppid
                .entry(entry.ppid)
                .or_default()
                .push(entry.pid);
        }
    }
    let mut stack: Vec<u32> = matched_pids.iter().copied().collect();
    while let Some(pid) = stack.pop() {
        if let Some(children) = children_by_ppid.get(&pid) {
            for &child in children {
                if !excluded.contains(&child) && matched_pids.insert(child) {
                    stack.push(child);
                }
            }
        }
    }

    let mut out_pids: Vec<u32> = matched_pids.into_iter().collect();
    out_pids.sort_unstable();
    (out_pids, target_comms)
}

fn entry_matches_targets(entry: &ProcEntry, expanded_targets: &[String]) -> bool {
    let comm_low = entry.comm.to_ascii_lowercase();
    let exe_low = entry.exe_name.to_ascii_lowercase();
    let pid_str = entry.pid.to_string();
    let first_arg = entry
        .cmdline
        .split_whitespace()
        .next()
        .and_then(|tok| Path::new(tok).file_name().and_then(|n| n.to_str()))
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    for target in expanded_targets {
        if target == &pid_str {
            return true;
        }
        if !comm_low.is_empty() && comm_low == *target {
            return true;
        }
        if !exe_low.is_empty() && exe_low == *target {
            return true;
        }
        if !first_arg.is_empty() && first_arg == *target {
            return true;
        }
    }
    false
}

pub fn resolve_display_label(pid: u32, comm: &[u8; 16], desktop: &DesktopMap) -> String {
    if pid > 0 {
        let exe_path = format!("/proc/{pid}/exe");
        if let Ok(link) = fs::read_link(exe_path) {
            if let Some(base) = link.file_name().and_then(|n| n.to_str()) {
                if let Some(friendly) = desktop.exec_to_name.get(&base.to_ascii_lowercase()) {
                    return friendly.clone();
                }
                if !base.is_empty() {
                    return base.to_string();
                }
            }
        }
        let comm_path = format!("/proc/{pid}/comm");
        if let Ok(text) = fs::read_to_string(comm_path) {
            let trimmed = text.trim();
            if let Some(friendly) = desktop.exec_to_name.get(&trimmed.to_ascii_lowercase()) {
                return friendly.clone();
            }
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    let end = comm.iter().position(|&b| b == 0).unwrap_or(comm.len());
    let fallback = String::from_utf8_lossy(&comm[..end]).trim().to_string();
    if let Some(friendly) = desktop.exec_to_name.get(&fallback.to_ascii_lowercase()) {
        return friendly.clone();
    }
    fallback
}

/// Reads tracefs `sched_process_fork/format` to dynamically determine `child_pid` offset
/// across Linux kernel versions without hardcoding kernel struct layouts.
pub fn detect_fork_child_pid_offset() -> Option<u32> {
    for path in [
        "/sys/kernel/tracing/events/sched/sched_process_fork/format",
        "/sys/kernel/debug/tracing/events/sched/sched_process_fork/format",
    ] {
        if let Ok(text) = fs::read_to_string(path) {
            if let Some(off) = parse_tracepoint_field_offset(&text, "child_pid") {
                return Some(off);
            }
        }
    }
    None
}

pub fn parse_tracepoint_field_offset(format_text: &str, field_name: &str) -> Option<u32> {
    let needle = format!("{field_name};");
    for line in format_text.lines() {
        if !line.contains(&needle) {
            continue;
        }
        for part in line.split('\t') {
            let trimmed = part.trim().trim_end_matches(';');
            if let Some(rest) = trimmed.strip_prefix("offset:") {
                if let Ok(val) = rest.trim().parse::<u32>() {
                    return Some(val);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracepoint_field_offset_from_format_text() {
        let sample = "\
name: sched_process_fork
ID: 318
format:
\tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;
\tfield:unsigned char common_flags;\toffset:2;\tsize:1;\tsigned:0;
\tfield:char parent_comm[16];\toffset:8;\tsize:16;\tsigned:0;
\tfield:pid_t parent_pid;\toffset:24;\tsize:4;\tsigned:1;
\tfield:char child_comm[16];\toffset:28;\tsize:16;\tsigned:0;
\tfield:pid_t child_pid;\toffset:44;\tsize:4;\tsigned:1;
";
        assert_eq!(parse_tracepoint_field_offset(sample, "child_pid"), Some(44));
        assert_eq!(parse_tracepoint_field_offset(sample, "missing"), None);
    }

    #[test]
    fn parses_ppid_from_proc_stat_including_spaces_in_comm() {
        let line = "1234 (Web Content (Isolated)) S 987 987 987 0 -1 4194304 0 0";
        assert_eq!(parse_ppid_from_stat(line), Some(987));
    }

    #[test]
    fn parses_desktop_entry_and_expands_descendant_pids() {
        let desktop_txt = "\
[Desktop Entry]
Name=Google Chrome
Exec=/usr/bin/google-chrome-stable %U
Type=Application
";
        assert_eq!(
            parse_desktop_entry(desktop_txt),
            Some(("Google Chrome".into(), "google-chrome-stable".into()))
        );

        let mut map = DesktopMap::default();
        map.name_to_execs
            .insert("google chrome".into(), vec!["google-chrome-stable".into()]);
        let expanded = expand_targets_with_desktop(&["Google Chrome".into()], &map);
        assert!(expanded.contains(&"google-chrome-stable".into()));

        let procs = vec![
            ProcEntry {
                pid: 100,
                ppid: 1,
                comm: "chrome".into(),
                exe_name: "google-chrome-stable".into(),
                cmdline: "/opt/google/chrome/chrome".into(),
            },
            ProcEntry {
                pid: 101,
                ppid: 100,
                comm: "Chrome_IOThread".into(),
                exe_name: "chrome".into(),
                cmdline: "/opt/google/chrome/chrome --type=utility".into(),
            },
            ProcEntry {
                pid: 200,
                ppid: 1,
                comm: "systemd-resolve".into(),
                exe_name: "systemd-resolved".into(),
                cmdline: "/lib/systemd/systemd-resolved".into(),
            },
        ];
        let excluded = detect_excluded_system_pids(&procs, 999, 888);
        assert!(excluded.contains(&200));
        let (pids, comms) = scan_matching_pids(&procs, &expanded, &excluded);
        assert_eq!(pids, vec![100, 101]);
        assert!(comms
            .iter()
            .any(|c| &c.bytes[..12] == b"google chrome" || &c.bytes[..15] == b"google-chrome-s"));
    }
}
