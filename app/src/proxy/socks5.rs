use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, lookup_host};

/// Runs RFC 1928 CONNECT (and optional RFC 1929 username/password authentication)
/// over an already connected socket to the SOCKS5 server.
pub(super) async fn socks5_handshake(
    stream: &mut TcpStream,
    target_host: &str,
    target_port: u16,
    remote_dns: bool,
    auth: Option<&(String, String)>,
) -> Result<()> {
    negotiate_method(stream, auth).await?;
    let request = build_connect_request(target_host, target_port, remote_dns).await?;
    stream
        .write_all(&request)
        .await
        .context("发送 SOCKS5 CONNECT 请求失败")?;

    let mut head = [0_u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .context("读取 SOCKS5 CONNECT 响应头失败")?;
    if head[0] != 0x05 {
        bail!("SOCKS5 CONNECT 响应版本无效: {:#x}", head[0]);
    }
    if head[1] != 0x00 {
        bail!("SOCKS5 代理拒绝连接: {}", rep_message(head[1]));
    }
    drain_bound_address(stream, head[3]).await
}

async fn negotiate_method(stream: &mut TcpStream, auth: Option<&(String, String)>) -> Result<()> {
    let greeting: &[u8] = if auth.is_some() {
        &[0x05, 0x02, 0x00, 0x02]
    } else {
        &[0x05, 0x01, 0x00]
    };
    stream
        .write_all(greeting)
        .await
        .context("发送 SOCKS5 握手问候失败")?;

    let mut choice = [0_u8; 2];
    stream
        .read_exact(&mut choice)
        .await
        .context("读取 SOCKS5 握手响应失败")?;
    if choice[0] != 0x05 {
        bail!("SOCKS5 握手协议版本无效: {:#x}", choice[0]);
    }
    match choice[1] {
        0x00 => Ok(()),
        0x02 => {
            let Some((user, pass)) = auth else {
                bail!("SOCKS5 代理要求用户名和密码认证");
            };
            send_username_password(stream, user, pass).await
        }
        0xFF => bail!("SOCKS5 代理没有可接受的认证方式"),
        other => bail!("不支持的 SOCKS5 认证方式: {other:#x}"),
    }
}

async fn send_username_password(stream: &mut TcpStream, user: &str, pass: &str) -> Result<()> {
    if user.len() > 255 || pass.len() > 255 {
        bail!("SOCKS5 用户名或密码超过 255 字节");
    }
    let mut buf = Vec::with_capacity(3 + user.len() + pass.len());
    buf.push(0x01);
    buf.push(user.len() as u8);
    buf.extend_from_slice(user.as_bytes());
    buf.push(pass.len() as u8);
    buf.extend_from_slice(pass.as_bytes());
    stream
        .write_all(&buf)
        .await
        .context("发送 SOCKS5 认证凭据失败")?;

    let mut reply = [0_u8; 2];
    stream
        .read_exact(&mut reply)
        .await
        .context("读取 SOCKS5 认证响应失败")?;
    if reply[0] != 0x01 || reply[1] != 0x00 {
        bail!("SOCKS5 用户名或密码认证失败");
    }
    Ok(())
}

async fn build_connect_request(
    target_host: &str,
    target_port: u16,
    remote_dns: bool,
) -> Result<Vec<u8>> {
    let clean = target_host.trim().trim_matches(|ch| ch == '[' || ch == ']');
    let mut out = Vec::with_capacity(7 + clean.len());
    out.extend_from_slice(&[0x05, 0x01, 0x00]);
    if let Ok(ip) = clean.parse::<std::net::IpAddr>() {
        push_ip_addr(&mut out, ip);
    } else if remote_dns {
        if clean.is_empty() || clean.len() > 255 {
            bail!("SOCKS5 目标域名长度无效");
        }
        out.push(0x03);
        out.push(clean.len() as u8);
        out.extend_from_slice(clean.as_bytes());
    } else {
        let addr = lookup_host((clean, target_port))
            .await
            .context("SOCKS5 本地解析目标域名失败")?
            .next()
            .ok_or_else(|| anyhow!("SOCKS5 未解析到目标地址"))?;
        push_ip_addr(&mut out, addr.ip());
    }
    out.extend_from_slice(&target_port.to_be_bytes());
    Ok(out)
}

fn push_ip_addr(buf: &mut Vec<u8>, ip: std::net::IpAddr) {
    match ip {
        std::net::IpAddr::V4(v4) => {
            buf.push(0x01);
            buf.extend_from_slice(&v4.octets());
        }
        std::net::IpAddr::V6(v6) => {
            buf.push(0x04);
            buf.extend_from_slice(&v6.octets());
        }
    }
}

async fn drain_bound_address(stream: &mut TcpStream, atyp: u8) -> Result<()> {
    match atyp {
        0x01 => {
            let mut tail = [0_u8; 6];
            stream
                .read_exact(&mut tail)
                .await
                .context("读取 SOCKS5 IPv4 绑定地址失败")?;
        }
        0x04 => {
            let mut tail = [0_u8; 18];
            stream
                .read_exact(&mut tail)
                .await
                .context("读取 SOCKS5 IPv6 绑定地址失败")?;
        }
        0x03 => {
            let mut len = [0_u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .context("读取 SOCKS5 域名长度失败")?;
            let mut tail = vec![0_u8; len[0] as usize + 2];
            stream
                .read_exact(&mut tail)
                .await
                .context("读取 SOCKS5 域名绑定地址失败")?;
        }
        other => bail!("SOCKS5 响应地址类型无效: {other:#x}"),
    }
    Ok(())
}

fn rep_message(rep: u8) -> &'static str {
    match rep {
        0x01 => "SOCKS 服务器一般性故障",
        0x02 => "规则禁止连接",
        0x03 => "网络不可达",
        0x04 => "主机不可达",
        0x05 => "连接被拒绝",
        0x06 => "TTL 已过期",
        0x07 => "不支持的命令",
        0x08 => "不支持的地址类型",
        _ => "未知错误",
    }
}
