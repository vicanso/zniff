//! Root CA stored in the user data directory, and per-host leaf certificates.
//!
//! The root is installed into the current user's trust store and kept when
//! the app exits. The private key stays in that directory.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::ServerConfig;
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
};

const CA_NAME: &str = "zniff CA";
/// The previous product name. Still removed so an already-trusted root does not linger.
const LEGACY_CA_NAME: &str = "Snoop CA";

const MAX_CACHED_LEAVES: usize = 256;

pub struct Ca {
    dir: PathBuf,
    /// Subject and key of the installed root. Leaf signing reads these.
    /// The PEM on disk is what gets installed.
    issuer: Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
    session_storage: Arc<dyn rustls::server::StoresServerSessions>,
    leaves: Mutex<Vec<(String, ServerConfig)>>,
}

pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join("Library/Application Support/zniff")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        PathBuf::from(base).join("zniff")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
            && !xdg.is_empty()
        {
            return PathBuf::from(xdg).join("zniff");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".local/share/zniff")
    }
}

impl Ca {
    pub fn open_default() -> Result<Self> {
        Self::open(&data_dir())
    }

    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("创建 {}", dir.display()))?;
        let cert_path = dir.join("ca.crt");
        let key_path = dir.join("ca.key");
        if cert_path.exists() && key_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path)?;
            let key_pem = fs::read_to_string(&key_path)?;
            return Self::from_pem(dir.to_path_buf(), cert_pem, key_pem);
        }
        Self::generate(dir)
    }

    fn generate(dir: &Path) -> Result<Self> {
        let key = KeyPair::generate().context("生成根 CA 密钥")?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, CA_NAME);
        name.push(DnType::OrganizationName, "zniff");
        params.distinguished_name = name;
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let cert = params.self_signed(&key).context("签发根 CA")?;
        let cert_pem = cert.pem();
        let key_pem = key.serialize_pem();
        write_secret(&dir.join("ca.key"), key_pem.as_bytes())?;
        fs::write(dir.join("ca.crt"), &cert_pem)?;
        Self::from_pem(dir.to_path_buf(), cert_pem, key_pem)
    }

    fn from_pem(dir: PathBuf, cert_pem: String, key_pem: String) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let issuer_key = KeyPair::from_pem(&key_pem).context("读取根 CA 私钥")?;
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, issuer_key).context("读取根 CA 证书")?;
        let ca_der = ca_der_from_pem(&cert_pem)?;
        Ok(Self {
            dir,
            issuer,
            ca_der,
            session_storage: rustls::server::ServerSessionMemoryCache::new(64),
            leaves: Mutex::new(Vec::new()),
        })
    }

    pub fn cert_path(&self) -> PathBuf {
        self.dir.join("ca.crt")
    }

    pub fn trust_status(&self) -> CaStatus {
        trust_status(self)
    }

    #[cfg(test)]
    pub fn cert_der(&self) -> CertificateDer<'static> {
        self.ca_der.clone()
    }

    #[cfg(test)]
    pub fn server_config(&self, host: &str, alpn: &[u8]) -> Result<ServerConfig> {
        self.server_config_with_custom(host, alpn, None)
    }

    pub fn server_config_with_custom(
        &self,
        host: &str,
        alpn: &[u8],
        custom: Option<(&Path, Option<&Path>)>,
    ) -> Result<ServerConfig> {
        let dir_found;
        let effective_custom = match custom {
            Some(pair) => Some(pair),
            None => {
                dir_found = self.dir_custom_cert(host);
                dir_found
                    .as_ref()
                    .map(|(cert, key)| (cert.as_path(), key.as_deref()))
            }
        };
        if let Some((cert_path, key_path)) = effective_custom {
            let (certs, key) = load_custom_cert_pair(cert_path, key_path)?;
            let mut config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .with_context(|| format!("组装自定义证书 {}", cert_path.display()))?;
            config.session_storage = Arc::clone(&self.session_storage);
            config.alpn_protocols = vec![alpn.to_vec()];
            return Ok(config);
        }
        let mut config = self.leaf_config(host)?;
        config.alpn_protocols = vec![alpn.to_vec()];
        Ok(config)
    }

    /// Looks up a custom certificate in `<data_dir>/certs/` for `host`
    /// (exact `<host>.crt`/`.pem` + `.key`, or wildcard `_wildcard.<parent>` / `*.<parent>`).
    fn dir_custom_cert(&self, host: &str) -> Option<(PathBuf, Option<PathBuf>)> {
        self.dir_custom_cert_in("certs", host)
    }

    pub fn has_client_cert_in_dir(&self, host: &str) -> bool {
        self.dir_custom_cert_in("client-certs", host).is_some()
    }

    pub fn client_identity_for_host(
        &self,
        host: &str,
        custom: Option<(&Path, Option<&Path>)>,
    ) -> Result<Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>> {
        let dir_found;
        let effective_custom = match custom {
            Some(pair) => Some(pair),
            None => {
                dir_found = self.dir_custom_cert_in("client-certs", host);
                dir_found
                    .as_ref()
                    .map(|(cert, key)| (cert.as_path(), key.as_deref()))
            }
        };
        let Some((cert_path, key_path)) = effective_custom else {
            return Ok(None);
        };
        let (certs, key) = load_custom_cert_pair(cert_path, key_path)
            .with_context(|| format!("读取客户端证书 {}", cert_path.display()))?;
        Ok(Some((certs, key)))
    }

    fn dir_custom_cert_in(&self, subdir: &str, host: &str) -> Option<(PathBuf, Option<PathBuf>)> {
        let certs_dir = self.dir.join(subdir);
        if !certs_dir.is_dir() {
            return None;
        }
        let lower = host.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        let mut stems = vec![lower.clone()];
        if let Some((_, parent)) = lower.split_once('.')
            && !parent.is_empty()
        {
            stems.push(format!("_wildcard.{parent}"));
            stems.push(format!("*.{parent}"));
        }
        for stem in stems {
            let key_candidate = certs_dir.join(format!("{stem}.key"));
            let has_key = key_candidate.is_file();
            for ext in ["crt", "pem"] {
                let cert_candidate = certs_dir.join(format!("{stem}.{ext}"));
                if cert_candidate.is_file() {
                    if has_key {
                        return Some((cert_candidate, Some(key_candidate)));
                    }
                    if ext == "pem" {
                        return Some((cert_candidate, None));
                    }
                }
            }
        }
        None
    }

    fn leaf_config(&self, host: &str) -> Result<ServerConfig> {
        let mut guard = self.leaves.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(pos) = guard.iter().position(|(name, _)| name == host) {
            let entry = guard.remove(pos);
            let cfg = entry.1.clone();
            guard.push(entry);
            return Ok(cfg);
        }
        let cfg = self.sign_leaf_config(host)?;
        while guard.len() >= MAX_CACHED_LEAVES {
            guard.remove(0);
        }
        guard.push((host.to_string(), cfg.clone()));
        Ok(cfg)
    }

    fn sign_leaf_config(&self, host: &str) -> Result<ServerConfig> {
        let leaf_key = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        let common = if host.len() > 64 { &host[..64] } else { host };
        name.push(DnType::CommonName, common);
        params.distinguished_name = name;
        params.subject_alt_names = vec![san_for(host)?];
        params.is_ca = IsCa::NoCa;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.use_authority_key_identifier_extension = true;
        let cert = params
            .signed_by(&leaf_key, &self.issuer)
            .context("签发叶子证书")?;
        let leaf_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![leaf_der, self.ca_der.clone()], key_der)
            .context("组装叶子证书")?;
        config.session_storage = Arc::clone(&self.session_storage);
        Ok(config)
    }
}

fn san_for(host: &str) -> Result<SanType> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SanType::IpAddress(ip));
    }
    let name = host
        .try_into()
        .map_err(|err| anyhow::anyhow!("主机名不能写入证书 {host}: {err}"))?;
    Ok(SanType::DnsName(name))
}

fn ca_der_from_pem(pem: &str) -> Result<CertificateDer<'static>> {
    let mut certs = certs_from_pem(pem)?;
    if certs.is_empty() {
        anyhow::bail!("PEM 中未找到 CERTIFICATE 块");
    }
    Ok(certs.swap_remove(0))
}

fn load_custom_cert_pair(
    cert_path: &Path,
    key_path: Option<&Path>,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert_pem = fs::read_to_string(cert_path)
        .with_context(|| format!("读取证书文件 {}", cert_path.display()))?;
    let certs =
        certs_from_pem(&cert_pem).with_context(|| format!("解析证书链 {}", cert_path.display()))?;
    if certs.is_empty() {
        anyhow::bail!("证书文件 {} 中未找到 CERTIFICATE", cert_path.display());
    }
    let key = match key_path {
        Some(kp) => {
            let key_pem =
                fs::read_to_string(kp).with_context(|| format!("读取私钥文件 {}", kp.display()))?;
            key_from_pem(&key_pem).with_context(|| format!("解析私钥 {}", kp.display()))?
        }
        None => key_from_pem(&cert_pem)
            .with_context(|| format!("解析合并 PEM 私钥 {}", cert_path.display()))?,
    };
    Ok((certs, key))
}

fn certs_from_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut certs = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed == "-----BEGIN CERTIFICATE-----" {
            inside = true;
            current.clear();
        } else if trimmed == "-----END CERTIFICATE-----" {
            if inside {
                let bytes = base64_decode::decode(&current).context("解析证书 PEM Base64")?;
                certs.push(CertificateDer::from(bytes));
                inside = false;
                current.clear();
            }
        } else if inside && !trimmed.starts_with('-') && !trimmed.is_empty() {
            current.push_str(trimmed);
        }
    }
    Ok(certs)
}

fn key_from_pem(pem: &str) -> Result<PrivateKeyDer<'static>> {
    let mut current = String::new();
    let mut kind: Option<&str> = None;
    for line in pem.lines() {
        let trimmed = line.trim();
        if kind.is_none() {
            kind = match trimmed {
                "-----BEGIN PRIVATE KEY-----" => Some("PKCS8"),
                "-----BEGIN RSA PRIVATE KEY-----" => Some("PKCS1"),
                "-----BEGIN EC PRIVATE KEY-----" => Some("SEC1"),
                _ => None,
            };
            if kind.is_some() {
                current.clear();
            }
        } else if trimmed.starts_with("-----END ") && trimmed.ends_with("PRIVATE KEY-----") {
            let bytes = base64_decode::decode(&current).context("解析私钥 PEM Base64")?;
            return Ok(match kind.unwrap_or("PKCS8") {
                "PKCS1" => PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(bytes)),
                "SEC1" => PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(bytes)),
                _ => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(bytes)),
            });
        } else if !trimmed.starts_with('-') && !trimmed.is_empty() {
            current.push_str(trimmed);
        }
    }
    anyhow::bail!("PEM 中未找到私钥块（PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY）")
}

/// Tiny base64 decoder so the CA module does not take another dependency.
mod base64_decode {
    use anyhow::{Result, bail};

    pub fn decode(text: &str) -> Result<Vec<u8>> {
        fn val(byte: u8) -> Result<u8> {
            Ok(match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => bail!("CA PEM 里有非法字符"),
            })
        }
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let (a, b, c, d) = (bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]);
            let av = val(a)?;
            let bv = val(b)?;
            out.push((av << 2) | (bv >> 4));
            if c != b'=' {
                let cv = val(c)?;
                out.push((bv << 4) | (cv >> 2));
                if d != b'=' {
                    let dv = val(d)?;
                    out.push((cv << 6) | dv);
                }
            }
            i += 4;
        }
        Ok(out)
    }
}

fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Installs the root into the current user's trust store.
pub fn install(ca: &Ca) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        install_macos(ca)
    }
    #[cfg(target_os = "windows")]
    {
        install_windows(ca)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        install_linux(ca)
    }
}

pub fn remove(ca: &Ca) -> Result<String> {
    let _ = ca.cert_path();
    #[cfg(target_os = "macos")]
    {
        let current = delete_named_cert(CA_NAME)?;
        let legacy = delete_named_cert(LEGACY_CA_NAME)?;
        if current || legacy {
            Ok("已从登录钥匙串移除".into())
        } else {
            anyhow::bail!("登录钥匙串里没有 {CA_NAME}")
        }
    }
    #[cfg(target_os = "windows")]
    {
        remove_windows()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        remove_linux()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaStatus {
    Trusted,
    InstalledUntrusted,
    NotInstalled,
}

impl CaStatus {
    pub fn is_installed(self) -> bool {
        !matches!(self, Self::NotInstalled)
    }
}

pub fn classify_trust_status(installed: bool, verified: bool) -> CaStatus {
    if !installed {
        CaStatus::NotInstalled
    } else if verified {
        CaStatus::Trusted
    } else {
        CaStatus::InstalledUntrusted
    }
}

/// Best-effort: checks whether the certificate is installed and trusted in the user store.
pub fn trust_status(ca: &Ca) -> CaStatus {
    let installed = is_installed();
    if !installed {
        return CaStatus::NotInstalled;
    }
    #[cfg(target_os = "macos")]
    {
        let verified = ca
            .cert_path()
            .to_str()
            .map(|cert_path| {
                std::process::Command::new("security")
                    .args(macos_verify_cert_args(cert_path))
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
            || macos_has_trust_root(CA_NAME);
        classify_trust_status(installed, verified)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ca;
        classify_trust_status(installed, installed)
    }
}

/// Best-effort: the certificate is present in the user store.
pub fn is_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("security")
            .args(["find-certificate", "-c", CA_NAME, &login_keychain()])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        windows_has_cert(CA_NAME)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let db = linux_nss_db(&home);
        std::process::Command::new("certutil")
            .args(linux_certutil_list_args(&db, CA_NAME))
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
fn windows_has_cert(name: &str) -> bool {
    std::process::Command::new("certutil")
        .args(windows_certutil_verify_args(name))
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn macos_verify_cert_args(cert_path: &str) -> [&str; 4] {
    ["verify-cert", "-c", cert_path, "-L"]
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn macos_add_trusted_cert_args<'a>(keychain: &'a str, cert_path: &'a str) -> [&'a str; 6] {
    [
        "add-trusted-cert",
        "-r",
        "trustRoot",
        "-k",
        keychain,
        cert_path,
    ]
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn parse_macos_trust_settings(dump: &str, cert_name: &str) -> bool {
    let mut in_target = false;
    let mut has_trust_root = false;
    for line in dump.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Cert ")
            && let Some((_, name)) = rest.split_once(':')
        {
            if in_target && has_trust_root {
                return true;
            }
            in_target = name.trim() == cert_name;
            has_trust_root = false;
            continue;
        }
        if !in_target {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Result Type")
            && let Some((_, result)) = rest.split_once(':')
        {
            let val = result.trim();
            if val == "kSecTrustSettingsResultDeny" {
                return false;
            }
            if val == "kSecTrustSettingsResultTrustRoot"
                || val == "kSecTrustSettingsResultTrustAsRoot"
            {
                has_trust_root = true;
            }
        }
    }
    in_target && has_trust_root
}

#[cfg(target_os = "macos")]
fn macos_has_trust_root(cert_name: &str) -> bool {
    for args in [
        &["dump-trust-settings"][..],
        &["dump-trust-settings", "-d"][..],
    ] {
        if let Ok(output) = std::process::Command::new("security").args(args).output()
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if parse_macos_trust_settings(&stdout, cert_name) {
                return true;
            }
        }
    }
    false
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn windows_certutil_verify_args(name: &str) -> [&str; 4] {
    ["-user", "-verifystore", "Root", name]
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn windows_certutil_del_args(name: &str) -> [&str; 4] {
    ["-user", "-delstore", "Root", name]
}

#[cfg_attr(not(any(all(unix, not(target_os = "macos")), test)), allow(dead_code))]
fn linux_nss_db(home: &str) -> String {
    format!("sql:{home}/.pki/nssdb")
}

#[cfg_attr(not(any(all(unix, not(target_os = "macos")), test)), allow(dead_code))]
fn linux_certutil_list_args<'a>(db: &'a str, name: &'a str) -> [&'a str; 5] {
    ["-d", db, "-L", "-n", name]
}

#[cfg_attr(not(any(all(unix, not(target_os = "macos")), test)), allow(dead_code))]
fn linux_certutil_del_args<'a>(db: &'a str, name: &'a str) -> [&'a str; 5] {
    ["-d", db, "-D", "-n", name]
}

#[cfg(target_os = "macos")]
fn install_macos(ca: &Ca) -> Result<String> {
    let keychain = login_keychain();
    let cert_path = ca.cert_path();
    let cert_str = cert_path.to_str().context("证书路径不是 UTF-8")?;
    let status = std::process::Command::new("security")
        .args(macos_add_trusted_cert_args(&keychain, cert_str))
        .status()
        .context("运行 security add-trusted-cert")?;
    if status.success() {
        Ok("已装进登录钥匙串".into())
    } else {
        anyhow::bail!("security add-trusted-cert 退出码 {}", status)
    }
}

#[cfg(target_os = "macos")]
fn login_keychain() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Keychains/login.keychain-db")
}

#[cfg(target_os = "macos")]
fn delete_named_cert(name: &str) -> Result<bool> {
    let output = std::process::Command::new("security")
        .args(["delete-certificate", "-c", name, &login_keychain()])
        .output()
        .with_context(|| format!("运行 security delete-certificate {name}"))?;
    Ok(output.status.success())
}

#[cfg(target_os = "windows")]
fn install_windows(ca: &Ca) -> Result<String> {
    let status = std::process::Command::new("certutil")
        .args([
            "-user",
            "-addstore",
            "Root",
            ca.cert_path().to_str().unwrap_or("ca.crt"),
        ])
        .status()
        .context("运行 certutil")?;
    if status.success() {
        Ok("已装进当前用户的 Root 存储".into())
    } else {
        anyhow::bail!("certutil 退出码 {}", status)
    }
}

#[cfg(target_os = "windows")]
fn remove_windows() -> Result<String> {
    let current = delete_windows_cert(CA_NAME)?;
    let legacy = delete_windows_cert(LEGACY_CA_NAME)?;
    if current || legacy {
        Ok("已从当前用户的 Root 存储移除".into())
    } else {
        anyhow::bail!("当前用户的 Root 存储里没有 {CA_NAME}")
    }
}

#[cfg(target_os = "windows")]
fn delete_windows_cert(name: &str) -> Result<bool> {
    if !windows_has_cert(name) {
        return Ok(false);
    }
    let output = std::process::Command::new("certutil")
        .args(windows_certutil_del_args(name))
        .output()
        .with_context(|| format!("运行 certutil -delstore {name}"))?;
    Ok(output.status.success())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install_linux(ca: &Ca) -> Result<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let db = linux_nss_db(&home);
    let _ = std::process::Command::new("mkdir")
        .args(["-p", &format!("{home}/.pki/nssdb")])
        .status();
    let status = std::process::Command::new("certutil")
        .args([
            "-d",
            &db,
            "-A",
            "-t",
            "C,,",
            "-n",
            CA_NAME,
            "-i",
            ca.cert_path().to_str().unwrap_or("ca.crt"),
        ])
        .status()
        .context("运行 certutil（NSS）")?;
    if status.success() {
        Ok("已装进 ~/.pki/nssdb".into())
    } else {
        anyhow::bail!("NSS certutil 退出码 {}", status)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn remove_linux() -> Result<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let db = linux_nss_db(&home);
    let current = delete_linux_cert(&db, CA_NAME)?;
    let legacy = delete_linux_cert(&db, LEGACY_CA_NAME)?;
    if current || legacy {
        Ok("已从 ~/.pki/nssdb 移除".into())
    } else {
        anyhow::bail!("~/.pki/nssdb 里没有 {CA_NAME}")
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn delete_linux_cert(db: &str, name: &str) -> Result<bool> {
    let output = std::process::Command::new("certutil")
        .args(linux_certutil_del_args(db, name))
        .output()
        .with_context(|| format!("运行 certutil（NSS）移除 {name}"))?;
    Ok(output.status.success())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IosSimOutcome {
    Installed(usize),
    NoneBooted,
}

pub fn simctl_add_root_cert_args<'a>(target: &'a str, cert_path: &'a str) -> [&'a str; 5] {
    ["simctl", "keychain", target, "add-root-cert", cert_path]
}

pub fn parse_booted_simulators(output: &str) -> Vec<(String, String)> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_suffix("(Booted)")
            .or_else(|| trimmed.strip_suffix("(booted)"))
        else {
            continue;
        };
        let rest = rest.trim_end();
        let Some(rest) = rest.strip_suffix(')') else {
            continue;
        };
        let Some(open_idx) = rest.rfind('(') else {
            continue;
        };
        let name = rest[..open_idx].trim();
        let udid = rest[open_idx + 1..].trim();
        if !name.is_empty()
            && udid.len() >= 8
            && udid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            devices.push((name.to_string(), udid.to_string()));
        }
    }
    devices
}

pub fn install_ios_simulators(ca: &Ca) -> Result<IosSimOutcome> {
    let cert_path = ca.cert_path();
    let cert_str = cert_path.to_str().context("证书路径不是 UTF-8")?;
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
        .context("运行 xcrun simctl list devices booted")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            anyhow::bail!("xcrun simctl list 退出码 {}", output.status);
        } else {
            anyhow::bail!("{detail}");
        }
    }
    let booted = parse_booted_simulators(&String::from_utf8_lossy(&output.stdout));
    if booted.is_empty() {
        return Ok(IosSimOutcome::NoneBooted);
    }
    for (name, udid) in &booted {
        let out = std::process::Command::new("xcrun")
            .args(simctl_add_root_cert_args(udid, cert_str))
            .output()
            .with_context(|| format!("向模拟器 {name} ({udid}) 安装证书"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.trim();
            if detail.is_empty() {
                anyhow::bail!("模拟器 {name} 退出码 {}", out.status);
            } else {
                anyhow::bail!("模拟器 {name}: {detail}");
            }
        }
    }
    Ok(IosSimOutcome::Installed(booted.len()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinuxDistroFamily {
    Arch,
    Debian,
    RedHat,
    Suse,
    Alpine,
}

pub fn detect_linux_distro_from(
    os_release: Option<&str>,
    has_arch_release: bool,
    has_redhat_release: bool,
) -> LinuxDistroFamily {
    if let Some(text) = os_release {
        let mut tokens = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(val) = line
                .strip_prefix("ID=")
                .or_else(|| line.strip_prefix("ID_LIKE="))
            {
                let unquoted = val.trim().trim_matches('"').trim_matches('\'');
                for word in unquoted.split_whitespace() {
                    tokens.push(word.to_ascii_lowercase());
                }
            }
        }
        for tok in &tokens {
            match tok.as_str() {
                "arch" | "manjaro" | "endeavouros" | "cachyos" => return LinuxDistroFamily::Arch,
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" => {
                    return LinuxDistroFamily::RedHat;
                }
                "suse" | "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => {
                    return LinuxDistroFamily::Suse;
                }
                "alpine" => return LinuxDistroFamily::Alpine,
                "debian" | "ubuntu" | "linuxmint" | "pop" | "kali" | "elementary" | "raspbian" => {
                    return LinuxDistroFamily::Debian;
                }
                _ => {}
            }
        }
    }
    if has_arch_release {
        LinuxDistroFamily::Arch
    } else if has_redhat_release {
        LinuxDistroFamily::RedHat
    } else {
        LinuxDistroFamily::Debian
    }
}

fn shell_quote_path(path: &str) -> String {
    if !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_'))
    {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

pub fn linux_system_ca_command_for(family: LinuxDistroFamily, cert_path: &str) -> String {
    let quoted = shell_quote_path(cert_path);
    match family {
        LinuxDistroFamily::Arch => format!("sudo trust anchor --store {quoted}"),
        LinuxDistroFamily::RedHat => format!(
            "sudo cp {quoted} /etc/pki/ca-trust/source/anchors/zniff-ca.crt && sudo update-ca-trust"
        ),
        LinuxDistroFamily::Suse => format!(
            "sudo cp {quoted} /etc/pki/trust/anchors/zniff-ca.crt && sudo update-ca-certificates"
        ),
        LinuxDistroFamily::Alpine | LinuxDistroFamily::Debian => format!(
            "sudo cp {quoted} /usr/local/share/ca-certificates/zniff-ca.crt && sudo update-ca-certificates"
        ),
    }
}

pub fn linux_system_ca_command(cert_path: &Path) -> String {
    let os_release = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .ok();
    let family = detect_linux_distro_from(
        os_release.as_deref(),
        Path::new("/etc/arch-release").exists(),
        Path::new("/etc/redhat-release").exists(),
    );
    linux_system_ca_command_for(family, &cert_path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_leaf_for_a_host() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("snoop-ca-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let ca = Ca::open(&dir).unwrap();
        let config = ca.server_config("example.com", b"http/1.1").unwrap();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);

        // Verify custom certificate loading via explicit paths and via `<data_dir>/certs/`
        let custom_key = KeyPair::generate().unwrap();
        let mut custom_params = CertificateParams::default();
        custom_params.subject_alt_names = vec![san_for("pin.example.com").unwrap()];
        let custom_cert = custom_params.self_signed(&custom_key).unwrap();
        let cert_file = dir.join("pin.crt");
        let key_file = dir.join("pin.key");
        fs::write(&cert_file, custom_cert.pem()).unwrap();
        fs::write(&key_file, custom_key.serialize_pem()).unwrap();
        let custom_cfg = ca
            .server_config_with_custom(
                "pin.example.com",
                b"h2",
                Some((&cert_file, Some(&key_file))),
            )
            .unwrap();
        assert_eq!(custom_cfg.alpn_protocols, vec![b"h2".to_vec()]);

        let certs_dir = dir.join("certs");
        fs::create_dir_all(&certs_dir).unwrap();
        let bundle_file = certs_dir.join("_wildcard.example.com.pem");
        fs::write(
            &bundle_file,
            format!("{}\n{}", custom_cert.pem(), custom_key.serialize_pem()),
        )
        .unwrap();
        assert!(ca.dir_custom_cert("sub.example.com").is_some());
        let dir_cfg = ca.server_config("sub.example.com", b"http/1.1").unwrap();
        assert_eq!(dir_cfg.alpn_protocols, vec![b"http/1.1".to_vec()]);

        let client_certs_dir = dir.join("client-certs");
        fs::create_dir_all(&client_certs_dir).unwrap();
        fs::write(
            client_certs_dir.join("mtls.example.com.pem"),
            format!("{}\n{}", custom_cert.pem(), custom_key.serialize_pem()),
        )
        .unwrap();
        assert!(ca.has_client_cert_in_dir("mtls.example.com"));
        assert!(
            ca.client_identity_for_host("mtls.example.com", None)
                .unwrap()
                .is_some()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn certutil_args_target_user_store_and_nss_db() {
        assert_eq!(
            windows_certutil_verify_args(CA_NAME),
            ["-user", "-verifystore", "Root", "zniff CA"]
        );
        assert_eq!(
            windows_certutil_del_args(LEGACY_CA_NAME),
            ["-user", "-delstore", "Root", "Snoop CA"]
        );
        let db = linux_nss_db("/home/alice");
        assert_eq!(db, "sql:/home/alice/.pki/nssdb");
        assert_eq!(
            linux_certutil_list_args(&db, CA_NAME),
            ["-d", "sql:/home/alice/.pki/nssdb", "-L", "-n", "zniff CA"]
        );
        assert_eq!(
            linux_certutil_del_args(&db, CA_NAME),
            ["-d", "sql:/home/alice/.pki/nssdb", "-D", "-n", "zniff CA"]
        );
        assert_eq!(
            macos_verify_cert_args("/tmp/zniff/ca.crt"),
            ["verify-cert", "-c", "/tmp/zniff/ca.crt", "-L"]
        );
        assert_eq!(
            macos_add_trusted_cert_args(
                "/Users/alice/Library/Keychains/login.keychain-db",
                "/tmp/zniff/ca.crt"
            ),
            [
                "add-trusted-cert",
                "-r",
                "trustRoot",
                "-k",
                "/Users/alice/Library/Keychains/login.keychain-db",
                "/tmp/zniff/ca.crt"
            ]
        );
    }

    #[test]
    fn classify_trust_status_distinguishes_installed_untrusted() {
        assert_eq!(classify_trust_status(false, false), CaStatus::NotInstalled);
        assert_eq!(classify_trust_status(false, true), CaStatus::NotInstalled);
        assert_eq!(
            classify_trust_status(true, false),
            CaStatus::InstalledUntrusted
        );
        assert_eq!(classify_trust_status(true, true), CaStatus::Trusted);
        assert!(!CaStatus::NotInstalled.is_installed());
        assert!(CaStatus::InstalledUntrusted.is_installed());
        assert!(CaStatus::Trusted.is_installed());

        let ssl_only_dump = "\
Number of certs: 2
Cert 0: Other Root CA
   Number of trust settings : 0
Cert 1: zniff CA
   Number of trust settings : 1
   Trust Setting 0:
      Policy OID            : SSL
      Result Type           : kSecTrustSettingsResultTrustRoot
";
        assert!(parse_macos_trust_settings(ssl_only_dump, CA_NAME));
        assert!(!parse_macos_trust_settings(ssl_only_dump, "Other Root CA"));

        let deny_dump = "\
Cert 0: zniff CA
   Number of trust settings : 1
   Trust Setting 0:
      Result Type           : kSecTrustSettingsResultDeny
";
        assert!(!parse_macos_trust_settings(deny_dump, CA_NAME));
    }

    #[test]
    fn parses_booted_simulators_and_builds_simctl_args() {
        let sample = "\
== Devices ==
-- iOS 17.5 --
    iPhone 15 (11111111-2222-3333-4444-555555555555) (Shutdown)
    iPhone 15 Pro (AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE) (Booted)
-- iOS 18.0 --
    iPad Pro 13-inch (M4) (12345678-9ABC-DEF0-1234-56789ABCDEF0) (Booted)
";
        let booted = parse_booted_simulators(sample);
        assert_eq!(
            booted,
            vec![
                (
                    "iPhone 15 Pro".to_string(),
                    "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE".to_string()
                ),
                (
                    "iPad Pro 13-inch (M4)".to_string(),
                    "12345678-9ABC-DEF0-1234-56789ABCDEF0".to_string()
                ),
            ]
        );
        assert_eq!(
            simctl_add_root_cert_args("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE", "/tmp/ca.crt"),
            [
                "simctl",
                "keychain",
                "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
                "add-root-cert",
                "/tmp/ca.crt"
            ]
        );
    }

    #[test]
    fn detects_linux_distro_and_builds_system_ca_command() {
        assert_eq!(
            detect_linux_distro_from(Some("ID=ubuntu\nID_LIKE=debian\n"), false, false),
            LinuxDistroFamily::Debian
        );
        assert_eq!(
            detect_linux_distro_from(Some("ID=\"manjaro\"\nID_LIKE=arch\n"), false, false),
            LinuxDistroFamily::Arch
        );
        assert_eq!(
            detect_linux_distro_from(Some("ID=fedora\n"), false, false),
            LinuxDistroFamily::RedHat
        );
        assert_eq!(
            detect_linux_distro_from(Some("ID=\"opensuse-tumbleweed\"\n"), false, false),
            LinuxDistroFamily::Suse
        );
        assert_eq!(
            detect_linux_distro_from(None, true, false),
            LinuxDistroFamily::Arch
        );
        assert_eq!(
            linux_system_ca_command_for(
                LinuxDistroFamily::Debian,
                "/home/alice/.local/share/zniff/ca.crt"
            ),
            "sudo cp /home/alice/.local/share/zniff/ca.crt /usr/local/share/ca-certificates/zniff-ca.crt && sudo update-ca-certificates"
        );
        assert_eq!(
            linux_system_ca_command_for(
                LinuxDistroFamily::Arch,
                "/home/alice/My Config/zniff/ca.crt"
            ),
            "sudo trust anchor --store '/home/alice/My Config/zniff/ca.crt'"
        );
        assert_eq!(
            linux_system_ca_command_for(LinuxDistroFamily::RedHat, "/tmp/ca.crt"),
            "sudo cp /tmp/ca.crt /etc/pki/ca-trust/source/anchors/zniff-ca.crt && sudo update-ca-trust"
        );
    }
}
