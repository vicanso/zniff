//! Check GitHub Releases (with Gitee mirror fallback) for a newer version,
//! download, verify against `SHA256SUMS`, install in place, and relaunch.
//!
//! Follows the same design as `zstats.app`:
//! - Silent periodic check every two days (`AUTO_CHECK_EVERY`), persisted in
//!   `update-check.toml` inside the user data directory.
//! - GitHub first, Gitee mirror second (`REACH_TIMEOUT` = 10s per host).
//! - SHA-256 verification against the release's `SHA256SUMS` before touching
//!   the standing install.
//! - On macOS, mounts the DMG silently (`hdiutil attach -nobrowse -noverify -plist`),
//!   verifies `CFBundleIdentifier`, renames the running `.app` bundle aside into
//!   `$TMPDIR`, copies the new bundle in place with `/usr/bin/ditto`, detaches
//!   the volume, and spawns a detached shell to reopen the bundle as soon as
//!   this process exits. Any obstacle (running via bare `cargo run` or an
//!   unwritable `/Applications`) falls back to opening the DMG in Finder.
//! - On Linux and Windows, unpacks the release `.tar.gz`, swaps the binary in
//!   place, and spawns a detached shell to relaunch the new binary.

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
#[cfg(target_os = "macos")]
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper_util::rt::TokioIo;
use ring::digest::{Context as Sha256Context, SHA256};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::{ca, config};

const LATEST_URL: &str = "https://api.github.com/repos/vicanso/zniff/releases/latest";
const LIST_URL: &str = "https://api.github.com/repos/vicanso/zniff/releases";
const GITEE_API: &str = "https://gitee.com/api/v5/repos/vicanso/zniff";
const GITEE_REPO_URL: &str = "https://gitee.com/vicanso/zniff";

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const REACH_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 8;
const HISTORY_PAGE: usize = 30;
const CHECKSUMS_NAME: &str = "SHA256SUMS";

pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(target_os = "macos")]
pub fn asset_name() -> Option<&'static str> {
    match env::consts::ARCH {
        "aarch64" => Some("zniff-aarch64.dmg"),
        "x86_64" => Some("zniff-x86_64.dmg"),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
pub fn asset_name() -> Option<&'static str> {
    match env::consts::ARCH {
        "x86_64" => Some("zniff-linux-x86_64.tar.gz"),
        "aarch64" => Some("zniff-linux-aarch64.tar.gz"),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
pub fn asset_name() -> Option<&'static str> {
    match env::consts::ARCH {
        "x86_64" => Some("zniff-windows-x86_64.tar.gz"),
        _ => None,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn asset_name() -> Option<&'static str> {
    None
}

fn release_download_url(tag: &str, name: &str) -> String {
    format!("https://github.com/vicanso/zniff/releases/download/{tag}/{name}")
}

fn gitee_download_url(tag: &str, name: &str) -> String {
    format!("{GITEE_REPO_URL}/releases/download/{tag}/{name}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate,
    Newer { version: String, notes: String },
    Failed(String),
}

/// What [`install`] did with the verified download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The fresh build was put in place of the running one; a relaunch
    /// completes the update.
    Replaced,
    /// The image was handed to the OS for the classic drag — no bundle
    /// to replace, or the in-place copy could not proceed.
    #[allow(dead_code)]
    OpenedForDrag,
}

// ---- HTTP(S) client with redirect & optional upstream proxy -------------

struct ParsedUrl {
    https: bool,
    host: String,
    port: u16,
    path_and_query: String,
}

fn parse_url(raw: &str) -> Result<ParsedUrl, String> {
    let trimmed = raw.trim();
    let (https, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("unsupported URL scheme: {trimmed}"));
    };
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_port)| host_port);
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (h, after) = bracketed
            .split_once(']')
            .ok_or_else(|| format!("malformed IPv6 host in {trimmed}"))?;
        let port = if let Some(p) = after.strip_prefix(':') {
            p.parse::<u16>()
                .map_err(|_| format!("invalid port in {trimmed}"))?
        } else if https {
            443
        } else {
            80
        };
        (h.to_string(), port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        let port = p
            .parse::<u16>()
            .map_err(|_| format!("invalid port in {trimmed}"))?;
        (h.to_string(), port)
    } else {
        (authority.to_string(), if https { 443 } else { 80 })
    };
    if host.is_empty() {
        return Err(format!("empty host in {trimmed}"));
    }
    Ok(ParsedUrl {
        https,
        host,
        port,
        path_and_query: path_and_query.to_string(),
    })
}

fn resolve_redirect(base: &ParsedUrl, location: &str) -> String {
    let loc = location.trim();
    if loc.starts_with("https://") || loc.starts_with("http://") {
        return loc.to_string();
    }
    let scheme = if base.https { "https" } else { "http" };
    let default_port = if base.https { 443 } else { 80 };
    let port_part = if base.port == default_port {
        String::new()
    } else {
        format!(":{}", base.port)
    };
    if let Some(rest) = loc.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    if loc.starts_with('/') {
        format!("{scheme}://{}{port_part}{loc}", base.host)
    } else {
        format!("{scheme}://{}{port_part}/{loc}", base.host)
    }
}

fn configured_upstream_proxy() -> Option<(String, u16)> {
    let cfg = config::load(&ca::data_dir());
    let raw = cfg.upstream_proxy.trim();
    if raw.is_empty() {
        return None;
    }
    let rest = raw
        .strip_prefix("http://")
        .or_else(|| raw.strip_prefix("https://"))
        .unwrap_or(raw);
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port_str) = authority.rsplit_once(':')?;
    let port = port_str.parse::<u16>().ok()?;
    if port == 0 || host.is_empty() {
        return None;
    }
    // Never loop through zniff's own listening port.
    if (host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost")) && port == cfg.listen_port {
        return None;
    }
    Some((host.to_string(), port))
}

async fn open_tcp_to(host: &str, port: u16) -> Result<TcpStream, String> {
    if let Some((proxy_host, proxy_port)) = configured_upstream_proxy() {
        let mut stream = TcpStream::connect((proxy_host.as_str(), proxy_port))
            .await
            .map_err(|e| format!("upstream proxy {proxy_host}:{proxy_port}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let connect_req = format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
        );
        stream
            .write_all(connect_req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(512);
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            let n = stream.read(&mut byte).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("upstream proxy closed during CONNECT".into());
            }
            buf.push(byte[0]);
            if buf.len() > 8192 {
                return Err("upstream proxy CONNECT header too large".into());
            }
        }
        let status_line = String::from_utf8_lossy(&buf);
        let ok = status_line
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .is_some_and(|code| code == "200");
        if !ok {
            return Err(format!(
                "upstream proxy CONNECT failed: {}",
                status_line.lines().next().unwrap_or_default()
            ));
        }
        Ok(stream)
    } else {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        Ok(stream)
    }
}

fn tls_connector() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsConnector::from(std::sync::Arc::new(config))
}

async fn fetch_url_streaming(
    initial_url: &str,
    accept: Option<&str>,
    reach_timeout: Duration,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    let connector = tls_connector();
    let mut current_url = initial_url.to_string();

    for _hop in 0..=MAX_REDIRECTS {
        let parsed = parse_url(&current_url)?;
        let host_header =
            if (parsed.https && parsed.port == 443) || (!parsed.https && parsed.port == 80) {
                parsed.host.clone()
            } else {
                format!("{}:{}", parsed.host, parsed.port)
            };

        let tcp = timeout(reach_timeout, open_tcp_to(&parsed.host, parsed.port))
            .await
            .map_err(|_| format!("connect timeout reaching {}", parsed.host))??;

        let mut req_builder = Request::builder()
            .method("GET")
            .uri(&parsed.path_and_query)
            .header("Host", &host_header)
            .header("User-Agent", format!("zniff/{}", version()))
            .header("Connection", "close");
        if let Some(acc) = accept {
            req_builder = req_builder.header("Accept", acc);
        }
        let req = req_builder
            .body(Empty::<Bytes>::new())
            .map_err(|e| e.to_string())?;

        let response = if parsed.https {
            let server_name = ServerName::try_from(parsed.host.clone())
                .map_err(|_| format!("invalid TLS server name: {}", parsed.host))?;
            let tls = timeout(reach_timeout, connector.connect(server_name, tcp))
                .await
                .map_err(|_| format!("TLS handshake timeout with {}", parsed.host))?
                .map_err(|e| format!("TLS error with {}: {e}", parsed.host))?;
            let (mut sender, conn) = timeout(
                reach_timeout,
                hyper::client::conn::http1::handshake(TokioIo::new(tls)),
            )
            .await
            .map_err(|_| "HTTP handshake timeout".to_string())?
            .map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            timeout(reach_timeout, sender.send_request(req))
                .await
                .map_err(|_| format!("response timeout from {}", parsed.host))?
                .map_err(|e| e.to_string())?
        } else {
            let (mut sender, conn) = timeout(
                reach_timeout,
                hyper::client::conn::http1::handshake(TokioIo::new(tcp)),
            )
            .await
            .map_err(|_| "HTTP handshake timeout".to_string())?
            .map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            timeout(reach_timeout, sender.send_request(req))
                .await
                .map_err(|_| format!("response timeout from {}", parsed.host))?
                .map_err(|e| e.to_string())?
        };

        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let loc = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("redirect ({status}) without Location header"))?;
            current_url = resolve_redirect(&parsed, loc);
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status} from {}", parsed.host));
        }

        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let mut body = response.into_body();
        let mut out = Vec::with_capacity(total.min(16 * 1024 * 1024) as usize);
        on_progress(0, total);
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| e.to_string())?;
            if let Ok(chunk) = frame.into_data() {
                out.extend_from_slice(&chunk);
                if out.len() as u64 > MAX_DOWNLOAD {
                    return Err("download exceeded the size cap".into());
                }
                on_progress(out.len() as u64, total);
            }
        }
        return Ok(out);
    }

    Err("too many HTTP redirects".into())
}

fn fetch_bytes_blocking(
    url: &str,
    accept: Option<&str>,
    reach_timeout: Duration,
    total_timeout: Duration,
    on_progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async move {
        timeout(
            total_timeout,
            fetch_url_streaming(url, accept, reach_timeout, on_progress),
        )
        .await
        .map_err(|_| "request timed out".to_string())?
    })
}

fn fetch_text(url: &str, accept: Option<&str>) -> Result<String, String> {
    let bytes = fetch_bytes_blocking(url, accept, FETCH_TIMEOUT, FETCH_TIMEOUT, |_, _| {})?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn fetch_asset_bytes(
    tag: &str,
    name: &str,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    let github_url = release_download_url(tag, name);
    let direct_err = match fetch_bytes_blocking(
        &github_url,
        None,
        REACH_TIMEOUT,
        DOWNLOAD_TIMEOUT,
        &mut on_progress,
    ) {
        Ok(bytes) => return Ok(bytes),
        Err(err) => err,
    };
    let gitee_url = gitee_download_url(tag, name);
    fetch_bytes_blocking(
        &gitee_url,
        None,
        REACH_TIMEOUT,
        DOWNLOAD_TIMEOUT,
        &mut on_progress,
    )
    .map_err(|mirror| format!("{direct_err}; mirror: {mirror}"))
}

/// Download `tag`'s release asset for this platform/arch and verify its SHA-256
/// against `SHA256SUMS`. Blocking — run on a background thread.
pub fn download(tag: &str, mut on_progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    let Some(asset) = asset_name() else {
        return Err(format!(
            "no release build for {}/{}",
            env::consts::OS,
            env::consts::ARCH
        ));
    };
    let bytes = fetch_asset_bytes(tag, asset, &mut on_progress)?;
    let path = env::temp_dir().join(format!("{tag}-{asset}"));
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;

    match fetch_checksum(tag, asset) {
        Some(expected) => {
            let Some(got) = file_sha256(&path) else {
                let _ = fs::remove_file(&path);
                return Err("could not verify the download (could not hash it)".into());
            };
            if !got.eq_ignore_ascii_case(&expected) {
                let _ = fs::remove_file(&path);
                return Err(format!("checksum mismatch: expected {expected}, got {got}"));
            }
        }
        None if cfg!(not(target_os = "macos")) => {
            let _ = fs::remove_file(&path);
            return Err(
                "the release carries no SHA256SUMS; not installing unverified bytes".into(),
            );
        }
        None => {}
    }

    Ok(path)
}

fn fetch_checksum(tag: &str, asset: &str) -> Option<String> {
    let bytes = fetch_asset_bytes(tag, CHECKSUMS_NAME, |_, _| {}).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    parse_checksum_line(&text, asset)
}

fn parse_checksum_line(text: &str, asset: &str) -> Option<String> {
    text.lines()
        .find(|line| {
            let trimmed = line.trim_end();
            trimmed.ends_with(&format!(" {asset}")) || trimmed.ends_with(&format!(" *{asset}"))
        })
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_string)
}

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut ctx = Sha256Context::new(&SHA256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    let digest = ctx.finish();
    let mut out = String::with_capacity(64);
    for byte in digest.as_ref() {
        out.push_str(&format!("{byte:02x}"));
    }
    Some(out)
}

// ---- macOS: in-place .app bundle replacement ---------------------------

#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "dev.zniff.app";

#[cfg(target_os = "macos")]
const BUNDLE_NAME: &str = "zniff.app";

#[cfg(target_os = "macos")]
const INSTALLER_VOLUME_PREFIX: &str = "zniff Installer";

#[cfg(target_os = "macos")]
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(target_os = "macos")]
pub fn install(dmg: &Path) -> Result<Delivery, String> {
    match running_bundle() {
        Some(target) => match install_over(&target, dmg) {
            Ok(()) => return Ok(Delivery::Replaced),
            Err(err) => {
                log::warn!("in-place install fell back to Finder drag window: {err}");
            }
        },
        None => {
            log::info!("no running .app bundle (bare binary); opening DMG in Finder");
        }
    }
    let status = Command::new("/usr/bin/open")
        .arg(dmg.as_os_str())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("open refused to mount the DMG".into());
    }
    let _ = Command::new("/usr/bin/open")
        .args(["-a", "Finder"])
        .status();
    Ok(Delivery::OpenedForDrag)
}

#[cfg(target_os = "macos")]
pub fn running_bundle() -> Option<PathBuf> {
    bundle_root_of(&env::current_exe().ok()?)
}

#[cfg(target_os = "macos")]
fn bundle_root_of(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.parent()?;
    (root.extension().is_some_and(|ext| ext == "app")).then(|| root.to_path_buf())
}

#[cfg(target_os = "macos")]
fn install_over(target: &Path, dmg: &Path) -> Result<(), String> {
    let volume = attach(dmg)?;
    let result = replace_bundle(target, &volume);
    detach(&volume);
    result
}

#[cfg(target_os = "macos")]
fn replace_bundle(target: &Path, volume: &Path) -> Result<(), String> {
    let fresh = volume.join(BUNDLE_NAME);
    if bundle_plist_value(&fresh, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
        return Err("the mounted image does not carry our bundle".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let aside = env::temp_dir().join(format!("zniff-previous-{}-{stamp}.app", process::id()));
    fs::rename(target, &aside).map_err(|e| format!("could not move the old bundle aside: {e}"))?;
    let copied = Command::new("/usr/bin/ditto")
        .arg(fresh.as_os_str())
        .arg(target.as_os_str())
        .output();
    let failure = match &copied {
        Ok(out) if out.status.success() => {
            log::info!("update installed in place at {}", target.display());
            return Ok(());
        }
        Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
        Err(e) => e.to_string(),
    };
    if let Err(e) = fs::rename(&aside, target) {
        log::error!(
            "could not restore the old bundle from {}: {e}",
            aside.display()
        );
    }
    Err(format!("ditto failed: {failure}"))
}

#[cfg(target_os = "macos")]
fn attach(dmg: &Path) -> Result<PathBuf, String> {
    let out = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-noverify", "-plist"])
        .arg(dmg.as_os_str())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "hdiutil attach: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mount_point_from_plist(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "no mount point in hdiutil's output".into())
}

#[cfg(target_os = "macos")]
fn mount_point_from_plist(xml: &str) -> Option<PathBuf> {
    let after = xml.split("<key>mount-point</key>").nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let end = start + after[start..].find("</string>")?;
    Some(PathBuf::from(&after[start..end]))
}

#[cfg(target_os = "macos")]
fn detach(volume: &Path) {
    for attempt in 0..2 {
        if attempt > 0 {
            thread::sleep(Duration::from_secs(1));
        }
        let detached = Command::new("hdiutil")
            .arg("detach")
            .arg(volume.as_os_str())
            .output()
            .is_ok_and(|out| out.status.success());
        if detached {
            return;
        }
    }
    log::warn!(
        "installer image {} left mounted; the launch sweep will retry",
        volume.display()
    );
}

#[cfg(target_os = "macos")]
pub fn relaunch() {
    let Some(bundle) = running_bundle() else {
        return;
    };
    let script = format!(
        "while /bin/kill -0 {pid} 2>/dev/null; do /bin/sleep 0.1; done; /usr/bin/open \"$0\"",
        pid = process::id()
    );
    match Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(bundle.as_os_str())
        .spawn()
    {
        Ok(_) => log::info!("restart requested for {}", bundle.display()),
        Err(e) => log::warn!("could not spawn the relauncher: {e}"),
    }
}

#[cfg(target_os = "macos")]
pub fn sweep_installer_mounts() {
    let running = env::current_exe().ok();
    let Ok(volumes) = fs::read_dir("/Volumes") else {
        return;
    };
    for entry in volumes.flatten() {
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(|name| name.starts_with(INSTALLER_VOLUME_PREFIX))
        {
            continue;
        }
        let volume = entry.path();
        let app = volume.join(BUNDLE_NAME);
        if bundle_plist_value(&app, "CFBundleIdentifier").as_deref() != Some(BUNDLE_ID) {
            continue;
        }
        if running.as_ref().is_some_and(|exe| exe.starts_with(&volume)) {
            continue;
        }
        let mounted_ver =
            bundle_plist_value(&app, "CFBundleShortVersionString").unwrap_or_default();
        if is_newer(&mounted_ver, version()) {
            continue;
        }
        let detached = Command::new("hdiutil")
            .arg("detach")
            .arg(volume.as_os_str())
            .output()
            .is_ok_and(|out| out.status.success());
        if !detached {
            log::warn!(
                "installer image {} would not detach during sweep",
                volume.display()
            );
            continue;
        }
        let _ = Command::new(LSREGISTER)
            .arg("-u")
            .arg(app.as_os_str())
            .output();
        log::info!(
            "stale installer image {} ({mounted_ver}) detached",
            volume.display()
        );
    }
}

#[cfg(target_os = "macos")]
fn bundle_plist_value(app: &Path, key: &str) -> Option<String> {
    let out = Command::new("defaults")
        .arg("read")
        .arg(app.join("Contents/Info").as_os_str())
        .arg(key)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

// ---- Linux: tarball over the running binary ----------------------------

#[cfg(target_os = "linux")]
static INSTALLED_AT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(target_os = "linux")]
const UNPACK_PREFIX: &str = "zniff-update-";

#[cfg(target_os = "linux")]
pub fn install(tarball: &Path) -> Result<Delivery, String> {
    let target = env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let target = without_deleted_suffix(&target);
    install_into(tarball, &target)?;
    *INSTALLED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target);
    Ok(Delivery::Replaced)
}

#[cfg(target_os = "linux")]
fn install_into(tarball: &Path, target: &Path) -> Result<(), String> {
    let unpack = env::temp_dir().join(format!("{UNPACK_PREFIX}{}", process::id()));
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack)
        .map_err(|e| format!("could not create {}: {e}", unpack.display()))?;
    let result = unpack_and_replace(tarball, &unpack, target);
    let _ = fs::remove_dir_all(&unpack);
    result
}

#[cfg(target_os = "linux")]
fn unpack_and_replace(tarball: &Path, unpack: &Path, target: &Path) -> Result<(), String> {
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(tarball.as_os_str())
        .arg("-C")
        .arg(unpack.as_os_str())
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tar -xzf: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let member = tarball_member_dir().ok_or("no release build for this architecture")?;
    let fresh = unpack.join(member).join("zniff");
    if !fresh.is_file() {
        return Err(format!("the tarball has no {member}/zniff in it"));
    }
    replace_binary(target, &fresh)
}

#[cfg(target_os = "linux")]
fn tarball_member_dir() -> Option<&'static str> {
    asset_name()?.strip_suffix(".tar.gz")
}

#[cfg(target_os = "linux")]
fn replace_binary(target: &Path, fresh: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the running binary has no file name")?;
    let staged = target.with_file_name(format!("{name}.new"));
    fs::copy(fresh, &staged).map_err(|e| {
        format!(
            "could not stage the new binary beside {}: {e} — installed by a package manager? update it there",
            target.display()
        )
    })?;
    if let Err(e) = fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)) {
        let _ = fs::remove_file(&staged);
        return Err(format!("could not mark the new binary executable: {e}"));
    }
    if let Err(e) = fs::rename(&staged, target) {
        let _ = fs::remove_file(&staged);
        return Err(format!("could not replace {}: {e}", target.display()));
    }
    log::info!("update installed in place at {}", target.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn without_deleted_suffix(exe: &Path) -> PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(clean) => PathBuf::from(clean),
        None => exe.to_path_buf(),
    }
}

#[cfg(target_os = "linux")]
pub fn relaunch() {
    let recorded = INSTALLED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(exe) =
        recorded.or_else(|| env::current_exe().ok().map(|e| without_deleted_suffix(&e)))
    else {
        return;
    };
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.1; done; exec \"$0\"",
        pid = process::id()
    );
    match Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(exe.as_os_str())
        .spawn()
    {
        Ok(_) => log::info!("restart requested for {}", exe.display()),
        Err(e) => log::warn!("could not spawn the relauncher: {e}"),
    }
}

#[cfg(target_os = "linux")]
pub fn sweep_installer_mounts() {
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(UNPACK_PREFIX));
        if stale && fs::remove_dir_all(entry.path()).is_ok() {
            log::info!(
                "stale update unpack directory {} removed",
                entry.path().display()
            );
        }
    }
}

// ---- Windows: tarball over the running .exe ----------------------------

#[cfg(target_os = "windows")]
static INSTALLED_AT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(target_os = "windows")]
const UNPACK_PREFIX: &str = "zniff-update-";

#[cfg(target_os = "windows")]
pub fn install(tarball: &Path) -> Result<Delivery, String> {
    let target = env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let unpack = env::temp_dir().join(format!("{UNPACK_PREFIX}{}", process::id()));
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack)
        .map_err(|e| format!("could not create {}: {e}", unpack.display()))?;
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(tarball.as_os_str())
        .arg("-C")
        .arg(unpack.as_os_str())
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        let _ = fs::remove_dir_all(&unpack);
        return Err(format!(
            "tar -xzf: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let member = asset_name()
        .and_then(|n| n.strip_suffix(".tar.gz"))
        .ok_or("no release build for this architecture")?;
    let fresh = unpack.join(member).join("zniff.exe");
    if !fresh.is_file() {
        let _ = fs::remove_dir_all(&unpack);
        return Err(format!("the archive has no {member}/zniff.exe in it"));
    }
    let old = target.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    fs::rename(&target, &old).map_err(|e| format!("could not rename running exe aside: {e}"))?;
    if let Err(e) = fs::copy(&fresh, &target) {
        let _ = fs::rename(&old, &target);
        let _ = fs::remove_dir_all(&unpack);
        return Err(format!("could not copy new exe into place: {e}"));
    }
    let _ = fs::remove_dir_all(&unpack);
    *INSTALLED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target);
    Ok(Delivery::Replaced)
}

#[cfg(target_os = "windows")]
pub fn relaunch() {
    let recorded = INSTALLED_AT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(exe) = recorded.or_else(|| env::current_exe().ok()) else {
        return;
    };
    let cmd = format!(
        "ping 127.0.0.1 -n 2 > nul & start \"\" \"{}\"",
        exe.display()
    );
    let _ = Command::new("cmd").args(["/C", &cmd]).spawn();
}

#[cfg(target_os = "windows")]
pub fn sweep_installer_mounts() {
    if let Ok(exe) = env::current_exe() {
        let old = exe.with_extension("exe.old");
        let _ = fs::remove_file(old);
    }
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(UNPACK_PREFIX));
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn install(_asset: &Path) -> Result<Delivery, String> {
    Err("in-place install is not implemented on this platform".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn relaunch() {}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn sweep_installer_mounts() {}

// ---- Release check & notes stitching -----------------------------------

/// Ask GitHub (or the Gitee mirror) for the latest release and compare it to
/// this build. Blocking — call on a background thread.
pub fn check() -> UpdateCheck {
    check_latest()
}

/// [`check`] plus any intermediate release notes between `version()` and the
/// latest release.
pub fn check_with_skipped() -> UpdateCheck {
    let (latest_ver, latest_notes) = match check_latest() {
        UpdateCheck::Newer { version, notes } => (version, notes),
        other => return other,
    };
    let github = format!("{LIST_URL}?per_page={HISTORY_PAGE}");
    let gitee = format!("{GITEE_API}/releases?per_page={HISTORY_PAGE}&direction=desc");
    let list = fetch_text(&github, Some("application/vnd.github+json")).or_else(|origin| {
        fetch_text(&gitee, None).map_err(|mirror| format!("{origin}; mirror: {mirror}"))
    });
    let notes = match list {
        Ok(list) => notes_through(&list, version(), &latest_ver, &latest_notes),
        Err(err) => {
            log::warn!("release list unavailable ({err}); showing latest notes only");
            latest_notes
        }
    };
    UpdateCheck::Newer {
        version: latest_ver,
        notes,
    }
}

fn check_latest() -> UpdateCheck {
    let body = match fetch_text(LATEST_URL, Some("application/vnd.github+json")) {
        Ok(body) => body,
        Err(origin) => match fetch_text(&format!("{GITEE_API}/releases/latest"), None) {
            Ok(body) => body,
            Err(mirror) => return UpdateCheck::Failed(format!("{origin}; mirror: {mirror}")),
        },
    };
    let Some(tag) = json_str_field(&body, "tag_name") else {
        return UpdateCheck::Failed("no tag_name in response".into());
    };
    let notes = release_body(&body);
    if is_newer(&tag, version()) {
        UpdateCheck::Newer {
            version: tag,
            notes,
        }
    } else {
        UpdateCheck::UpToDate
    }
}

fn release_body(release: &str) -> String {
    json_str_field(release, "body")
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn notes_through(list: &str, current: &str, latest: &str, latest_notes: &str) -> String {
    let mut skipped: Vec<(String, String)> = json_objects(list)
        .into_iter()
        .filter_map(|release| {
            let tag = json_str_field(release, "tag_name")?;
            (is_newer(&tag, current) && is_newer(latest, &tag))
                .then(|| (tag, release_body(release)))
        })
        .filter(|(_, body)| !body.trim().is_empty())
        .collect();
    if skipped.is_empty() {
        return latest_notes.to_string();
    }
    skipped.sort_by(|(a, _), (b, _)| is_newer(b, a).cmp(&is_newer(a, b)));
    skipped.dedup_by(|(a, _), (b, _)| a == b);
    let headed = |tag: &str, body: &str| {
        let body = body.trim();
        let number = tag.trim_start_matches('v');
        let opens_named = body
            .lines()
            .next()
            .is_some_and(|line| line.starts_with('#') && line.contains(number));
        if opens_named {
            body.to_string()
        } else {
            format!("## {tag}\n\n{body}")
        }
    };
    let mut parts = Vec::with_capacity(skipped.len() + 1);
    if !latest_notes.trim().is_empty() {
        parts.push(headed(latest, latest_notes));
    }
    parts.extend(skipped.iter().map(|(tag, body)| headed(tag, body)));
    parts.join("\n\n")
}

fn json_objects(array: &str) -> Vec<&str> {
    let mut objects = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for (at, c) in array.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = at;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    objects.push(&array[start..=at]);
                }
            }
            _ => {}
        }
    }
    objects
}

fn json_str_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut search = 0;
    loop {
        let rel = json[search..].find(&needle)?;
        let after = search + rel + needle.len();
        let rest = json[after..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if rest.starts_with("null") {
                return Some(String::new());
            }
            if let Some(rest) = rest.strip_prefix('"') {
                return unescape_json_string(rest);
            }
        }
        search = after;
    }
}

fn unescape_json_string(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000c}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() != 4 {
                        return None;
                    }
                    let cp = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(cp)?);
                }
                _ => return None,
            },
            other => out.push(other),
        }
    }
    None
}

// ---- Silent periodic check (update-check.toml) -------------------------

const AUTO_CHECK_EVERY: Duration = Duration::from_secs(2 * 24 * 60 * 60);

fn auto_check_path(dir: &Path) -> PathBuf {
    let file_name = if config::is_dev_mode() {
        "update-check.dev.toml"
    } else {
        "update-check.toml"
    };
    dir.join(file_name)
}

#[derive(Default)]
struct CheckFile {
    checked_unix: u64,
    latest: Option<String>,
    ignored: Option<String>,
}

fn read_check_in(dir: &Path) -> CheckFile {
    let doc = fs::read_to_string(auto_check_path(dir))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok());
    let Some(doc) = doc else {
        return CheckFile::default();
    };
    let get = |k: &str| doc.get(k).and_then(toml::Value::as_str).map(str::to_string);
    CheckFile {
        checked_unix: doc
            .get("checked_unix")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0)
            .max(0) as u64,
        latest: get("latest"),
        ignored: get("ignored"),
    }
}

fn write_check_in(dir: &Path, file: &CheckFile) {
    let clean = |v: &str| v.replace(['\\', '"'], "");
    let mut out = format!("checked_unix = {}\n", file.checked_unix);
    if let Some(v) = &file.latest {
        out.push_str(&format!("latest = \"{}\"\n", clean(v)));
    }
    if let Some(v) = &file.ignored {
        out.push_str(&format!("ignored = \"{}\"\n", clean(v)));
    }
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(auto_check_path(dir), out);
}

pub fn auto_check_due(now: SystemTime) -> bool {
    auto_check_due_in(&ca::data_dir(), now)
}

fn auto_check_due_in(dir: &Path, now: SystemTime) -> bool {
    let checked = UNIX_EPOCH + Duration::from_secs(read_check_in(dir).checked_unix);
    now.duration_since(checked)
        .map_or(true, |age| age >= AUTO_CHECK_EVERY)
}

pub fn record_outcome(now: SystemTime, outcome: &UpdateCheck) {
    record_outcome_in(&ca::data_dir(), now, outcome);
}

fn record_outcome_in(dir: &Path, now: SystemTime, outcome: &UpdateCheck) {
    let mut file = read_check_in(dir);
    file.checked_unix = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match outcome {
        UpdateCheck::Newer { version, .. } => file.latest = Some(version.clone()),
        UpdateCheck::UpToDate => file.latest = None,
        UpdateCheck::Failed(_) => {}
    }
    write_check_in(dir, &file);
}

pub fn ignore(version: &str) {
    ignore_in(&ca::data_dir(), version);
}

fn ignore_in(dir: &Path, version: &str) {
    let mut file = read_check_in(dir);
    file.ignored = Some(version.to_string());
    write_check_in(dir, &file);
}

pub fn ignored() -> Option<String> {
    ignored_in(&ca::data_dir(), version())
}

fn ignored_in(dir: &Path, current: &str) -> Option<String> {
    let marked = read_check_in(dir).ignored?;
    is_newer(&marked, current).then_some(marked)
}

pub fn unignore() {
    unignore_in(&ca::data_dir());
}

fn unignore_in(dir: &Path) {
    let mut file = read_check_in(dir);
    file.ignored = None;
    write_check_in(dir, &file);
}

pub fn nudge() -> Option<String> {
    nudge_in(&ca::data_dir(), version())
}

fn nudge_in(dir: &Path, current: &str) -> Option<String> {
    let file = read_check_in(dir);
    let latest = file.latest?;
    if file.ignored.as_deref() == Some(latest.as_str()) {
        return None;
    }
    is_newer(&latest, current).then_some(latest)
}

/// `v0.1.2` vs `0.1.1` — numeric segment compare, missing segments are zero.
pub fn is_newer(remote_tag: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|seg| {
                seg.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (remote, local) = (parse(remote_tag), parse(current));
    for i in 0..remote.len().max(local.len()) {
        let r = remote.get(i).copied().unwrap_or(0);
        let l = local.get(i).copied().unwrap_or(0);
        if r != l {
            return r > l;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_sweep_identifier_matches_macos_app_script() {
        let script = include_str!("../../scripts/macos-app.sh");
        assert!(script.contains(&format!("<string>{BUNDLE_ID}</string>")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mount_point_comes_out_of_hdiutil_plist_output() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>system-entities</key><array>
  <dict>
    <key>content-hint</key><string>GUID_partition_scheme</string>
    <key>dev-entry</key><string>/dev/disk5</string>
  </dict>
  <dict>
    <key>content-hint</key><string>Apple_HFS</string>
    <key>dev-entry</key><string>/dev/disk5s1</string>
    <key>mount-point</key>
    <string>/Volumes/zniff Installer</string>
  </dict>
</array></dict></plist>"#;
        assert_eq!(
            mount_point_from_plist(xml),
            Some(PathBuf::from("/Volumes/zniff Installer"))
        );
        assert_eq!(mount_point_from_plist("<plist></plist>"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bundle_root_is_the_app_directory_or_nothing() {
        assert_eq!(
            bundle_root_of(Path::new("/Applications/zniff.app/Contents/MacOS/zniff")),
            Some(PathBuf::from("/Applications/zniff.app"))
        );
        assert_eq!(
            bundle_root_of(Path::new("/Users/x/proj/target/debug/zniff")),
            None
        );
    }

    #[cfg(target_os = "macos")]
    fn fixture_dmg(dir: &Path, id: &str, marker: &[u8], volname: &str) -> PathBuf {
        let contents = dir.join("payload").join(BUNDLE_NAME).join("Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        fs::write(contents.join("MacOS/zniff"), marker).unwrap();
        fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>{id}</string>
</dict></plist>
"#
            ),
        )
        .unwrap();
        let dmg = dir.join("update.dmg");
        let created = Command::new("hdiutil")
            .args(["create", "-srcfolder"])
            .arg(dir.join("payload").as_os_str())
            .args(["-volname", volname, "-format", "UDZO", "-quiet"])
            .arg(dmg.as_os_str())
            .output()
            .unwrap();
        assert!(created.status.success(), "hdiutil create failed");
        dmg
    }

    #[cfg(target_os = "macos")]
    fn fixture_target(dir: &Path) -> PathBuf {
        let target = dir.join("Applications").join(BUNDLE_NAME);
        fs::create_dir_all(target.join("Contents/MacOS")).unwrap();
        fs::write(target.join("Contents/MacOS/zniff"), b"old build").unwrap();
        target
    }

    #[cfg(target_os = "macos")]
    fn asides() -> Vec<PathBuf> {
        let prefix = format!("zniff-previous-{}-", process::id());
        fs::read_dir(env::temp_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn in_place_install_swaps_the_bundle_and_detaches() {
        let dir = env::temp_dir().join(format!("zniff-inplace-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let volname = "zniff-inplace-test";
        let dmg = fixture_dmg(&dir, BUNDLE_ID, b"new build", volname);
        let target = fixture_target(&dir);

        install_over(&target, &dmg).expect("in-place install");

        assert_eq!(
            fs::read(target.join("Contents/MacOS/zniff")).unwrap(),
            b"new build"
        );
        assert!(
            !Path::new("/Volumes").join(volname).exists(),
            "the volume must be detached"
        );
        let parked = asides();
        assert!(
            parked
                .iter()
                .any(|p| fs::read(p.join("Contents/MacOS/zniff"))
                    .is_ok_and(|bytes| bytes == b"old build")),
            "the old bundle must survive in temp"
        );
        for p in parked {
            let _ = fs::remove_dir_all(p);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_foreign_bundle_is_refused_before_anything_moves() {
        let dir = env::temp_dir().join(format!("zniff-foreign-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let volname = "zniff-foreign-test";
        let dmg = fixture_dmg(&dir, "com.example.stranger", b"impostor", volname);
        let target = fixture_target(&dir);

        let refused = install_over(&target, &dmg);

        assert!(refused.is_err(), "a foreign identifier must be refused");
        assert_eq!(
            fs::read(target.join("Contents/MacOS/zniff")).unwrap(),
            b"old build",
            "the standing install must be untouched"
        );
        assert!(
            !Path::new("/Volumes").join(volname).exists(),
            "the volume must be detached even on refusal"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mirror_url_is_spelled_the_same_way_githubs_is() {
        let tag = "v0.3.2";
        let name = "zniff-aarch64.dmg";
        assert_eq!(
            gitee_download_url(tag, name),
            "https://gitee.com/vicanso/zniff/releases/download/v0.3.2/zniff-aarch64.dmg"
        );
        let tail = |url: String| url.split_once("/releases/").map(|(_, t)| t.to_string());
        assert_eq!(
            tail(release_download_url(tag, name)),
            tail(gitee_download_url(tag, name)),
        );
    }

    #[test]
    fn version_compare_is_numeric_per_segment() {
        assert!(is_newer("v0.1.2", "0.1.1"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.1", "0.1.1"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(is_newer("v0.1.1.1", "0.1.1"));
        assert!(!is_newer("v0.1.1", "0.1.1.0"));
        assert!(!is_newer("nightly", "0.1.1"));
    }

    #[test]
    fn json_fields_come_from_the_top_level_not_the_notes() {
        let body = r#"{"url":"x","html_url":"https://github.com/vicanso/zniff/releases/tag/v0.1.2","id":1,"tag_name":"v0.1.2","body":"notes mentioning \"tag_name\": \"v9.9.9\" in text"}"#;
        assert_eq!(json_str_field(body, "tag_name").as_deref(), Some("v0.1.2"));
        assert_eq!(
            json_str_field(body, "html_url").as_deref(),
            Some("https://github.com/vicanso/zniff/releases/tag/v0.1.2")
        );
        assert_eq!(
            json_str_field(body, "body").as_deref(),
            Some("notes mentioning \"tag_name\": \"v9.9.9\" in text")
        );
        assert_eq!(json_str_field(body, "missing"), None);
    }

    #[test]
    fn skipped_release_notes_are_stitched_newest_first() {
        let list = r###"[
            {"tag_name":"v0.1.3","body":"## [0.1.3](c) - d\n\n- newest"},
            {"tag_name":"nightly","body":"Rolling build"},
            {"tag_name":"v0.1.2","body":"hand-written notes\r\n"},
            {"tag_name":"v0.1.1","body":"## [0.1.1](c) - d\n\n- current"}
        ]"###;
        let combined = notes_through(list, "0.1.1", "v0.1.3", "## [0.1.3](c) - d\n\n- newest");
        assert_eq!(
            combined,
            "## [0.1.3](c) - d\n\n- newest\n\n## v0.1.2\n\nhand-written notes"
        );
    }

    #[test]
    fn check_file_tracks_cadence_nudge_and_ignore() {
        let dir = env::temp_dir().join(format!("zniff-update-check-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        let t0 = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert!(auto_check_due_in(&dir, t0));

        record_outcome_in(
            &dir,
            t0,
            &UpdateCheck::Newer {
                version: "v0.2.0".into(),
                notes: "- fast".into(),
            },
        );
        assert!(!auto_check_due_in(&dir, t0 + Duration::from_secs(3600)));
        assert!(auto_check_due_in(&dir, t0 + AUTO_CHECK_EVERY));
        assert_eq!(nudge_in(&dir, "0.1.0").as_deref(), Some("v0.2.0"));
        assert_eq!(nudge_in(&dir, "0.2.0"), None);

        ignore_in(&dir, "v0.2.0");
        assert_eq!(nudge_in(&dir, "0.1.0"), None);
        assert_eq!(ignored_in(&dir, "0.1.0").as_deref(), Some("v0.2.0"));
        assert_eq!(ignored_in(&dir, "0.2.0"), None);

        unignore_in(&dir);
        assert_eq!(nudge_in(&dir, "0.1.0").as_deref(), Some("v0.2.0"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn checksum_and_url_helpers_handle_redirects_and_sha256() {
        let dir = env::temp_dir().join(format!("zniff-sha256-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.bin");
        fs::write(&file, b"hello zniff").unwrap();
        let hash = file_sha256(&file).unwrap();
        let sums = format!("{hash}  zniff-aarch64.dmg\n");
        assert_eq!(
            parse_checksum_line(&sums, "zniff-aarch64.dmg").as_deref(),
            Some(hash.as_str())
        );

        let base = parse_url("https://github.com/vicanso/zniff/releases/download/v0.1.0/a.dmg")
            .expect("valid url");
        assert_eq!(
            resolve_redirect(&base, "/login"),
            "https://github.com/login"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
