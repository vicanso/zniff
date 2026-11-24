#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_comm, bpf_get_current_pid_tgid, bpf_get_socket_cookie},
    macros::{cgroup_sock_addr, map, sock_ops, tracepoint},
    maps::{Array, HashMap, LruHashMap},
    programs::{SockAddrContext, SockOpsContext, TracePointContext},
    EbpfContext,
};
use zniff_ebpf_common::{
    is_bypassed_ipv4, is_bypassed_ipv6, CommKey, Config, OrigDst, SrcPortKey, AF_INET, AF_INET6,
    BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB, IPPROTO_TCP, IPPROTO_UDP,
};

#[map]
static CONFIG: Array<Config> = Array::with_max_entries(1, 0);

#[map]
static EXCLUDED_PIDS: HashMap<u32, u8> = HashMap::with_max_entries(1024, 0);

#[map]
static TARGET_PIDS: LruHashMap<u32, u8> = LruHashMap::with_max_entries(16_384, 0);

#[map]
static TARGET_COMMS: HashMap<CommKey, u8> = HashMap::with_max_entries(512, 0);

#[map]
static ORIG_DST_BY_COOKIE: LruHashMap<u64, OrigDst> = LruHashMap::with_max_entries(16_384, 0);

#[map]
static ORIG_DST_BY_SRC_PORT: LruHashMap<SrcPortKey, OrigDst> =
    LruHashMap::with_max_entries(16_384, 0);

#[inline(always)]
fn current_tgid_and_tid() -> (u32, u32) {
    let pid_tgid = bpf_get_current_pid_tgid();
    ((pid_tgid >> 32) as u32, pid_tgid as u32)
}

#[inline(always)]
fn load_active_config() -> Option<Config> {
    let ptr = CONFIG.get_ptr(0)?;
    let cfg = unsafe { *ptr };
    if cfg.enabled == 0 {
        None
    } else {
        Some(cfg)
    }
}

#[inline(always)]
fn should_intercept_current(cfg: &Config) -> Option<(u32, CommKey)> {
    let (tgid, _) = current_tgid_and_tid();
    if tgid == 0 || tgid == cfg.zniff_pid || tgid == cfg.helper_pid {
        return None;
    }
    if EXCLUDED_PIDS.get_ptr(&tgid).is_some() {
        return None;
    }
    let raw_comm = bpf_get_current_comm().unwrap_or([0; 16]);
    let comm_key = CommKey::from_bytes_lowercased(&raw_comm);
    if cfg.capture_all != 0
        || TARGET_PIDS.get_ptr(&tgid).is_some()
        || TARGET_COMMS.get_ptr(&comm_key).is_some()
    {
        Some((tgid, comm_key))
    } else {
        None
    }
}

#[cgroup_sock_addr(connect4)]
pub fn zniff_connect4(ctx: SockAddrContext) -> i32 {
    try_connect4(&ctx)
}

#[inline(always)]
fn try_connect4(ctx: &SockAddrContext) -> i32 {
    let Some(cfg) = load_active_config() else {
        return 1;
    };
    let sa = unsafe { &mut *ctx.sock_addr };
    let dst_port = u16::from_be(sa.user_port as u16);
    if dst_port == 0 || dst_port == 53 {
        return 1;
    }
    let ip_host = u32::from_be(sa.user_ip4);
    if is_bypassed_ipv4(ip_host) {
        return 1;
    }
    let Some((tgid, comm_key)) = should_intercept_current(&cfg) else {
        return 1;
    };
    if sa.protocol == IPPROTO_UDP {
        if cfg.block_quic != 0 && dst_port == 443 {
            return 0;
        }
        return 1;
    }
    if sa.protocol != IPPROTO_TCP || cfg.bridge_port_v4_be == 0 {
        return 1;
    }

    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    if cookie == 0 {
        return 1;
    }
    let mut dst_addr = [0u8; 16];
    let ip_bytes = sa.user_ip4.to_ne_bytes();
    dst_addr[0] = ip_bytes[0];
    dst_addr[1] = ip_bytes[1];
    dst_addr[2] = ip_bytes[2];
    dst_addr[3] = ip_bytes[3];

    let orig = OrigDst {
        family: AF_INET,
        dst_port,
        _pad: 0,
        dst_addr,
        pid: tgid,
        comm: comm_key.bytes,
    };
    if ORIG_DST_BY_COOKIE.insert(&cookie, &orig, 0).is_ok() {
        sa.user_ip4 = u32::to_be(0x7f00_0001);
        sa.user_port = u32::from(cfg.bridge_port_v4_be);
    }
    1
}

#[cgroup_sock_addr(connect6)]
pub fn zniff_connect6(ctx: SockAddrContext) -> i32 {
    try_connect6(&ctx)
}

#[inline(always)]
fn ipv6_words_to_octets(words: [u32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    let b0 = words[0].to_ne_bytes();
    let b1 = words[1].to_ne_bytes();
    let b2 = words[2].to_ne_bytes();
    let b3 = words[3].to_ne_bytes();
    out[0..4].copy_from_slice(&b0);
    out[4..8].copy_from_slice(&b1);
    out[8..12].copy_from_slice(&b2);
    out[12..16].copy_from_slice(&b3);
    out
}

#[inline(always)]
fn try_connect6(ctx: &SockAddrContext) -> i32 {
    let Some(cfg) = load_active_config() else {
        return 1;
    };
    let sa = unsafe { &mut *ctx.sock_addr };
    let dst_port = u16::from_be(sa.user_port as u16);
    if dst_port == 0 || dst_port == 53 {
        return 1;
    }
    let dst_addr = ipv6_words_to_octets(sa.user_ip6);
    if is_bypassed_ipv6(dst_addr) {
        return 1;
    }
    let Some((tgid, comm_key)) = should_intercept_current(&cfg) else {
        return 1;
    };
    if sa.protocol == IPPROTO_UDP {
        if cfg.block_quic != 0 && dst_port == 443 {
            return 0;
        }
        return 1;
    }
    if sa.protocol != IPPROTO_TCP || cfg.bridge_port_v6_be == 0 {
        return 1;
    }

    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    if cookie == 0 {
        return 1;
    }
    let orig = OrigDst {
        family: AF_INET6,
        dst_port,
        _pad: 0,
        dst_addr,
        pid: tgid,
        comm: comm_key.bytes,
    };
    if ORIG_DST_BY_COOKIE.insert(&cookie, &orig, 0).is_ok() {
        sa.user_ip6 = [0, 0, 0, u32::to_be(1)];
        sa.user_port = u32::from(cfg.bridge_port_v6_be);
    }
    1
}

#[cgroup_sock_addr(sendmsg4)]
pub fn zniff_sendmsg4(ctx: SockAddrContext) -> i32 {
    let Some(cfg) = load_active_config() else {
        return 1;
    };
    if cfg.block_quic == 0 {
        return 1;
    }
    let sa = unsafe { &*ctx.sock_addr };
    let dst_port = u16::from_be(sa.user_port as u16);
    if dst_port != 443 || is_bypassed_ipv4(u32::from_be(sa.user_ip4)) {
        return 1;
    }
    if should_intercept_current(&cfg).is_some() {
        0
    } else {
        1
    }
}

#[cgroup_sock_addr(sendmsg6)]
pub fn zniff_sendmsg6(ctx: SockAddrContext) -> i32 {
    let Some(cfg) = load_active_config() else {
        return 1;
    };
    if cfg.block_quic == 0 {
        return 1;
    }
    let sa = unsafe { &*ctx.sock_addr };
    let dst_port = u16::from_be(sa.user_port as u16);
    if dst_port != 443 || is_bypassed_ipv6(ipv6_words_to_octets(sa.user_ip6)) {
        return 1;
    }
    if should_intercept_current(&cfg).is_some() {
        0
    } else {
        1
    }
}

#[sock_ops]
pub fn zniff_sockops(ctx: SockOpsContext) -> u32 {
    if ctx.op() != BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB {
        return 1;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    if cookie == 0 {
        return 1;
    }
    let Some(orig_ptr) = ORIG_DST_BY_COOKIE.get_ptr(&cookie) else {
        return 1;
    };
    let orig = unsafe { *orig_ptr };
    let _ = ORIG_DST_BY_COOKIE.remove(&cookie);

    let key = SrcPortKey {
        family: ctx.family(),
        src_port: ctx.local_port() as u16,
        _pad: 0,
    };
    let _ = ORIG_DST_BY_SRC_PORT.insert(&key, &orig, 0);
    1
}

#[tracepoint]
pub fn zniff_sched_exec(_ctx: TracePointContext) -> u32 {
    let Some(cfg) = load_active_config() else {
        return 0;
    };
    let (tgid, _) = current_tgid_and_tid();
    if tgid == 0 || tgid == cfg.zniff_pid || tgid == cfg.helper_pid {
        return 0;
    }
    if EXCLUDED_PIDS.get_ptr(&tgid).is_some() {
        return 0;
    }
    let raw_comm = bpf_get_current_comm().unwrap_or([0; 16]);
    let key = CommKey::from_bytes_lowercased(&raw_comm);
    if TARGET_COMMS.get_ptr(&key).is_some() {
        let _ = TARGET_PIDS.insert(&tgid, &1u8, 0);
    }
    0
}

#[tracepoint]
pub fn zniff_sched_fork(ctx: TracePointContext) -> u32 {
    let Some(cfg) = load_active_config() else {
        return 0;
    };
    if cfg.fork_child_pid_offset == 0 {
        return 0;
    }
    let (parent_tgid, _) = current_tgid_and_tid();
    if TARGET_PIDS.get_ptr(&parent_tgid).is_none() {
        return 0;
    }
    if let Ok(child_pid) = unsafe { ctx.read_at::<i32>(cfg.fork_child_pid_offset as usize) } {
        if child_pid > 0 {
            let pid_u32 = child_pid as u32;
            let _ = TARGET_PIDS.insert(&pid_u32, &1u8, 0);
        }
    }
    0
}

#[tracepoint]
pub fn zniff_sched_exit(_ctx: TracePointContext) -> u32 {
    let (tgid, tid) = current_tgid_and_tid();
    if tgid != 0 && tgid == tid {
        let _ = TARGET_PIDS.remove(&tgid);
        let _ = EXCLUDED_PIDS.remove(&tgid);
    }
    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
