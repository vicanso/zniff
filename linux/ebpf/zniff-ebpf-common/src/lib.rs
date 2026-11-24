#![cfg_attr(not(feature = "user"), no_std)]

pub const AF_INET: u32 = 2;
pub const AF_INET6: u32 = 10;
pub const IPPROTO_TCP: u32 = 6;
pub const IPPROTO_UDP: u32 = 17;
pub const BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub bridge_port_v4_be: u16,
    pub bridge_port_v6_be: u16,
    pub capture_all: u8,
    pub block_quic: u8,
    pub enabled: u8,
    pub _pad: u8,
    pub zniff_pid: u32,
    pub helper_pid: u32,
    pub fork_child_pid_offset: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CommKey {
    pub bytes: [u8; 16],
}

impl CommKey {
    pub fn from_bytes_lowercased(raw: &[u8]) -> Self {
        let mut bytes = [0u8; 16];
        for (idx, &b) in raw.iter().take(15).enumerate() {
            if b == 0 {
                break;
            }
            bytes[idx] = b.to_ascii_lowercase();
        }
        Self { bytes }
    }

    pub fn from_str_lowercased(s: &str) -> Self {
        Self::from_bytes_lowercased(s.trim().as_bytes())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrigDst {
    pub family: u32,
    pub dst_port: u16,
    pub _pad: u16,
    pub dst_addr: [u8; 16],
    pub pid: u32,
    pub comm: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SrcPortKey {
    pub family: u32,
    pub src_port: u16,
    pub _pad: u16,
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for Config {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for CommKey {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for OrigDst {}
#[cfg(feature = "user")]
unsafe impl aya::Pod for SrcPortKey {}

/// Returns `true` for non-routable / loopback / link-local / multicast IPv4 targets
/// that transparent proxying must never hijack (host-byte-order `ip`).
pub const fn is_bypassed_ipv4(ip_host_order: u32) -> bool {
    let first = (ip_host_order >> 24) as u8;
    let second = ((ip_host_order >> 16) & 0xff) as u8;
    // 0.0.0.0/8 (unspecified), 127.0.0.0/8 (loopback),
    // 169.254.0.0/16 (link-local), 224.0.0.0/4 (multicast & reserved 240.0.0.0/4)
    first == 0 || first == 127 || (first == 169 && second == 254) || first >= 224
}

/// Returns `true` for unspecified (`::`), loopback (`::1`), link-local (`fe80::/10`),
/// multicast (`ff00::/8`), or bypassed IPv4-mapped (`::ffff:a.b.c.d`) addresses.
pub const fn is_bypassed_ipv6(octets: [u8; 16]) -> bool {
    let is_zero_prefix = octets[0] == 0
        && octets[1] == 0
        && octets[2] == 0
        && octets[3] == 0
        && octets[4] == 0
        && octets[5] == 0
        && octets[6] == 0
        && octets[7] == 0
        && octets[8] == 0
        && octets[9] == 0;

    if is_zero_prefix && octets[10] == 0 && octets[11] == 0 {
        let tail = u32::from_be_bytes([octets[12], octets[13], octets[14], octets[15]]);
        return tail == 0 || tail == 1;
    }

    // IPv4-mapped IPv6 ::ffff:0:0/96
    if is_zero_prefix && octets[10] == 0xff && octets[11] == 0xff {
        let ipv4 = u32::from_be_bytes([octets[12], octets[13], octets[14], octets[15]]);
        return is_bypassed_ipv4(ipv4);
    }

    // Link-local fe80::/10 or multicast ff00::/8
    (octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80) || octets[0] == 0xff
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    #[test]
    fn shared_structs_have_expected_repr_c_sizes() {
        assert_eq!(size_of::<Config>(), 20);
        assert_eq!(align_of::<Config>(), 4);
        assert_eq!(size_of::<CommKey>(), 16);
        assert_eq!(size_of::<OrigDst>(), 44);
        assert_eq!(size_of::<SrcPortKey>(), 8);
    }

    #[test]
    fn comm_key_lowercases_and_truncates_at_15_bytes() {
        let key = CommKey::from_str_lowercased("Chrome_IOThreadExtra");
        assert_eq!(&key.bytes[..15], b"chrome_iothread");
        assert_eq!(key.bytes[15], 0);

        let nul_key = CommKey::from_bytes_lowercased(b"Curl\0garbage");
        assert_eq!(&nul_key.bytes[..4], b"curl");
        assert_eq!(nul_key.bytes[4], 0);
    }

    #[test]
    fn bypassed_ipv4_covers_loopback_link_local_multicast() {
        assert!(is_bypassed_ipv4(u32::from_be_bytes([127, 0, 0, 1])));
        assert!(is_bypassed_ipv4(u32::from_be_bytes([0, 0, 0, 0])));
        assert!(is_bypassed_ipv4(u32::from_be_bytes([169, 254, 10, 1])));
        assert!(is_bypassed_ipv4(u32::from_be_bytes([224, 0, 0, 251])));
        assert!(is_bypassed_ipv4(u32::from_be_bytes([255, 255, 255, 255])));
        assert!(!is_bypassed_ipv4(u32::from_be_bytes([1, 1, 1, 1])));
        assert!(!is_bypassed_ipv4(u32::from_be_bytes([192, 168, 1, 10])));
    }

    #[test]
    fn bypassed_ipv6_covers_loopback_link_local_multicast_and_v4_mapped() {
        assert!(is_bypassed_ipv6([0; 16]));
        let mut loopback = [0u8; 16];
        loopback[15] = 1;
        assert!(is_bypassed_ipv6(loopback));
        assert!(is_bypassed_ipv6([
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
        assert!(is_bypassed_ipv6([
            0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
        assert!(is_bypassed_ipv6([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 127, 0, 0, 1
        ]));
        assert!(!is_bypassed_ipv6([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 8, 8, 8, 8
        ]));
        assert!(!is_bypassed_ipv6([
            0x26, 0x06, 0x47, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11
        ]));
    }
}
