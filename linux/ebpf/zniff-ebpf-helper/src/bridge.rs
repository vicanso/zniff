use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use zniff_ebpf_common::{OrigDst, AF_INET, AF_INET6};

pub type LookupFn = Arc<dyn Fn(u32, u16) -> Option<OrigDst> + Send + Sync>;
pub type LabelFn = Arc<dyn Fn(u32, &[u8; 16]) -> String + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct BridgeRuntimeConfig {
    pub proxy_port: u16,
    pub proxy_auth: String,
}

pub struct BridgeServer {
    pub port_v4: u16,
    pub port_v6: u16,
    shutdown_tx: watch::Sender<bool>,
}

impl BridgeServer {
    pub async fn start(
        runtime_cfg: Arc<RwLock<BridgeRuntimeConfig>>,
        lookup: LookupFn,
        label_resolver: LabelFn,
    ) -> anyhow::Result<Self> {
        let listener_v4 = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
        let port_v4 = listener_v4.local_addr()?.port();

        let listener_v6 = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, 0)))
            .await
            .ok();
        let port_v6 = listener_v6
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
            .unwrap_or(0);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        spawn_accept_loop(
            listener_v4,
            AF_INET,
            runtime_cfg.clone(),
            lookup.clone(),
            label_resolver.clone(),
            shutdown_rx.clone(),
        );
        if let Some(l6) = listener_v6 {
            spawn_accept_loop(
                l6,
                AF_INET6,
                runtime_cfg,
                lookup,
                label_resolver,
                shutdown_rx,
            );
        }

        Ok(Self {
            port_v4,
            port_v6,
            shutdown_tx,
        })
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

fn spawn_accept_loop(
    listener: TcpListener,
    family: u32,
    runtime_cfg: Arc<RwLock<BridgeRuntimeConfig>>,
    lookup: LookupFn,
    label_resolver: LabelFn,
    mut shutdown: watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, peer)) => {
                            let cfg = runtime_cfg.clone();
                            let lookup = lookup.clone();
                            let label_resolver = label_resolver.clone();
                            tokio::spawn(async move {
                                let _ = relay_connection(stream, peer, family, cfg, lookup, label_resolver).await;
                            });
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
}

async fn relay_connection(
    mut client: TcpStream,
    peer: SocketAddr,
    family: u32,
    runtime_cfg: Arc<RwLock<BridgeRuntimeConfig>>,
    lookup: LookupFn,
    label_resolver: LabelFn,
) -> anyhow::Result<()> {
    let _ = client.set_nodelay(true);
    let src_port = peer.port();

    let mut found = None;
    for attempt in 0..4 {
        if let Some(orig) = lookup(family, src_port) {
            found = Some(orig);
            break;
        }
        if attempt < 3 {
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    }
    let Some(orig) = found else {
        anyhow::bail!("missing original destination for {family}:{src_port}");
    };

    let (proxy_port, proxy_auth) = {
        let guard = runtime_cfg
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (guard.proxy_port, guard.proxy_auth.clone())
    };
    if proxy_port == 0 {
        anyhow::bail!("zniff proxy port is 0");
    }

    let authority = format_orig_authority(&orig);
    let source_app = sanitize_header_value(&label_resolver(orig.pid, &orig.comm));

    let mut proxy = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, proxy_port))).await?;
    let _ = proxy.set_nodelay(true);

    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if !source_app.is_empty() {
        req.push_str(&format!("X-Zniff-Source-App: {source_app}\r\n"));
    }
    if !proxy_auth.is_empty() {
        req.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64_encode(proxy_auth.as_bytes())
        ));
    }
    req.push_str("\r\n");
    proxy.write_all(req.as_bytes()).await?;
    proxy.flush().await?;

    let mut reader = BufReader::new(proxy);
    let mut status_line = String::new();
    let n = reader.read_line(&mut status_line).await?;
    if n == 0 || !is_http_200(&status_line) {
        anyhow::bail!("CONNECT rejected by zniff proxy: {}", status_line.trim());
    }
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    // Preserve any buffered bytes after HTTP CONNECT headers (if the peer pipelined).
    let buffered = reader.buffer().to_vec();
    let mut proxy = reader.into_inner();
    if !buffered.is_empty() {
        client.write_all(&buffered).await?;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut proxy).await;
    Ok(())
}

pub fn format_orig_authority(orig: &OrigDst) -> String {
    if orig.family == AF_INET6 {
        let ip = IpAddr::V6(Ipv6Addr::from(orig.dst_addr));
        format!("[{ip}]:{}", orig.dst_port)
    } else {
        let ip = IpAddr::V4(Ipv4Addr::new(
            orig.dst_addr[0],
            orig.dst_addr[1],
            orig.dst_addr[2],
            orig.dst_addr[3],
        ));
        format!("{ip}:{}", orig.dst_port)
    }
}

fn is_http_200(status_line: &str) -> bool {
    status_line
        .split_whitespace()
        .nth(1)
        .is_some_and(|code| code == "200")
}

fn sanitize_header_value(raw: &str) -> String {
    raw.chars()
        .filter(|&c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((combined >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(combined & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn bridge_sends_connect_with_source_app_and_auth_then_tunnels() {
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();

        let captured_headers = Arc::new(RwLock::new(String::new()));
        let captured_clone = captured_headers.clone();
        let proxy_task = tokio::spawn(async move {
            let (mut sock, _) = proxy_listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            *captured_clone.write().unwrap() = req;
            sock.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let mut msg = [0u8; 64];
            let m = sock.read(&mut msg).await.unwrap();
            sock.write_all(&msg[..m]).await.unwrap();
        });

        let runtime_cfg = Arc::new(RwLock::new(BridgeRuntimeConfig {
            proxy_port,
            proxy_auth: "alice:s3cr3t".into(),
        }));
        let lookup: LookupFn = Arc::new(|family, _src_port| {
            let mut dst_addr = [0u8; 16];
            dst_addr[0..4].copy_from_slice(&[93, 184, 216, 34]);
            let mut comm = [0u8; 16];
            comm[..4].copy_from_slice(b"curl");
            Some(OrigDst {
                family,
                dst_port: 443,
                _pad: 0,
                dst_addr,
                pid: 0,
                comm,
            })
        });
        let labeler: LabelFn = Arc::new(|_pid, comm| {
            let end = comm.iter().position(|&b| b == 0).unwrap_or(comm.len());
            String::from_utf8_lossy(&comm[..end]).into_owned()
        });

        let bridge = BridgeServer::start(runtime_cfg, lookup, labeler)
            .await
            .unwrap();

        let mut client =
            TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, bridge.port_v4)))
                .await
                .unwrap();
        client.write_all(b"PING-TLS").await.unwrap();
        let mut reply = [0u8; 16];
        let n = client.read(&mut reply).await.unwrap();
        assert_eq!(&reply[..n], b"PING-TLS");

        proxy_task.await.unwrap();
        bridge.stop();

        let req = captured_headers.read().unwrap().clone();
        assert!(req.starts_with("CONNECT 93.184.216.34:443 HTTP/1.1\r\n"));
        assert!(req.contains("X-Zniff-Source-App: curl\r\n"));
        assert!(req.contains("Proxy-Authorization: Basic YWxpY2U6czNjcjN0\r\n"));
    }
}
