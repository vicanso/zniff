#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_socket_cookie, generated},
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
fn load_active_config() -> Option<&'static Config> {
    let cfg = CONFIG.get(0)?;
    if cfg.enabled == 0 {
        None
    } else {
        Some(cfg)
    }
}

#[inline(always)]
fn read_current_comm_lowercased(out: &mut CommKey) {
    let mut raw = [0u8; 16];
    let ret = unsafe { generated::bpf_get_current_comm(raw.as_mut_ptr().cast(), 16) };
    if ret == 0 {
        out.init_from_comm(&raw);
    } else {
        out.bytes = [0; 16];
    }
}

#[inline(always)]
fn should_intercept_current(cfg: &Config, comm_key: &mut CommKey) -> u32 {
    let tgid = (bpf_get_current_pid_tgid() >> 32) as u32;
    if tgid == 0 || tgid == cfg.zniff_pid || tgid == cfg.helper_pid {
        return 0;
    }
    if unsafe { EXCLUDED_PIDS.get(&tgid) }.is_some() {
        return 0;
    }
    read_current_comm_lowercased(comm_key);
    if cfg.capture_all != 0
        || unsafe { TARGET_PIDS.get(&tgid) }.is_some()
        || unsafe { TARGET_COMMS.get(comm_key) }.is_some()
    {
        tgid
    } else {
        0
    }
}

#[inline(always)]
fn write_ipv6_octets(words: &[u32; 4], out: &mut [u8; 16]) {
    let b0 = words[0].to_ne_bytes();
    let b1 = words[1].to_ne_bytes();
    let b2 = words[2].to_ne_bytes();
    let b3 = words[3].to_ne_bytes();
    out[0] = b0[0];
    out[1] = b0[1];
    out[2] = b0[2];
    out[3] = b0[3];
    out[4] = b1[0];
    out[5] = b1[1];
    out[6] = b1[2];
    out[7] = b1[3];
    out[8] = b2[0];
    out[9] = b2[1];
    out[10] = b2[2];
    out[11] = b2[3];
    out[12] = b3[0];
    out[13] = b3[1];
    out[14] = b3[2];
    out[15] = b3[3];
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
    let mut comm_key = CommKey { bytes: [0; 16] };
    let tgid = should_intercept_current(cfg, &mut comm_key);
    if tgid == 0 {
        return 1;
    }
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
fn try_connect6(ctx: &SockAddrContext) -> i32 {
    let Some(cfg) = load_active_config() else {
        return 1;
    };
    let sa = unsafe { &mut *ctx.sock_addr };
    let dst_port = u16::from_be(sa.user_port as u16);
    if dst_port == 0 || dst_port == 53 {
        return 1;
    }
    let mut dst_addr = [0u8; 16];
    write_ipv6_octets(&sa.user_ip6, &mut dst_addr);
    if is_bypassed_ipv6(&dst_addr) {
        return 1;
    }
    let mut comm_key = CommKey { bytes: [0; 16] };
    let tgid = should_intercept_current(cfg, &mut comm_key);
    if tgid == 0 {
        return 1;
    }
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
    let mut comm_key = CommKey { bytes: [0; 16] };
    if should_intercept_current(cfg, &mut comm_key) != 0 {
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
    if dst_port != 443 {
        return 1;
    }
    let mut dst_addr = [0u8; 16];
    write_ipv6_octets(&sa.user_ip6, &mut dst_addr);
    if is_bypassed_ipv6(&dst_addr) {
        return 1;
    }
    let mut comm_key = CommKey { bytes: [0; 16] };
    if should_intercept_current(cfg, &mut comm_key) != 0 {
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
    let Some(orig_ref) = (unsafe { ORIG_DST_BY_COOKIE.get(&cookie) }) else {
        return 1;
    };
    let orig = *orig_ref;
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
    let tgid = (bpf_get_current_pid_tgid() >> 32) as u32;
    if tgid == 0 || tgid == cfg.zniff_pid || tgid == cfg.helper_pid {
        return 0;
    }
    if unsafe { EXCLUDED_PIDS.get(&tgid) }.is_some() {
        return 0;
    }
    let mut key = CommKey { bytes: [0; 16] };
    read_current_comm_lowercased(&mut key);
    if unsafe { TARGET_COMMS.get(&key) }.is_some() {
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
    let parent_tgid = (bpf_get_current_pid_tgid() >> 32) as u32;
    if unsafe { TARGET_PIDS.get(&parent_tgid) }.is_none() {
        return 0;
    }
    let mut child_pid: i32 = 0;
    let ret = unsafe {
        generated::bpf_probe_read_kernel(
            (&mut child_pid as *mut i32).cast(),
            core::mem::size_of::<i32>() as u32,
            ctx.as_ptr()
                .cast::<u8>()
                .add(cfg.fork_child_pid_offset as usize)
                .cast(),
        )
    };
    if ret == 0 && child_pid > 0 {
        let pid_u32 = child_pid as u32;
        let _ = TARGET_PIDS.insert(&pid_u32, &1u8, 0);
    }
    0
}

#[tracepoint]
pub fn zniff_sched_exit(_ctx: TracePointContext) -> u32 {
    let pid_tgid = bpf_get_current_pid_tgid();
    let tgid = (pid_tgid >> 32) as u32;
    let tid = pid_tgid as u32;
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
