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
const MAX_CACHED_CUSTOM_CERTS: usize = 64;

struct CachedCustomCert {
    cert_path: PathBuf,
    key_path: Option<PathBuf>,
    cert_mtime: std::time::SystemTime,
    key_mtime: Option<std::time::SystemTime>,
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

pub struct Ca {
    dir: PathBuf,
    /// Subject and key of the installed root. Leaf signing reads these.
    /// The PEM on disk is what gets installed.
    issuer: Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
    session_storage: Arc<dyn rustls::server::StoresServerSessions>,
    leaves: Mutex<Vec<(String, ServerConfig)>>,
    custom_certs: Mutex<Vec<CachedCustomCert>>,
}

static DATA_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Overrides [`data_dir`] for the current process (used by `--data-dir`).
pub fn set_data_dir_override(dir: impl Into<PathBuf>) {
    let path = dir.into();
    if !path.as_os_str().is_empty() {
        *DATA_DIR_OVERRIDE
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = Some(path);
    }
}

fn resolve_custom_data_dir(override_dir: Option<&Path>, env_dir: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = override_dir.filter(|p| !p.as_os_str().is_empty()) {
        return Some(dir.to_path_buf());
    }
    let trimmed = env_dir?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn default_platform_data_dir() -> PathBuf {
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

pub fn data_dir() -> PathBuf {
    let override_dir = DATA_DIR_OVERRIDE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let env_dir = std::env::var("ZNIFF_DATA_DIR").ok();
    resolve_custom_data_dir(override_dir.as_deref(), env_dir.as_deref())
        .unwrap_or_else(default_platform_data_dir)
}

impl Ca {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

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
            custom_certs: Mutex::new(Vec::new()),
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
            let (certs, key) = self.load_cached_custom_cert_pair(cert_path, key_path)?;
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
        let (certs, key) = self
            .load_cached_custom_cert_pair(cert_path, key_path)
            .with_context(|| format!("读取客户端证书 {}", cert_path.display()))?;
        Ok(Some((certs, key)))
    }

    fn load_cached_custom_cert_pair(
        &self,
        cert_path: &Path,
        key_path: Option<&Path>,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
        let cert_mtime = fs::metadata(cert_path).and_then(|m| m.modified()).ok();
        let key_mtime = key_path.map(|kp| fs::metadata(kp).and_then(|m| m.modified()).ok());
        if let Some(c_mtime) = cert_mtime
            && key_mtime.is_none_or(|m| m.is_some())
        {
            let flat_key_mtime = key_mtime.flatten();
            {
                let mut guard = self
                    .custom_certs
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                if let Some(pos) = guard.iter().position(|entry| {
                    entry.cert_path == cert_path
                        && entry.key_path.as_deref() == key_path
                        && entry.cert_mtime == c_mtime
                        && entry.key_mtime == flat_key_mtime
                }) {
                    let entry = guard.remove(pos);
                    let out = (entry.certs.clone(), entry.key.clone_key());
                    guard.push(entry);
                    return Ok(out);
                }
            }
            let (certs, key) = load_custom_cert_pair(cert_path, key_path)?;
            let mut guard = self
                .custom_certs
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            while guard.len() >= MAX_CACHED_CUSTOM_CERTS {
                guard.remove(0);
            }
            guard.push(CachedCustomCert {
                cert_path: cert_path.to_path_buf(),
                key_path: key_path.map(Path::to_path_buf),
                cert_mtime: c_mtime,
                key_mtime: flat_key_mtime,
                certs: certs.clone(),
                key: key.clone_key(),
            });
            return Ok((certs, key));
        }
        load_custom_cert_pair(cert_path, key_path)
    }

    fn dir_custom_cert_in(&self, subdir: &str, host: &str) -> Option<(PathBuf, Option<PathBuf>)> {
        let certs_dir = self.dir.join(subdir);
        if !certs_dir.is_dir() {
            return None;
        }
        let lower = normalize_ca_host(host);
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
        let normalized = normalize_ca_host(host);
        let target = if normalized.is_empty() {
            host.trim()
        } else {
            normalized.as_str()
        };
        let mut guard = self.leaves.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(pos) = guard.iter().position(|(name, _)| name == target) {
            let entry = guard.remove(pos);
            let cfg = entry.1.clone();
            guard.push(entry);
            return Ok(cfg);
        }
        let cfg = self.sign_leaf_config(target)?;
        while guard.len() >= MAX_CACHED_LEAVES {
            guard.remove(0);
        }
        guard.push((target.to_string(), cfg.clone()));
        Ok(cfg)
    }

    fn sign_leaf_config(&self, host: &str) -> Result<ServerConfig> {
        let leaf_key = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        let common = &host[..host.floor_char_boundary(64)];
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

fn normalize_ca_host(host: &str) -> String {
    host.trim()
        .trim_matches(|ch| ch == '[' || ch == ']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
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
    let mut has_encrypted_pkcs8 = false;
    let mut has_legacy_encrypted = false;
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed == "-----BEGIN ENCRYPTED PRIVATE KEY-----"
            || (trimmed.starts_with("-----BEGIN ") && trimmed.contains("ENCRYPTED PRIVATE KEY"))
        {
            has_encrypted_pkcs8 = true;
        }
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
            if has_legacy_encrypted {
                anyhow::bail!(
                    "私钥已加密（Proc-Type: 4,ENCRYPTED），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl rsa -in encrypted.key -out decrypted.key）"
                );
            }
            let bytes = base64_decode::decode(&current).context("解析私钥 PEM Base64")?;
            return Ok(match kind.unwrap_or("PKCS8") {
                "PKCS1" => PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(bytes)),
                "SEC1" => PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(bytes)),
                _ => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(bytes)),
            });
        } else if kind.is_some() && trimmed.starts_with("Proc-Type: 4,ENCRYPTED") {
            has_legacy_encrypted = true;
        } else if !trimmed.starts_with('-') && !trimmed.is_empty() {
            current.push_str(trimmed);
        }
    }
    if has_encrypted_pkcs8 {
        anyhow::bail!(
            "私钥已加密（ENCRYPTED PRIVATE KEY），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl pkcs8 -in encrypted.key -out decrypted.key）"
        );
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
    Installed { count: usize, port: u16 },
    NoneBooted,
}

pub fn simctl_add_root_cert_args<'a>(target: &'a str, cert_path: &'a str) -> [&'a str; 5] {
    ["simctl", "keychain", target, "add-root-cert", cert_path]
}

/// Simulator shares the Mac network stack, so this host is the Mac itself.
/// `simctl` has no proxy subcommand; CFNetwork reads these global defaults.
pub const IOS_SIM_PROXY_HOST: &str = "127.0.0.1";

pub fn simctl_set_proxy_args(udid: &str, port: u16) -> Vec<Vec<String>> {
    let port = port.to_string();
    vec![
        simctl_defaults_write(udid, "HTTPEnable", "-int", "1"),
        simctl_defaults_write(udid, "HTTPProxy", "-string", IOS_SIM_PROXY_HOST),
        simctl_defaults_write(udid, "HTTPPort", "-int", &port),
        simctl_defaults_write(udid, "HTTPSEnable", "-int", "1"),
        simctl_defaults_write(udid, "HTTPSProxy", "-string", IOS_SIM_PROXY_HOST),
        simctl_defaults_write(udid, "HTTPSPort", "-int", &port),
    ]
}

pub fn simctl_clear_proxy_args(udid: &str) -> Vec<Vec<String>> {
    vec![
        simctl_defaults_write(udid, "HTTPEnable", "-int", "0"),
        simctl_defaults_write(udid, "HTTPSEnable", "-int", "0"),
        simctl_defaults_delete(udid, "HTTPProxy"),
        simctl_defaults_delete(udid, "HTTPPort"),
        simctl_defaults_delete(udid, "HTTPSProxy"),
        simctl_defaults_delete(udid, "HTTPSPort"),
    ]
}

fn simctl_defaults_write(udid: &str, key: &str, flag: &str, value: &str) -> Vec<String> {
    [
        "simctl", "spawn", udid, "defaults", "write", "-g", key, flag, value,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn simctl_defaults_delete(udid: &str, key: &str) -> Vec<String> {
    ["simctl", "spawn", udid, "defaults", "delete", "-g", key]
        .into_iter()
        .map(str::to_string)
        .collect()
}

static IOS_SIM_PROXY_UDIDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn remember_ios_sim_proxies(udids: &[String]) {
    let mut kept = IOS_SIM_PROXY_UDIDS
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    for udid in udids {
        if !kept.iter().any(|have| have == udid) {
            kept.push(udid.clone());
        }
    }
}

/// Turns HTTP and HTTPS proxy off on simulators this process configured.
/// Missing keys and a missing `xcrun` are ignored.
pub fn clear_ios_simulator_proxies() {
    let udids = {
        let mut kept = IOS_SIM_PROXY_UDIDS
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *kept)
    };
    for udid in udids {
        for args in simctl_clear_proxy_args(&udid) {
            let _ = std::process::Command::new("xcrun").args(&args).output();
        }
    }
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

pub fn install_ios_simulators(ca: &Ca, port: u16) -> Result<IosSimOutcome> {
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
    let mut proxied = Vec::new();
    for (name, udid) in &booted {
        if let Err(err) = run_xcrun(
            simctl_add_root_cert_args(udid, cert_str),
            &format!("向模拟器 {name} ({udid}) 安装证书"),
        ) {
            remember_ios_sim_proxies(&proxied);
            return Err(err);
        }
        if port > 0 {
            proxied.push(udid.clone());
            for args in simctl_set_proxy_args(udid, port) {
                if let Err(err) = run_xcrun(&args, &format!("向模拟器 {name} 设置 HTTP/HTTPS 代理"))
                {
                    remember_ios_sim_proxies(&proxied);
                    return Err(err);
                }
            }
        }
    }
    remember_ios_sim_proxies(&proxied);
    Ok(IosSimOutcome::Installed {
        count: booted.len(),
        port,
    })
}

fn run_xcrun<I, S>(args: I, context: &str) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = std::process::Command::new("xcrun")
        .args(args)
        .output()
        .with_context(|| context.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        anyhow::bail!("{context}，退出码 {}", output.status);
    }
    anyhow::bail!("{context}: {detail}");
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
        let udid = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE";
        let set = simctl_set_proxy_args(udid, 8877);
        assert_eq!(
            set,
            vec![
                vec![
                    "simctl",
                    "spawn",
                    udid,
                    "defaults",
                    "write",
                    "-g",
                    "HTTPEnable",
                    "-int",
                    "1"
                ],
                vec![
                    "simctl",
                    "spawn",
                    udid,
                    "defaults",
                    "write",
                    "-g",
                    "HTTPProxy",
                    "-string",
                    "127.0.0.1"
                ],
                vec![
                    "simctl", "spawn", udid, "defaults", "write", "-g", "HTTPPort", "-int", "8877"
                ],
                vec![
                    "simctl",
                    "spawn",
                    udid,
                    "defaults",
                    "write",
                    "-g",
                    "HTTPSEnable",
                    "-int",
                    "1"
                ],
                vec![
                    "simctl",
                    "spawn",
                    udid,
                    "defaults",
                    "write",
                    "-g",
                    "HTTPSProxy",
                    "-string",
                    "127.0.0.1"
                ],
                vec![
                    "simctl",
                    "spawn",
                    udid,
                    "defaults",
                    "write",
                    "-g",
                    "HTTPSPort",
                    "-int",
                    "8877"
                ],
            ]
            .into_iter()
            .map(|args| args.into_iter().map(str::to_string).collect::<Vec<_>>())
            .collect::<Vec<_>>()
        );
        let clear = simctl_clear_proxy_args(udid);
        assert_eq!(clear.len(), 6);
        assert_eq!(
            clear[0],
            [
                "simctl",
                "spawn",
                udid,
                "defaults",
                "write",
                "-g",
                "HTTPEnable",
                "-int",
                "0"
            ]
            .map(str::to_string)
        );
        assert_eq!(
            clear[1],
            [
                "simctl",
                "spawn",
                udid,
                "defaults",
                "write",
                "-g",
                "HTTPSEnable",
                "-int",
                "0"
            ]
            .map(str::to_string)
        );
        assert!(clear[2..].iter().all(|args| args[4] == "delete"));
        assert_eq!(
            clear
                .iter()
                .map(|args| args[6].as_str())
                .collect::<Vec<_>>(),
            vec![
                "HTTPEnable",
                "HTTPSEnable",
                "HTTPProxy",
                "HTTPPort",
                "HTTPSProxy",
                "HTTPSPort",
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

    #[test]
    fn key_from_pem_detects_encrypted_pkcs8_key() {
        let pem = "-----BEGIN ENCRYPTED PRIVATE KEY-----\n\
                   MIGbMF8GCSqGSIb3DQEFDTBKMCkGCSqGSIb3DQEFDDAcBAi+...\n\
                   -----END ENCRYPTED PRIVATE KEY-----\n";
        let err = key_from_pem(pem).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("私钥已加密"), "expected encrypted hint: {msg}");
        assert!(
            msg.contains("ENCRYPTED PRIVATE KEY"),
            "expected PKCS#8 hint: {msg}"
        );
        assert!(
            msg.contains("openssl pkcs8 -in encrypted.key -out decrypted.key"),
            "expected openssl remediation command: {msg}"
        );
    }

    #[test]
    fn key_from_pem_detects_legacy_openssl_encrypted_key() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\n\
                   Proc-Type: 4,ENCRYPTED\n\
                   DEK-Info: DES-EDE3-CBC,1234567812345678\n\
                   \n\
                   AAAA\n\
                   -----END RSA PRIVATE KEY-----\n";
        let err = key_from_pem(pem).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("私钥已加密"), "expected encrypted hint: {msg}");
        assert!(
            msg.contains("Proc-Type: 4,ENCRYPTED"),
            "expected legacy hint: {msg}"
        );
        assert!(
            msg.contains("openssl rsa"),
            "expected openssl rsa hint: {msg}"
        );
    }

    #[test]
    fn key_from_pem_reports_missing_key_block_when_only_cert_given() {
        let pem = "-----BEGIN CERTIFICATE-----\n\
                   MIIB...fakecert...\n\
                   -----END CERTIFICATE-----\n";
        let err = key_from_pem(pem).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("PEM 中未找到私钥块"),
            "expected missing key block message: {msg}"
        );
    }

    #[test]
    fn host_normalization_utf8_cn_truncation_and_custom_cert_mtime_cache() {
        let dir = std::env::temp_dir().join(format!(
            "zniff-ca-norm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ca = Ca::open(&dir).unwrap();

        // Trailing dot, uppercase, and bracketed IPv6 normalize to the same leaf cache entry.
        let _ = ca.server_config("Example.COM.", b"http/1.1").unwrap();
        let _ = ca.server_config("example.com", b"http/1.1").unwrap();
        assert_eq!(ca.leaves.lock().unwrap().len(), 1);
        let _ = ca.server_config("[::1]", b"http/1.1").unwrap();
        assert_eq!(ca.leaves.lock().unwrap().len(), 2);

        // > 64 byte multi-byte UTF-8 hostname must not panic at non-char boundary (index 64).
        // 22 * 3 bytes = 66 bytes ("测" is 3 bytes in UTF-8, so byte 64 is inside a character).
        let utf8_host = "测".repeat(22);
        assert_eq!(utf8_host.len(), 66);
        assert!(!utf8_host.is_char_boundary(64));
        // `san_for` rejects raw non-ASCII DNS names, while CommonName truncation runs before `san_for` without panicking.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = ca.sign_leaf_config(&utf8_host);
        }));
        assert!(
            res.is_ok(),
            "sign_leaf_config must not panic on UTF-8 boundary"
        );

        // Custom cert cache hits on unchanged mtime and reloads when cert/key file changes.
        let k1 = KeyPair::generate().unwrap();
        let c1 = CertificateParams::new(vec!["custom.example.com".to_string()])
            .unwrap()
            .self_signed(&k1)
            .unwrap();
        let pem_path = dir.join("custom.pem");
        fs::write(&pem_path, format!("{}\n{}", c1.pem(), k1.serialize_pem())).unwrap();

        let (chain1, _) = ca
            .client_identity_for_host("custom.example.com", Some((&pem_path, None)))
            .unwrap()
            .unwrap();
        let (chain1_cached, _) = ca
            .client_identity_for_host("custom.example.com", Some((&pem_path, None)))
            .unwrap()
            .unwrap();
        assert_eq!(chain1, chain1_cached);
        assert_eq!(ca.custom_certs.lock().unwrap().len(), 1);

        // Replace PEM on disk and bump mtime so cache invalidates cleanly.
        let k2 = KeyPair::generate().unwrap();
        let c2 = CertificateParams::new(vec!["custom.example.com".to_string()])
            .unwrap()
            .self_signed(&k2)
            .unwrap();
        // Force a distinct modification timestamp without sleeping long.
        let future_time = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let file = fs::File::create(&pem_path).unwrap();
        {
            let mut writer = std::io::BufWriter::new(&file);
            write!(writer, "{}\n{}", c2.pem(), k2.serialize_pem()).unwrap();
        }
        file.set_times(fs::FileTimes::new().set_modified(future_time))
            .unwrap();

        let (chain2, _) = ca
            .client_identity_for_host("custom.example.com", Some((&pem_path, None)))
            .unwrap()
            .unwrap();
        assert_ne!(chain1, chain2, "updated mtime must reload custom cert");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_data_dir_prefers_cli_override_then_env() {
        assert_eq!(
            resolve_custom_data_dir(Some(Path::new("/cli/data")), Some("/env/data")),
            Some(PathBuf::from("/cli/data"))
        );
        assert_eq!(
            resolve_custom_data_dir(None, Some("  /data/zniff  ")),
            Some(PathBuf::from("/data/zniff"))
        );
        assert_eq!(resolve_custom_data_dir(None, Some("   ")), None);
        assert_eq!(resolve_custom_data_dir(Some(Path::new("")), None), None);
    }
}
