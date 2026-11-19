//! Detection, validation, and safe reuse of trusted local root CAs
//! created by compatible tools (Whistle, mitmproxy, mkcert, Proxyman).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rcgen::{Issuer, KeyPair, PublicKeyData};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::ca;

static PROBE_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalCaKind {
    Whistle,
    Mitmproxy,
    Mkcert,
    Proxyman,
}

impl ExternalCaKind {
    pub const ALL: [Self; 4] = [Self::Whistle, Self::Mitmproxy, Self::Mkcert, Self::Proxyman];

    pub fn id(self) -> &'static str {
        match self {
            Self::Whistle => "whistle",
            Self::Mitmproxy => "mitmproxy",
            Self::Mkcert => "mkcert",
            Self::Proxyman => "proxyman",
        }
    }

    pub fn from_id(raw: &str) -> Option<Self> {
        let lower = raw.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|kind| kind.id() == lower)
    }

    pub fn tool_name(self) -> &'static str {
        match self {
            Self::Whistle => "Whistle",
            Self::Mitmproxy => "mitmproxy",
            Self::Mkcert => "mkcert",
            Self::Proxyman => "Proxyman",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalCaSummary {
    pub id: String,
    pub tool_name: String,
    pub ca_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalCaCandidate {
    pub kind: ExternalCaKind,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

pub struct LoadedExternalCa {
    pub summary: ExternalCaSummary,
    pub cert_only_pem: String,
    pub ca_der: CertificateDer<'static>,
    pub issuer: Issuer<'static, KeyPair>,
}

#[derive(Clone, Debug, Default)]
pub struct ExternalCaSearchEnv {
    pub home_dir: Option<PathBuf>,
    pub whistle_path: Option<PathBuf>,
    pub caroot: Option<PathBuf>,
    #[cfg(all(unix, not(target_os = "macos")))]
    pub xdg_data_home: Option<PathBuf>,
    #[cfg(target_os = "windows")]
    pub local_app_data: Option<PathBuf>,
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    let val = std::env::var_os(key)?;
    if val.is_empty() || val.to_str().is_some_and(|s| s.trim().is_empty()) {
        return None;
    }
    Some(PathBuf::from(val))
}

impl ExternalCaSearchEnv {
    pub fn from_process_env() -> Self {
        Self {
            home_dir: non_empty_env_path("HOME").or_else(|| non_empty_env_path("USERPROFILE")),
            whistle_path: non_empty_env_path("WHISTLE_PATH"),
            caroot: non_empty_env_path("CAROOT"),
            #[cfg(all(unix, not(target_os = "macos")))]
            xdg_data_home: non_empty_env_path("XDG_DATA_HOME"),
            #[cfg(target_os = "windows")]
            local_app_data: non_empty_env_path("LOCALAPPDATA"),
        }
    }

    pub fn discover_candidates(&self) -> Vec<ExternalCaCandidate> {
        let mut out = Vec::new();
        for kind in ExternalCaKind::ALL {
            if let Some(found) = first_existing_pair(kind, &self.candidate_pairs(kind)) {
                out.push(found);
            }
        }
        out
    }

    fn candidate_pairs(&self, kind: ExternalCaKind) -> Vec<(PathBuf, PathBuf)> {
        match kind {
            ExternalCaKind::Whistle => self.whistle_pairs(),
            ExternalCaKind::Mitmproxy => self.mitmproxy_pairs(),
            ExternalCaKind::Mkcert => self.mkcert_pairs(),
            ExternalCaKind::Proxyman => self.proxyman_pairs(),
        }
    }

    fn whistle_pairs(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut roots = Vec::new();
        if let Some(custom) = &self.whistle_path {
            roots.push(custom.join("certs"));
        }
        if let Some(home) = &self.home_dir {
            roots.push(home.join(".WhistleAppData").join("certs"));
        }
        roots
            .into_iter()
            .map(|dir| (dir.join("root.crt"), dir.join("root.key")))
            .collect()
    }

    fn mitmproxy_pairs(&self) -> Vec<(PathBuf, PathBuf)> {
        let Some(home) = &self.home_dir else {
            return Vec::new();
        };
        let dir = home.join(".mitmproxy");
        let combined = dir.join("mitmproxy-ca.pem");
        let split_cert = dir.join("mitmproxy-ca-cert.pem");
        vec![(split_cert, combined.clone()), (combined.clone(), combined)]
    }

    fn mkcert_pairs(&self) -> Vec<(PathBuf, PathBuf)> {
        let mut dirs = Vec::new();
        if let Some(caroot) = &self.caroot {
            dirs.push(caroot.clone());
        }
        #[cfg(target_os = "macos")]
        if let Some(home) = &self.home_dir {
            dirs.push(home.join("Library/Application Support/mkcert"));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            if let Some(xdg) = &self.xdg_data_home {
                dirs.push(xdg.join("mkcert"));
            }
            if let Some(home) = &self.home_dir {
                dirs.push(home.join(".local/share/mkcert"));
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(local_app_data) = &self.local_app_data {
                dirs.push(local_app_data.join("mkcert"));
            }
            if let Some(home) = &self.home_dir {
                dirs.push(home.join("AppData/Local/mkcert"));
            }
        }
        dirs.into_iter()
            .map(|dir| (dir.join("rootCA.pem"), dir.join("rootCA-key.pem")))
            .collect()
    }

    fn proxyman_pairs(&self) -> Vec<(PathBuf, PathBuf)> {
        let Some(home) = &self.home_dir else {
            return Vec::new();
        };
        let dirs = [
            home.join("Library/Application Support/com.proxyman.NSProxy/certificates"),
            home.join(".proxyman/certificates"),
        ];
        let names = [
            ("proxyman-ca.pem", "proxyman-ca-key.pem"),
            ("ProxymanCA.pem", "ProxymanCA-key.pem"),
            ("root.pem", "root-key.pem"),
        ];
        let mut pairs = Vec::with_capacity(dirs.len() * names.len());
        for dir in dirs {
            for (cert, key) in names {
                pairs.push((dir.join(cert), dir.join(key)));
            }
        }
        pairs
    }
}

fn first_existing_pair(
    kind: ExternalCaKind,
    pairs: &[(PathBuf, PathBuf)],
) -> Option<ExternalCaCandidate> {
    pairs
        .iter()
        .find(|(cert, key)| cert.is_file() && key.is_file())
        .map(|(cert, key)| ExternalCaCandidate {
            kind,
            cert_path: cert.clone(),
            key_path: key.clone(),
        })
}

/// Extracts only the first `CERTIFICATE` PEM block so combined PEM files
/// can never leak private keys when written to `external-ca.crt`.
pub fn extract_first_cert_pem(pem: &str) -> Result<String> {
    let mut lines = Vec::new();
    let mut inside = false;
    for raw in pem.lines() {
        let trimmed = raw.trim();
        if !inside && trimmed == "-----BEGIN CERTIFICATE-----" {
            inside = true;
            lines.push(trimmed.to_string());
        } else if inside {
            lines.push(trimmed.to_string());
            if trimmed == "-----END CERTIFICATE-----" {
                let block = format!("{}\n", lines.join("\n"));
                let certs = ca::certs_from_pem(&block)?;
                if certs.is_empty() {
                    anyhow::bail!("CERTIFICATE PEM 块为空");
                }
                return Ok(block);
            }
        }
    }
    anyhow::bail!("PEM 中未找到完整的 CERTIFICATE 块")
}

fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 5 + content.len());
    out.push(tag);
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let start = bytes
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(bytes.len() - 1);
        let count = bytes.len() - start;
        out.push(0x80 | (count as u8));
        out.extend_from_slice(&bytes[start..]);
    }
    out.extend_from_slice(content);
    out
}

fn wrap_pkcs8_der(alg_oid_and_params: &[u8], inner_key_der: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0x02, 0x01, 0x00]);
    body.extend_from_slice(&der_tlv(0x30, alg_oid_and_params));
    body.extend_from_slice(&der_tlv(0x04, inner_key_der));
    der_tlv(0x30, &body)
}

fn pkcs1_rsa_to_pkcs8(pkcs1_der: &[u8]) -> Vec<u8> {
    // 1.2.840.113549.1.1.1 (rsaEncryption) + NULL
    const RSA_ALG: [u8; 13] = [
        0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
    ];
    wrap_pkcs8_der(&RSA_ALG, pkcs1_der)
}

fn sec1_ec_to_pkcs8(sec1_der: &[u8]) -> Result<Vec<u8>> {
    // 1.2.840.10045.2.1 (id-ecPublicKey)
    const EC_OID: [u8; 9] = [0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    // Default namedCurve OIDs for P-256 and P-384
    const P256_OID: [u8; 10] = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    const P384_OID: [u8; 7] = [0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22];

    let (seq_tag, mut rest, _) =
        ca::read_der_tlv(sec1_der).context("SEC1 EC 私钥不是 DER SEQUENCE")?;
    if seq_tag != 0x30 {
        anyhow::bail!("SEC1 EC 私钥格式错误");
    }
    let (_, _, after_ver) = ca::read_der_tlv(rest).context("SEC1 EC 缺少 version")?;
    let (priv_tag, priv_bytes, after_priv) =
        ca::read_der_tlv(after_ver).context("SEC1 EC 缺少 privateKey")?;
    rest = after_priv;

    let mut curve_oid: Option<&[u8]> = None;
    while let Some((tag, body, next)) = ca::read_der_tlv(rest) {
        rest = next;
        if tag == 0xa0
            && let Some((0x06, _, oid_after)) = ca::read_der_tlv(body)
        {
            curve_oid = Some(&body[..body.len() - oid_after.len()]);
            break;
        }
    }
    let curve = match curve_oid {
        Some(oid) => oid,
        None if priv_tag == 0x04 && priv_bytes.len() == 48 => &P384_OID,
        None => &P256_OID,
    };

    let mut alg = Vec::with_capacity(EC_OID.len() + curve.len());
    alg.extend_from_slice(&EC_OID);
    alg.extend_from_slice(curve);
    Ok(wrap_pkcs8_der(&alg, sec1_der))
}

fn key_pair_from_external_pem(pem: &str) -> Result<KeyPair> {
    let key_der = ca::key_from_pem(pem)?;
    let pkcs8_der = match key_der {
        PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der().to_vec(),
        PrivateKeyDer::Pkcs1(k) => pkcs1_rsa_to_pkcs8(k.secret_pkcs1_der()),
        PrivateKeyDer::Sec1(k) => sec1_ec_to_pkcs8(k.secret_sec1_der())?,
        _ => anyhow::bail!("不支持的私钥格式"),
    };
    KeyPair::try_from(pkcs8_der).context("解析外部 CA 私钥失败")
}

fn parse_two_digits(bytes: &[u8]) -> Option<i64> {
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(((bytes[0] - b'0') as i64) * 10 + ((bytes[1] - b'0') as i64))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_asn1_time(tag: u8, bytes: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let trimmed = text.strip_suffix('Z')?;
    let (year, rest) = match tag {
        0x17 if trimmed.len() == 12 => {
            let yy = parse_two_digits(&trimmed.as_bytes()[..2])?;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (year, &trimmed.as_bytes()[2..])
        }
        0x18 if trimmed.len() >= 14 => {
            let core = trimmed.split('.').next()?;
            if core.len() != 14 {
                return None;
            }
            let high = parse_two_digits(&core.as_bytes()[..2])?;
            let low = parse_two_digits(&core.as_bytes()[2..4])?;
            (high * 100 + low, &core.as_bytes()[4..])
        }
        _ => return None,
    };
    let month = parse_two_digits(&rest[0..2])?;
    let day = parse_two_digits(&rest[2..4])?;
    let hour = parse_two_digits(&rest[4..6])?;
    let min = parse_two_digits(&rest[6..8])?;
    let sec = parse_two_digits(&rest[8..10])?.min(59);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + min * 60 + sec)
}

pub fn parse_x509_ca_validity_and_is_ca(cert_der: &[u8]) -> Result<(i64, i64, bool)> {
    let (cert_tag, cert_body, _) =
        ca::read_der_tlv(cert_der).context("X.509 根证书不是 DER SEQUENCE")?;
    if cert_tag != 0x30 {
        anyhow::bail!("X.509 根证书顶层结构错误");
    }
    let (tbs_tag, mut tbs_rest, _) =
        ca::read_der_tlv(cert_body).context("X.509 证书缺少 TBSCertificate")?;
    if tbs_tag != 0x30 {
        anyhow::bail!("X.509 TBSCertificate 结构错误");
    }
    if let Some((0xa0, _, next)) = ca::read_der_tlv(tbs_rest) {
        tbs_rest = next;
    }
    for _ in 0..3 {
        let (_, _, next) = ca::read_der_tlv(tbs_rest).context("X.509 证书头字段残缺")?;
        tbs_rest = next;
    }
    let (val_tag, val_body, after_val) =
        ca::read_der_tlv(tbs_rest).context("X.509 证书缺少 Validity")?;
    if val_tag != 0x30 {
        anyhow::bail!("X.509 Validity 结构错误");
    }
    let (nb_tag, nb_bytes, after_nb) =
        ca::read_der_tlv(val_body).context("X.509 Validity 缺少 notBefore")?;
    let (na_tag, na_bytes, _) =
        ca::read_der_tlv(after_nb).context("X.509 Validity 缺少 notAfter")?;
    let not_before = parse_asn1_time(nb_tag, nb_bytes).context("解析 notBefore 时间失败")?;
    let not_after = parse_asn1_time(na_tag, na_bytes).context("解析 notAfter 时间失败")?;

    tbs_rest = after_val;
    for _ in 0..2 {
        let (_, _, next) = ca::read_der_tlv(tbs_rest).context("X.509 主体或公钥字段残缺")?;
        tbs_rest = next;
    }
    let is_ca = scan_extensions_is_ca(tbs_rest);
    Ok((not_before, not_after, is_ca))
}

fn scan_extensions_is_ca(mut tbs_tail: &[u8]) -> bool {
    // OID 2.5.29.19 (id-ce-basicConstraints)
    const BASIC_CONSTRAINTS_OID: [u8; 3] = [0x55, 0x1d, 0x13];

    while let Some((tag, body, next)) = ca::read_der_tlv(tbs_tail) {
        tbs_tail = next;
        if tag != 0xa3 {
            continue;
        }
        let Some((0x30, mut exts_rest, _)) = ca::read_der_tlv(body) else {
            continue;
        };
        while let Some((ext_tag, ext_body, next_ext)) = ca::read_der_tlv(exts_rest) {
            exts_rest = next_ext;
            if ext_tag != 0x30 {
                continue;
            }
            let Some((0x06, oid, after_oid)) = ca::read_der_tlv(ext_body) else {
                continue;
            };
            if oid != BASIC_CONSTRAINTS_OID {
                continue;
            }
            let Some((next_tag, next_body, after_mid)) = ca::read_der_tlv(after_oid) else {
                return false;
            };
            let octet_body = if next_tag == 0x04 {
                next_body
            } else if next_tag == 0x01
                && let Some((0x04, val, _)) = ca::read_der_tlv(after_mid)
            {
                val
            } else {
                return false;
            };
            let Some((0x30, bc_body, _)) = ca::read_der_tlv(octet_body) else {
                return false;
            };
            return matches!(
                ca::read_der_tlv(bc_body),
                Some((0x01, val, _)) if val.first().is_some_and(|&b| b != 0)
            );
        }
    }
    false
}

pub fn validate_external_ca_pem(
    kind: ExternalCaKind,
    cert_pem: &str,
    key_pem: &str,
    now_unix: i64,
) -> Result<LoadedExternalCa> {
    let cert_only_pem = extract_first_cert_pem(cert_pem)?;
    let mut certs = ca::certs_from_pem(&cert_only_pem)?;
    if certs.is_empty() {
        anyhow::bail!("外部 CA PEM 中未找到证书");
    }
    let ca_der = certs.swap_remove(0);

    let (not_before, not_after, is_ca) = parse_x509_ca_validity_and_is_ca(ca_der.as_ref())?;
    if !is_ca {
        anyhow::bail!("证书不是 CA 根证书 (BasicConstraints cA != true)");
    }
    if now_unix < not_before {
        anyhow::bail!("外部 CA 证书尚未到生效时间");
    }
    if now_unix > not_after {
        anyhow::bail!("外部 CA 证书已过期");
    }

    let issuer_key = key_pair_from_external_pem(key_pem)?;
    let cert_spki = ca::extract_spki_der(ca_der.as_ref()).context("无法读取证书公钥 (SPKI)")?;
    let key_spki = issuer_key.subject_public_key_info();
    if cert_spki != key_spki.as_slice() {
        anyhow::bail!("外部 CA 证书与私钥不匹配");
    }

    let ca_name =
        ca::extract_ca_common_name(&ca_der).unwrap_or_else(|| kind.tool_name().to_string());
    let issuer = Issuer::from_ca_cert_der(&ca_der, issuer_key).context("加载外部 CA 签发器失败")?;

    Ok(LoadedExternalCa {
        summary: ExternalCaSummary {
            id: kind.id().to_string(),
            tool_name: kind.tool_name().to_string(),
            ca_name,
        },
        cert_only_pem,
        ca_der,
        issuer,
    })
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn load_candidate(cand: &ExternalCaCandidate, now_unix: i64) -> Result<LoadedExternalCa> {
    let cert_pem = fs::read_to_string(&cand.cert_path)
        .with_context(|| format!("读取外部证书 {}", cand.cert_path.display()))?;
    let key_pem = if cand.key_path == cand.cert_path {
        cert_pem.clone()
    } else {
        fs::read_to_string(&cand.key_path)
            .with_context(|| format!("读取外部私钥 {}", cand.key_path.display()))?
    };
    validate_external_ca_pem(cand.kind, &cert_pem, &key_pem, now_unix)
}

struct TempCertGuard(PathBuf);

impl TempCertGuard {
    fn write_public_cert(cert_only_pem: &str) -> Result<Self> {
        let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "zniff-ext-ca-probe-{}-{seq}.crt",
            std::process::id()
        ));
        fs::write(&path, cert_only_pem.as_bytes())
            .with_context(|| format!("写入临时证书校验文件 {}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCertGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn check_loaded_trusted(loaded: &LoadedExternalCa) -> bool {
    let Ok(guard) = TempCertGuard::write_public_cert(&loaded.cert_only_pem) else {
        return false;
    };
    ca::is_cert_file_trusted(guard.path(), &loaded.summary.ca_name)
}

/// Scans well-known local proxy/dev tool CA directories and returns those whose
/// root certificate + private key are valid, unexpired CA pairs already trusted by this OS.
pub fn detect_trusted_external_cas() -> Vec<ExternalCaSummary> {
    let now = now_unix_secs();
    ExternalCaSearchEnv::from_process_env()
        .discover_candidates()
        .into_iter()
        .filter_map(|cand| {
            let loaded = load_candidate(&cand, now).ok()?;
            check_loaded_trusted(&loaded).then_some(loaded.summary)
        })
        .collect()
}

/// Loads, cryptographically validates, and verifies OS trust for an external tool CA by id.
pub fn load_trusted_candidate(id: &str) -> Result<LoadedExternalCa> {
    let kind = ExternalCaKind::from_id(id).with_context(|| format!("未知的外部证书来源: {id}"))?;
    let env = ExternalCaSearchEnv::from_process_env();
    let cand = first_existing_pair(kind, &env.candidate_pairs(kind))
        .with_context(|| format!("未在本机找到 {} 根证书或私钥文件", kind.tool_name()))?;
    let loaded = load_candidate(&cand, now_unix_secs())?;
    if !check_loaded_trusted(&loaded) {
        anyhow::bail!(
            "{} 根证书（{}）尚未在本机信任库中受信任",
            kind.tool_name(),
            loaded.summary.ca_name
        );
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa};

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(tag: &str) -> Self {
            let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "zniff-ext-ca-test-{}-{}-{seq}",
                tag,
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_test_ca_pem(cn: &str, is_ca: bool) -> (String, String, KeyPair) {
        let key = KeyPair::generate().expect("keygen");
        let mut params = CertificateParams::default();
        params.is_ca = if is_ca {
            IsCa::Ca(BasicConstraints::Unconstrained)
        } else {
            IsCa::NoCa
        };
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, cn);
        params.distinguished_name = name;
        let cert = params.self_signed(&key).expect("sign cert");
        (cert.pem(), key.serialize_pem(), key)
    }

    #[test]
    fn valid_ca_and_hot_switch_lifecycle() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, key_pem, _) = make_test_ca_pem("Whistle Test CA", true);
        let loaded = validate_external_ca_pem(
            ExternalCaKind::Whistle,
            &cert_pem,
            &key_pem,
            now_unix_secs(),
        )
        .expect("validate external CA");
        assert_eq!(
            loaded.summary,
            ExternalCaSummary {
                id: "whistle".into(),
                tool_name: "Whistle".into(),
                ca_name: "Whistle Test CA".into(),
            }
        );
        assert!(loaded.cert_only_pem.contains("BEGIN CERTIFICATE"));
        assert!(!loaded.cert_only_pem.contains("PRIVATE KEY"));

        let dir = TempDirGuard::new("switch");
        let ca = ca::Ca::open(dir.path()).expect("open builtin ca");
        let builtin_der = ca.cert_der();

        ca.switch_to_external(loaded).expect("switch to external");
        assert!(ca.is_external());
        assert_eq!(ca.ca_name(), "Whistle Test CA");
        assert_eq!(ca.cert_path(), dir.path().join("external-ca.crt"));

        let public_only = fs::read_to_string(ca.cert_path()).expect("read external-ca.crt");
        assert!(public_only.contains("BEGIN CERTIFICATE"));
        assert!(!public_only.contains("PRIVATE KEY"));

        assert!(ca.server_config("api.vicanso.dev", b"http/1.1").is_ok());

        let err = ca::remove(&ca).unwrap_err().to_string();
        assert!(err.contains("外部"), "{err}");

        ca.switch_to_builtin().expect("switch back to builtin");
        assert!(!ca.is_external());
        assert_eq!(ca.cert_der(), builtin_der);
        assert!(!dir.path().join("external-ca.crt").exists());
        assert!(ca.server_config("api.vicanso.dev", b"http/1.1").is_ok());
    }

    #[test]
    fn combined_mitmproxy_pem_both_orders_and_sec1_ec_key() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, key_pem, key) = make_test_ca_pem("mitmproxy Test CA", true);
        let now = now_unix_secs();

        let key_first = format!("{key_pem}\n{cert_pem}");
        let loaded_a =
            validate_external_ca_pem(ExternalCaKind::Mitmproxy, &key_first, &key_first, now)
                .expect("key-first combined pem");
        assert!(!loaded_a.cert_only_pem.contains("PRIVATE KEY"));
        assert_eq!(loaded_a.summary.ca_name, "mitmproxy Test CA");

        let cert_first = format!("{cert_pem}\n{key_pem}");
        let loaded_b =
            validate_external_ca_pem(ExternalCaKind::Mitmproxy, &cert_first, &cert_first, now)
                .expect("cert-first combined pem");
        assert!(!loaded_b.cert_only_pem.contains("PRIVATE KEY"));

        // Extract inner SEC1 ECPrivateKey bytes from PKCS#8 PrivateKeyInfo and re-wrap as SEC1 PEM.
        let pkcs8_der = key.serialize_der();
        let (_, seq_body, _) = ca::read_der_tlv(&pkcs8_der).expect("pkcs8 seq");
        let (_, _, after_ver) = ca::read_der_tlv(seq_body).expect("version");
        let (_, _, after_alg) = ca::read_der_tlv(after_ver).expect("alg");
        let (oct_tag, sec1_der, _) = ca::read_der_tlv(after_alg).expect("octet");
        assert_eq!(oct_tag, 0x04);
        let sec1_pem = format!(
            "-----BEGIN EC PRIVATE KEY-----\n{}\n-----END EC PRIVATE KEY-----\n",
            crate::session::base64_encode(sec1_der)
        );
        let loaded_sec1 =
            validate_external_ca_pem(ExternalCaKind::Mkcert, &cert_pem, &sec1_pem, now)
                .expect("sec1 ec key conversion");
        assert_eq!(loaded_sec1.ca_der, loaded_a.ca_der);
    }

    #[test]
    fn rejects_invalid_external_ca_pems() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let now = now_unix_secs();
        let (leaf_cert, leaf_key, _) = make_test_ca_pem("Leaf Only", false);
        assert!(
            validate_external_ca_pem(ExternalCaKind::Whistle, &leaf_cert, &leaf_key, now).is_err()
        );

        let (ca_cert, ca_key, _) = make_test_ca_pem("Valid CA", true);
        let (_, other_key, _) = make_test_ca_pem("Other CA", true);
        let valid = validate_external_ca_pem(ExternalCaKind::Whistle, &ca_cert, &ca_key, now)
            .expect("valid ca");
        let (not_before, not_after, is_ca) =
            parse_x509_ca_validity_and_is_ca(valid.ca_der.as_ref()).expect("parse validity");
        assert!(is_ca);

        // Mismatched key
        assert!(
            validate_external_ca_pem(ExternalCaKind::Whistle, &ca_cert, &other_key, now).is_err()
        );
        // Expired
        assert!(
            validate_external_ca_pem(ExternalCaKind::Whistle, &ca_cert, &ca_key, not_after + 1)
                .is_err()
        );
        // Not yet valid
        assert!(
            validate_external_ca_pem(ExternalCaKind::Whistle, &ca_cert, &ca_key, not_before - 1)
                .is_err()
        );
        // Encrypted key
        let enc =
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
        assert!(validate_external_ca_pem(ExternalCaKind::Whistle, &ca_cert, enc, now).is_err());
    }

    #[test]
    fn search_env_discovers_tools_in_priority_order_and_skips_incomplete() {
        let root = TempDirGuard::new("discover");
        let home = root.path().join("home");
        let whistle_custom = root.path().join("whistle-custom");
        let caroot = root.path().join("caroot");

        let whistle_certs = whistle_custom.join("certs");
        let mitm_dir = home.join(".mitmproxy");
        let proxyman_dir = home.join(".proxyman/certificates");
        for dir in [&whistle_certs, &mitm_dir, &caroot, &proxyman_dir] {
            fs::create_dir_all(dir).expect("mkdir");
        }
        fs::write(whistle_certs.join("root.crt"), "c").expect("write");
        fs::write(whistle_certs.join("root.key"), "k").expect("write");
        fs::write(mitm_dir.join("mitmproxy-ca.pem"), "ck").expect("write");
        fs::write(caroot.join("rootCA.pem"), "c").expect("write");
        fs::write(caroot.join("rootCA-key.pem"), "k").expect("write");
        // Write only cert without key for proxyman first name, complete pair for root.pem
        fs::write(proxyman_dir.join("proxyman-ca.pem"), "c").expect("write");
        fs::write(proxyman_dir.join("root.pem"), "c").expect("write");
        fs::write(proxyman_dir.join("root-key.pem"), "k").expect("write");

        let env = ExternalCaSearchEnv {
            home_dir: Some(home),
            whistle_path: Some(whistle_custom),
            caroot: Some(caroot),
            #[cfg(all(unix, not(target_os = "macos")))]
            xdg_data_home: None,
            #[cfg(target_os = "windows")]
            local_app_data: None,
        };
        let found = env.discover_candidates();
        let kinds: Vec<ExternalCaKind> = found.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ExternalCaKind::Whistle,
                ExternalCaKind::Mitmproxy,
                ExternalCaKind::Mkcert,
                ExternalCaKind::Proxyman,
            ]
        );
        assert_eq!(found[0].cert_path, whistle_certs.join("root.crt"));
        assert_eq!(found[3].cert_path, proxyman_dir.join("root.pem"));
    }
}
