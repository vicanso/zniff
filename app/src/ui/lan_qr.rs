#[allow(unused_imports)]
use super::*;

pub(crate) fn load_listen_all() -> bool {
    saved_config().listen_all
}

pub(crate) fn store_listen_all(on: bool) -> std::io::Result<()> {
    save_config(|config| config.listen_all = on)
}

pub(crate) fn load_lang_pref() -> Option<Lang> {
    saved_config().language.as_deref().and_then(parse_pref)
}

pub(crate) fn store_lang_pref(lang: Option<Lang>) -> std::io::Result<()> {
    save_config(|config| config.language = config::language_name(lang))
}

pub(crate) fn publish_lan(
    listen: &str,
    previous: &str,
    addrs_out: Signal<Vec<LanAddr>>,
    choice: Signal<String>,
    url_out: Signal<String>,
    qr_out: Signal<String>,
) {
    let host_ok = listen
        .rsplit_once(':')
        .is_some_and(|(host, _)| host == "0.0.0.0");
    if !host_ok {
        addrs_out.set(Vec::new());
        url_out.set(String::new());
        qr_out.set(String::new());
        return;
    }
    let addrs = lan_addrs();
    let chosen = choose_lan(&addrs, routed_v4(), previous);
    let ip = chosen.as_ref().map(|addr| addr.ip);
    addrs_out.set(addrs);
    choice.set(ip.map(|ip| ip.to_string()).unwrap_or_default());
    let url = cert_url(listen, ip);
    url_out.set(url.clone());
    qr_out.set(match ip {
        Some(ip) if !url.is_empty() => write_ca_qr(&url, ip).unwrap_or_default(),
        _ => String::new(),
    });
}

pub(crate) fn cert_url(listen: &str, ip: Option<Ipv4Addr>) -> String {
    let Some((host, port)) = listen.rsplit_once(':') else {
        return String::new();
    };
    if host != "0.0.0.0" {
        return String::new();
    }
    let Some(ip) = ip else {
        return String::new();
    };
    let Ok(port) = port.parse::<u16>() else {
        return String::new();
    };
    if port == 0 {
        return String::new();
    }
    format!("http://{ip}:{port}/zniff")
}

/// Keeps `previous` when that address is still up. Otherwise the public
/// route when it is a normal LAN address, then a physical interface ahead
/// of a tunnel. `198.18.0.0/15` is the usual TUN fake-ip range.
pub(crate) fn choose_lan(
    addrs: &[LanAddr],
    routed: Option<Ipv4Addr>,
    previous: &str,
) -> Option<LanAddr> {
    if let Some(found) = addrs.iter().find(|addr| addr.ip.to_string() == previous) {
        return Some(found.clone());
    }
    if let Some(ip) = routed
        && let Some(found) = addrs
            .iter()
            .find(|addr| addr.ip == ip && lan_rank(addr) < 9)
    {
        return Some(found.clone());
    }
    if let Some(found) = addrs
        .iter()
        .filter(|addr| lan_rank(addr) < 9)
        .min_by(|left, right| {
            lan_rank(left)
                .cmp(&lan_rank(right))
                .then(left.name.cmp(&right.name))
                .then(left.ip.octets().cmp(&right.ip.octets()))
        })
    {
        return Some(found.clone());
    }
    addrs.first().cloned().or_else(|| {
        routed
            .filter(|ip| usable_v4(*ip) && !fake_ip(*ip))
            .map(|ip| LanAddr {
                name: String::new(),
                ip,
            })
    })
}

pub(crate) fn lan_rank(addr: &LanAddr) -> u8 {
    if tunnel_name(&addr.name) || fake_ip(addr.ip) {
        return 9;
    }
    let name = addr.name.to_ascii_lowercase();
    if name.starts_with("en")
        || name.starts_with("eth")
        || name.starts_with("wlan")
        || name.starts_with("wl")
    {
        return 0;
    }
    if name.starts_with("bridge") {
        return 2;
    }
    5
}

pub(crate) fn tunnel_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("utun")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
}

pub(crate) fn fake_ip(ip: Ipv4Addr) -> bool {
    let [first, second, _, _] = ip.octets();
    first == 198 && (second == 18 || second == 19)
}

pub(crate) fn usable_v4(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_multicast()
}

/// IPv4 of the interface the OS would use to reach the public internet.
pub(crate) fn routed_v4() -> Option<Ipv4Addr> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("1.1.1.1:80").ok()?;
        match socket.local_addr().ok()?.ip() {
            std::net::IpAddr::V4(ip) if usable_v4(ip) => Some(ip),
            _ => None,
        }
    }
}

pub(crate) fn lan_addrs() -> Vec<LanAddr> {
    let mut addrs = list_v4_addrs();
    addrs.sort_by(|left, right| {
        lan_rank(left)
            .cmp(&lan_rank(right))
            .then(left.name.cmp(&right.name))
            .then(left.ip.octets().cmp(&right.ip.octets()))
    });
    addrs
}

#[cfg(unix)]
pub(crate) fn list_v4_addrs() -> Vec<LanAddr> {
    let mut out = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut cursor = head;
        while !cursor.is_null() {
            let ifa = &*cursor;
            cursor = ifa.ifa_next;
            if ifa.ifa_addr.is_null() {
                continue;
            }
            let flags = ifa.ifa_flags as u32;
            let up = libc::IFF_UP as u32;
            let loopback = libc::IFF_LOOPBACK as u32;
            if flags & up == 0 || flags & loopback != 0 {
                continue;
            }
            if i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
            if !usable_v4(ip) || out.iter().any(|addr| addr.ip == ip) {
                continue;
            }
            let name = if ifa.ifa_name.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(ifa.ifa_name)
                    .to_string_lossy()
                    .into_owned()
            };
            out.push(LanAddr { name, ip });
        }
        libc::freeifaddrs(head);
    }
    out
}

#[cfg(windows)]
pub(crate) fn list_v4_addrs() -> Vec<LanAddr> {
    use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersInfo, IP_ADAPTER_INFO};

    let mut out = Vec::new();
    unsafe {
        let mut size = 16 * 1024u32;
        let mut buf;
        loop {
            buf = vec![0u8; size as usize];
            let result = GetAdaptersInfo(buf.as_mut_ptr() as *mut IP_ADAPTER_INFO, &mut size);
            if result == 0 {
                break;
            }
            if result != ERROR_BUFFER_OVERFLOW || size > 1024 * 1024 {
                return out;
            }
        }
        let mut cursor = buf.as_ptr() as *const IP_ADAPTER_INFO;
        while !cursor.is_null() {
            let adapter = &*cursor;
            let name = c_array_to_string(&adapter.Description);
            let mut address = &adapter.IpAddressList
                as *const windows_sys::Win32::NetworkManagement::IpHelper::IP_ADDR_STRING;
            while !address.is_null() {
                let item = &*address;
                let text = c_array_to_string(&item.IpAddress.String);
                if let Ok(ip) = text.parse::<Ipv4Addr>()
                    && usable_v4(ip)
                    && !out.iter().any(|addr| addr.ip == ip)
                {
                    out.push(LanAddr {
                        name: name.clone(),
                        ip,
                    });
                }
                address = item.Next;
            }
            cursor = adapter.Next;
        }
    }
    out
}

#[cfg(windows)]
pub(crate) fn c_array_to_string(bytes: &[i8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let raw = bytes[..end]
        .iter()
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&raw).into_owned()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn list_v4_addrs() -> Vec<LanAddr> {
    Vec::new()
}

pub(crate) fn write_ca_qr(text: &str, ip: Ipv4Addr) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let modules = code.width();
    let scale = 4u32;
    let quiet = 2u32;
    let side = (modules as u32 + quiet * 2) * scale;
    let mut raw = Vec::with_capacity(((side + 1) * side) as usize);
    for y in 0..side {
        raw.push(0);
        for x in 0..side {
            let mx = x / scale;
            let my = y / scale;
            let inside = mx >= quiet
                && my >= quiet
                && (mx - quiet) < modules as u32
                && (my - quiet) < modules as u32;
            let dark = inside
                && code[((mx - quiet) as usize, (my - quiet) as usize)] == qrcode::Color::Dark;
            raw.push(if dark { 0 } else { 255 });
        }
    }
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut zlib, &raw).ok()?;
    let compressed = zlib.finish().ok()?;
    let png = png_gray(&compressed, side, side);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ip;
        Some(format!(
            "data:image/png;base64,{}",
            session::base64_encode(&png)
        ))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = ca::data_dir().join("preview");
        std::fs::create_dir_all(&dir).ok()?;
        let port = text.rsplit_once(':')?.1.split('/').next().unwrap_or("0");
        let path = dir.join(format!(
            "ca-qr-{}-{port}.png",
            ip.to_string().replace('.', "-")
        ));
        std::fs::write(&path, png).ok()?;
        Some(path.to_string_lossy().into_owned())
    }
}

pub(crate) fn png_gray(zlib_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    out.extend(present::png_chunk(b"IHDR", &ihdr));
    out.extend(present::png_chunk(b"IDAT", zlib_data));
    out.extend(present::png_chunk(b"IEND", &[]));
    out
}

pub(crate) fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        let mb = bytes as f64 / (1024.0 * 1024.0);
        if mb >= 99.95 {
            format!("{mb:.0} MB")
        } else {
            format!("{mb:.1} MB")
        }
    } else if bytes >= 1024 {
        let kb = bytes as f64 / 1024.0;
        if kb >= 99.95 {
            format!("{kb:.0} KB")
        } else {
            format!("{kb:.1} KB")
        }
    } else {
        format!("{bytes} B")
    }
}
