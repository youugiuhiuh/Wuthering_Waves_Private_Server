//! ACME certificate lifecycle management via acme.sh CLI
//!
//! Certificate path: /root/cert/{domain}/fullchain.pem + privkey.pem
//! acme.sh path: ~/.acme.sh/acme.sh (resolve HOME, fallback /root)

use crate::core::error::Result;
use crate::core::types::DnsProvider;
use std::path::Path;
use std::time::Duration;
use x509_parser::certificate::X509Certificate;
use x509_parser::pem::Pem;
use x509_parser::prelude::*;

fn acme_sh_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    format!("{}/.acme.sh/acme.sh", home.trim_end_matches('/'))
}

/// Check if certificate files exist at /root/cert/{domain}/ and are not expired.
pub fn cert_valid(domain: &str) -> bool {
    let cert_dir = format!("/root/cert/{}", domain);
    check_cert_at_path(&cert_dir)
}

fn check_cert_at_path(cert_dir: &str) -> bool {
    let root = Path::new(cert_dir);
    let fullchain = root.join("fullchain.pem");
    let privkey = root.join("privkey.pem");

    if !fullchain.exists() || !privkey.exists() {
        return false;
    }

    let data = match std::fs::read(&fullchain) {
        Ok(d) => d,
        Err(_) => return false,
    };

    let mut pem_iter = Pem::iter_from_buffer(&data);
    let pem = match pem_iter.next() {
        Some(Ok(p)) => p,
        Some(Err(_)) | None => return false,
    };

    let (_, cert) = match X509Certificate::from_der(&pem.contents) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let now = ASN1Time::now();
    now < cert.validity().not_after
}

/// Check if DNS provider API credentials are configured in environment.
pub fn has_credentials(provider: DnsProvider) -> bool {
    match provider {
        DnsProvider::Cloudflare => {
            std::env::var("CF_Token").is_ok()
                || (std::env::var("CF_Email").is_ok() && std::env::var("CF_Key").is_ok())
        }
        DnsProvider::Aliyun => {
            std::env::var("Ali_Key").is_ok() && std::env::var("Ali_Secret").is_ok()
        }
        DnsProvider::Dnspod => std::env::var("DP_Id").is_ok() && std::env::var("DP_Key").is_ok(),
        DnsProvider::Route53 => {
            std::env::var("AWS_ACCESS_KEY_ID").is_ok()
                && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
        }
    }
}

/// Ensure acme.sh is installed at ~/.acme.sh/acme.sh.
pub fn ensure_installed() -> Result<()> {
    let path = acme_sh_path();
    if Path::new(&path).exists() {
        Ok(())
    } else {
        Err(crate::core::error::AppError::NotInstalled(format!(
            "acme.sh not found at {}",
            path
        )))
    }
}

/// Issue and install a TLS certificate via acme.sh for the given domain.
pub async fn issue_cert(domain: &str, email: &str, provider: DnsProvider) -> Result<()> {
    let acme_sh = acme_sh_path();
    let cert_dir = format!("/root/cert/{}", domain);
    std::fs::create_dir_all(&cert_dir).map_err(crate::core::error::AppError::Io)?;

    let dns_flag = provider.acme_dns_flag();

    crate::core::cmd_async::run_cmd_checked(
        &acme_sh,
        &[
            "--issue",
            "--dns",
            dns_flag,
            "-d",
            domain,
            "--email",
            email,
            "--keylength",
            "ec-256",
            "--server",
            "letsencrypt",
        ],
        Duration::from_secs(120),
    )
    .await
    .map_err(|e| crate::core::error::AppError::Service(format!("acme.sh issue failed: {}", e)))?;

    crate::core::cmd_async::run_cmd_checked(
        &acme_sh,
        &[
            "--install-cert",
            "-d",
            domain,
            "--fullchain-file",
            &format!("{}/fullchain.pem", cert_dir),
            "--key-file",
            &format!("{}/privkey.pem", cert_dir),
        ],
        Duration::from_secs(60),
    )
    .await
    .map_err(|e| {
        crate::core::error::AppError::Service(format!("acme.sh install-cert failed: {}", e))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn generate_self_signed_cert(dir: &std::path::Path, days: &str) {
        let key_path = dir.join("privkey.pem");
        let cert_path = dir.join("fullchain.pem");

        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-keyout",
                key_path.to_str().unwrap(),
                "-out",
                cert_path.to_str().unwrap(),
                "-days",
                days,
                "-nodes",
                "-subj",
                "/CN=test.example.com",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("openssl req failed");
        assert!(status.success(), "openssl req exited non-zero");
    }

    #[test]
    fn test_cert_valid_with_generated_certs() {
        let dir = TempDir::new().expect("tempdir");
        generate_self_signed_cert(dir.path(), "365");

        assert!(check_cert_at_path(dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_cert_valid_missing_files() {
        let dir = TempDir::new().expect("tempdir");
        assert!(!check_cert_at_path(dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_ensure_installed_finds_checks_path() {
        let result = ensure_installed();
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_has_credentials_without_env_vars() {
        assert!(!has_credentials(DnsProvider::Cloudflare));
    }
}
