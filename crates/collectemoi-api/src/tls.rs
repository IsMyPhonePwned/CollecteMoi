//! TLS certificate load / self-signed generation for the device API.

use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use time::{Duration, OffsetDateTime};

/// Resolve cert/key paths and ensure PEM files exist (generate if missing).
pub fn ensure_tls_files(
    data_dir: &Path,
    tls_cert: Option<&Path>,
    tls_key: Option<&Path>,
    public_url: &str,
) -> Result<(PathBuf, PathBuf)> {
    let tls_dir = data_dir.join("tls");
    let cert_path = tls_cert
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tls_dir.join("server.crt"));
    let key_path = tls_key
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tls_dir.join("server.key"));

    if cert_path.exists() && key_path.exists() {
        tracing::info!(
            cert = %cert_path.display(),
            key = %key_path.display(),
            "using existing TLS certificate"
        );
        return Ok((cert_path, key_path));
    }

    if tls_cert.is_some() || tls_key.is_some() {
        anyhow::bail!(
            "TLS cert/key not found: {} / {} (both must exist, or omit flags to auto-generate)",
            cert_path.display(),
            key_path.display()
        );
    }

    if let Some(parent) = cert_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let hosts = san_hosts(public_url);
    generate_self_signed(&cert_path, &key_path, &hosts)
        .with_context(|| format!("generate TLS cert at {}", cert_path.display()))?;
    tracing::info!(
        cert = %cert_path.display(),
        key = %key_path.display(),
        hosts = ?hosts,
        "generated self-signed TLS certificate (replace with --tls-cert / --tls-key for production)"
    );
    Ok((cert_path, key_path))
}

fn san_hosts(public_url: &str) -> Vec<String> {
    let mut hosts = vec!["localhost".into(), "127.0.0.1".into(), "::1".into()];
    if let Some(host) = host_from_url(public_url) {
        if !hosts.iter().any(|x| x == &host) {
            hosts.push(host);
        }
    }
    hosts
}

fn host_from_url(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.split("://").nth(1).unwrap_or(s);
    let hostport = rest.split('/').next()?.trim();
    if hostport.is_empty() {
        return None;
    }
    if hostport.starts_with('[') {
        let end = hostport.find(']')?;
        return Some(hostport[1..end].to_string());
    }
    Some(hostport.split(':').next()?.to_string())
}

fn generate_self_signed(cert_path: &Path, key_path: &Path, hosts: &[String]) -> Result<()> {
    let mut params = CertificateParams::default();
    params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
    params.is_ca = IsCa::NoCa;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "CollecteMoi Lab");
    dn.push(DnType::OrganizationName, "CollecteMoi");
    params.distinguished_name = dn;

    params.subject_alt_names.clear();
    for h in hosts {
        if let Ok(ip) = h.parse::<IpAddr>() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        } else if let Ok(name) = h.as_str().try_into() {
            params.subject_alt_names.push(SanType::DnsName(name));
        }
    }
    if params.subject_alt_names.is_empty() {
        params
            .subject_alt_names
            .push(SanType::DnsName("localhost".try_into().unwrap()));
    }

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    fs::write(cert_path, cert.pem())?;
    fs::write(key_path, key_pair.serialize_pem())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(key_path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
