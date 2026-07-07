use crate::core::paths;
use rcgen::{CertificateParams, IsCa, KeyPair};
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TlsMode {
    DomainAcme,
    IpAcme,
    SelfSigned,
    ReverseProxy,
}

pub enum TlsResult {
    Ready { cert_path: String, key_path: String },
    SkippedReverseProxy,
}

fn check_acme_sh() -> bool {
    std::process::Command::new("acme.sh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn setup_acme_domain(domain: &str) -> Result<TlsResult, String> {
    if !check_acme_sh() {
        return Err("acme.sh not installed, cannot issue domain certificate".to_string());
    }
    let cert_path = format!(
        "/root/.acme.sh/{}_ecc/fullchain.cer",
        domain.replace('*', "_")
    );
    let key_path = format!(
        "/root/.acme.sh/{}_ecc/{}.key",
        domain.replace('*', "_"),
        domain
    );
    if !std::path::Path::new(&cert_path).exists() {
        let output = std::process::Command::new("acme.sh")
            .args([
                "--issue",
                "-d",
                domain,
                "--standalone",
                "--keylength",
                "ec-256",
            ])
            .output()
            .map_err(|e| format!("acme.sh execution failed: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "acme.sh issue failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    let cert_dir = paths::sub_server::CERTS_DIR;
    fs::create_dir_all(cert_dir).map_err(|e| format!("create cert dir failed: {e}"))?;
    fs::copy(&cert_path, paths::sub_server::TLS_CERT)
        .map_err(|e| format!("copy cert failed: {e}"))?;
    fs::copy(&key_path, paths::sub_server::TLS_KEY).map_err(|e| format!("copy key failed: {e}"))?;
    Ok(TlsResult::Ready {
        cert_path: paths::sub_server::TLS_CERT.to_string(),
        key_path: paths::sub_server::TLS_KEY.to_string(),
    })
}

pub fn setup_acme_ip(ip: &str) -> Result<TlsResult, String> {
    if !check_acme_sh() {
        return Err("acme.sh not installed, cannot issue IP certificate".to_string());
    }
    let output = std::process::Command::new("acme.sh")
        .args([
            "--issue",
            "--standalone",
            "-d",
            ip,
            "--keylength",
            "ec-256",
            "--server",
            "letsencrypt",
        ])
        .output()
        .map_err(|e| format!("acme.sh execution failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "acme.sh IP issue failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let cert_dir = paths::sub_server::CERTS_DIR;
    fs::create_dir_all(cert_dir).map_err(|e| format!("create cert dir failed: {e}"))?;
    let cert_path = format!("/root/.acme.sh/{ip}_ecc/fullchain.cer");
    let key_path = format!("/root/.acme.sh/{ip}_ecc/{ip}.key");
    if std::path::Path::new(&cert_path).exists() {
        fs::copy(&cert_path, paths::sub_server::TLS_CERT)
            .map_err(|e| format!("copy cert failed: {e}"))?;
        fs::copy(&key_path, paths::sub_server::TLS_KEY)
            .map_err(|e| format!("copy key failed: {e}"))?;
    }
    Ok(TlsResult::Ready {
        cert_path: paths::sub_server::TLS_CERT.to_string(),
        key_path: paths::sub_server::TLS_KEY.to_string(),
    })
}

pub fn setup_self_signed() -> Result<TlsResult, String> {
    let cert_dir = paths::sub_server::CERTS_DIR;
    fs::create_dir_all(cert_dir).map_err(|e| format!("create cert dir failed: {e}"))?;

    let cert_path = std::path::Path::new(paths::sub_server::TLS_CERT);
    if cert_path.exists() {
        return Ok(TlsResult::Ready {
            cert_path: paths::sub_server::TLS_CERT.to_string(),
            key_path: paths::sub_server::TLS_KEY.to_string(),
        });
    }

    let mut params = CertificateParams::new(vec!["0.0.0.0".to_string()])
        .map_err(|e| format!("create cert params failed: {e}"))?;
    params.is_ca = IsCa::ExplicitNoCa;
    let key_pair = KeyPair::generate().map_err(|e| format!("generate key pair failed: {e}"))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("self-sign failed: {e}"))?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    fs::write(paths::sub_server::TLS_CERT, &cert_pem)
        .map_err(|e| format!("write cert failed: {e}"))?;
    fs::write(paths::sub_server::TLS_KEY, &key_pem)
        .map_err(|e| format!("write key failed: {e}"))?;
    Ok(TlsResult::Ready {
        cert_path: paths::sub_server::TLS_CERT.to_string(),
        key_path: paths::sub_server::TLS_KEY.to_string(),
    })
}
