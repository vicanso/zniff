#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context;
#[cfg(target_os = "linux")]
use aya::{
    include_bytes_aligned,
    maps::{Array, HashMap},
    programs::{CgroupAttachMode, CgroupSockAddr, SockOps, TracePoint},
    Ebpf,
};
#[cfg(target_os = "linux")]
use zniff_ebpf_common::{CommKey, Config, OrigDst, SrcPortKey};

#[cfg(target_os = "linux")]
use crate::proc_scan::{
    detect_excluded_system_pids, detect_fork_child_pid_offset, expand_targets_with_desktop,
    list_proc_entries, load_desktop_map, resolve_display_label, scan_matching_pids, DesktopMap,
};
#[cfg(target_os = "linux")]
use crate::protocol::HelperConfig;

#[cfg(target_os = "linux")]
static EBPF_BYTES: &[u8] = include_bytes_aligned!(concat!(env!("OUT_DIR"), "/zniff-ebpf-prog"));

#[cfg(target_os = "linux")]
pub struct EbpfController {
    ebpf: Ebpf,
    desktop_map: DesktopMap,
    fork_child_pid_offset: u32,
    applied_pids: Vec<u32>,
    applied_comms: Vec<CommKey>,
    applied_excluded: Vec<u32>,
}

#[cfg(target_os = "linux")]
impl EbpfController {
    pub fn load_and_attach() -> anyhow::Result<Self> {
        if EBPF_BYTES.is_empty() {
            anyhow::bail!("zniff-ebpf-helper was built without embedded eBPF bytecode");
        }
        if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
            anyhow::bail!("cgroup v2 is required at /sys/fs/cgroup");
        }

        let mut ebpf = Ebpf::load(EBPF_BYTES).context("failed to parse embedded eBPF ELF")?;

        attach_cgroup_sock_addr(&mut ebpf, "zniff_connect4")?;
        attach_cgroup_sock_addr(&mut ebpf, "zniff_connect6")?;
        attach_cgroup_sock_addr(&mut ebpf, "zniff_sendmsg4")?;
        attach_cgroup_sock_addr(&mut ebpf, "zniff_sendmsg6")?;
        attach_sockops(&mut ebpf, "zniff_sockops")?;

        attach_tracepoint(&mut ebpf, "zniff_sched_exec", "sched", "sched_process_exec")?;
        attach_tracepoint(&mut ebpf, "zniff_sched_exit", "sched", "sched_process_exit")?;
        let fork_child_pid_offset = match detect_fork_child_pid_offset() {
            Some(off) if off > 0 => {
                if attach_tracepoint(&mut ebpf, "zniff_sched_fork", "sched", "sched_process_fork")
                    .is_ok()
                {
                    off
                } else {
                    0
                }
            }
            _ => 0,
        };

        Ok(Self {
            ebpf,
            desktop_map: load_desktop_map(),
            fork_child_pid_offset,
            applied_pids: Vec::new(),
            applied_comms: Vec::new(),
            applied_excluded: Vec::new(),
        })
    }

    pub fn apply_config(
        &mut self,
        cfg: &HelperConfig,
        bridge_port_v4: u16,
        bridge_port_v6: u16,
    ) -> anyhow::Result<()> {
        self.desktop_map = load_desktop_map();
        let procs = list_proc_entries();
        let self_pid = std::process::id();
        let excluded = detect_excluded_system_pids(&procs, self_pid, cfg.zniff_pid);
        self.sync_excluded_pids(&excluded)?;

        let expanded = expand_targets_with_desktop(&cfg.targets, &self.desktop_map);
        let (matched_pids, target_comms) = if cfg.capture_all {
            (Vec::new(), Vec::new())
        } else {
            scan_matching_pids(&procs, &expanded, &excluded)
        };
        self.sync_target_comms(&target_comms)?;
        self.sync_target_pids(&matched_pids)?;

        let bpf_cfg = Config {
            bridge_port_v4_be: bridge_port_v4.to_be(),
            bridge_port_v6_be: bridge_port_v6.to_be(),
            capture_all: u8::from(cfg.capture_all),
            block_quic: u8::from(cfg.block_quic),
            enabled: 1,
            _pad: 0,
            zniff_pid: cfg.zniff_pid,
            helper_pid: self_pid,
            fork_child_pid_offset: self.fork_child_pid_offset,
        };
        let mut config_map: Array<_, Config> = Array::try_from(
            self.ebpf
                .map_mut("CONFIG")
                .context("missing CONFIG eBPF map")?,
        )?;
        config_map.set(0, &bpf_cfg, 0)?;
        Ok(())
    }

    pub fn refresh_proc_pids(&mut self, cfg: &HelperConfig) -> anyhow::Result<()> {
        let procs = list_proc_entries();
        let excluded = detect_excluded_system_pids(&procs, std::process::id(), cfg.zniff_pid);
        self.sync_excluded_pids(&excluded)?;
        if !cfg.capture_all {
            let expanded = expand_targets_with_desktop(&cfg.targets, &self.desktop_map);
            let (matched_pids, _) = scan_matching_pids(&procs, &expanded, &excluded);
            self.sync_target_pids(&matched_pids)?;
        }
        Ok(())
    }

    pub fn lookup_and_remove_src_port(&mut self, family: u32, src_port: u16) -> Option<OrigDst> {
        let map = self.ebpf.map_mut("ORIG_DST_BY_SRC_PORT")?;
        let mut hash: HashMap<_, SrcPortKey, OrigDst> = HashMap::try_from(map).ok()?;
        let key = SrcPortKey {
            family,
            src_port,
            _pad: 0,
        };
        let orig = hash.get(&key, 0).ok()?;
        let _ = hash.remove(&key);
        Some(orig)
    }

    pub fn resolve_label(&self, pid: u32, comm: &[u8; 16]) -> String {
        resolve_display_label(pid, comm, &self.desktop_map)
    }

    pub fn disable(&mut self) {
        if let Some(map) = self.ebpf.map_mut("CONFIG") {
            if let Ok(mut config_map) = Array::<_, Config>::try_from(map) {
                let _ = config_map.set(0, &Config::default(), 0);
            }
        }
    }

    fn sync_excluded_pids(&mut self, next: &[u32]) -> anyhow::Result<()> {
        let map = self
            .ebpf
            .map_mut("EXCLUDED_PIDS")
            .context("missing EXCLUDED_PIDS map")?;
        let mut hash: HashMap<_, u32, u8> = HashMap::try_from(map)?;
        for old in &self.applied_excluded {
            if !next.contains(old) {
                let _ = hash.remove(old);
            }
        }
        for pid in next {
            hash.insert(pid, &1u8, 0)?;
        }
        self.applied_excluded = next.to_vec();
        Ok(())
    }

    fn sync_target_comms(&mut self, next: &[CommKey]) -> anyhow::Result<()> {
        let map = self
            .ebpf
            .map_mut("TARGET_COMMS")
            .context("missing TARGET_COMMS map")?;
        let mut hash: HashMap<_, CommKey, u8> = HashMap::try_from(map)?;
        for old in &self.applied_comms {
            if !next.contains(old) {
                let _ = hash.remove(old);
            }
        }
        for comm in next {
            hash.insert(comm, &1u8, 0)?;
        }
        self.applied_comms = next.to_vec();
        Ok(())
    }

    fn sync_target_pids(&mut self, next: &[u32]) -> anyhow::Result<()> {
        let map = self
            .ebpf
            .map_mut("TARGET_PIDS")
            .context("missing TARGET_PIDS map")?;
        let mut hash: HashMap<_, u32, u8> = HashMap::try_from(map)?;
        for old in &self.applied_pids {
            if !next.contains(old) {
                let _ = hash.remove(old);
            }
        }
        for pid in next {
            hash.insert(pid, &1u8, 0)?;
        }
        self.applied_pids = next.to_vec();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn attach_cgroup_sock_addr(ebpf: &mut Ebpf, name: &str) -> anyhow::Result<()> {
    let prog: &mut CgroupSockAddr = ebpf
        .program_mut(name)
        .with_context(|| format!("missing eBPF program {name}"))?
        .try_into()?;
    prog.load().with_context(|| format!("load {name}"))?;
    let cgroup = File::open("/sys/fs/cgroup").context("open /sys/fs/cgroup")?;
    if prog
        .attach(cgroup, CgroupAttachMode::AllowMultiple)
        .is_err()
    {
        let cgroup = File::open("/sys/fs/cgroup").context("reopen /sys/fs/cgroup")?;
        prog.attach(cgroup, CgroupAttachMode::Single)
            .with_context(|| format!("attach {name}"))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn attach_sockops(ebpf: &mut Ebpf, name: &str) -> anyhow::Result<()> {
    let prog: &mut SockOps = ebpf
        .program_mut(name)
        .with_context(|| format!("missing eBPF program {name}"))?
        .try_into()?;
    prog.load().with_context(|| format!("load {name}"))?;
    let cgroup = File::open("/sys/fs/cgroup").context("open /sys/fs/cgroup")?;
    if prog
        .attach(cgroup, CgroupAttachMode::AllowMultiple)
        .is_err()
    {
        let cgroup = File::open("/sys/fs/cgroup").context("reopen /sys/fs/cgroup")?;
        prog.attach(cgroup, CgroupAttachMode::Single)
            .with_context(|| format!("attach {name}"))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn attach_tracepoint(
    ebpf: &mut Ebpf,
    name: &str,
    category: &str,
    event: &str,
) -> anyhow::Result<()> {
    let prog: &mut TracePoint = ebpf
        .program_mut(name)
        .with_context(|| format!("missing tracepoint {name}"))?
        .try_into()?;
    prog.load().with_context(|| format!("load {name}"))?;
    prog.attach(category, event)
        .with_context(|| format!("attach {category}:{event}"))?;
    Ok(())
}
