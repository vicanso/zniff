//! Root CA stored in the user data directory, and per-host leaf certificates.
//!
//! The root is installed into the current user's trust store and kept when
//! the app exits. The private key stays in that directory.

use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const CA_NAME: &str = "zniff CA";
/// The previous product name. Still removed so an already-trusted root does not linger.
#[cfg(target_os = "macos")]
const LEGACY_CA_NAME: &str = "Snoop CA";

pub struct Ca {
    dir: PathBuf,
    /// Subject and key of the installed root. Leaf signing reads these.
    /// The PEM on disk is what gets installed.
    issuer: Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
    leaves: Mutex<Vec<(String, Leaf)>>,
}

struct Leaf {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
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
        let issuer_key = KeyPair::from_pem(&key_pem).context("读取根 CA 私钥")?;
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, issuer_key).context("读取根 CA 证书")?;
        let ca_der = ca_der_from_pem(&cert_pem)?;
        Ok(Self {
            dir,
            issuer,
            ca_der,
            leaves: Mutex::new(Vec::new()),
        })
    }

    pub fn cert_path(&self) -> PathBuf {
        self.dir.join("ca.crt")
    }

    #[cfg(test)]
    pub fn cert_der(&self) -> CertificateDer<'static> {
        self.ca_der.clone()
    }

    pub fn server_config(&self, host: &str, alpn: &[u8]) -> Result<ServerConfig> {
        let leaf = self.leaf(host)?;
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.cert.clone(), self.ca_der.clone()],
                leaf.key.clone_key(),
            )
            .context("组装叶子证书")?;
        config.alpn_protocols = vec![alpn.to_vec()];
        Ok(config)
    }

    fn leaf(&self, host: &str) -> Result<Leaf> {
        let mut guard = self.leaves.lock().unwrap_or_else(|err| err.into_inner());
        if let Some((_, leaf)) = guard.iter().find(|(name, _)| name == host) {
            return Ok(leaf.clone());
        }
        let leaf = self.sign_leaf(host)?;
        guard.push((host.to_string(), leaf.clone()));
        Ok(leaf)
    }

    fn sign_leaf(&self, host: &str) -> Result<Leaf> {
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
        Ok(Leaf {
            cert: CertificateDer::from(cert.der().to_vec()),
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        })
    }
}

impl Clone for Leaf {
    fn clone(&self) -> Self {
        Self {
            cert: self.cert.clone(),
            key: self.key.clone_key(),
        }
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
    let der = pem
        .lines()
        .filter(|line| !line.starts_with('-') && !line.trim().is_empty())
        .collect::<String>();
    use base64_decode::decode;
    let bytes = decode(&der).context("解析 CA PEM")?;
    Ok(CertificateDer::from(bytes))
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
        anyhow::bail!("Windows 上移除证书尚未接上，见 docs/known-issues.md")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        anyhow::bail!("Linux 上移除 NSS 证书尚未接上，见 docs/known-issues.md")
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
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn install_macos(ca: &Ca) -> Result<String> {
    let status = std::process::Command::new("security")
        .args([
            "add-trusted-cert",
            "-r",
            "trustRoot",
            "-p",
            "ssl",
            "-k",
            &login_keychain(),
            ca.cert_path().to_str().context("证书路径不是 UTF-8")?,
        ])
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
    let status = std::process::Command::new("security")
        .args(["delete-certificate", "-c", name, &login_keychain()])
        .status()
        .with_context(|| format!("运行 security delete-certificate {name}"))?;
    Ok(status.success())
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

#[cfg(all(unix, not(target_os = "macos")))]
fn install_linux(ca: &Ca) -> Result<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let db = format!("sql:{home}/.pki/nssdb");
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
        let _ = fs::remove_dir_all(&dir);
    }
}
