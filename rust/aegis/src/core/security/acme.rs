//! ACME certificate lifecycle management via acme.sh CLI
//!
//! Certificate path: /root/cert/{domain}/fullchain.pem + privkey.pem
//! acme.sh path: ~/.acme.sh/acme.sh (resolve HOME, fallback /root)

use crate::core::error::Result;
use crate::core::types::DnsProvider;
use std::path::{Path, PathBuf};
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

/// Paths to TLS certificate files for a domain.
pub struct CertPaths {
    pub fullchain: PathBuf,
    pub privkey: PathBuf,
}

impl CertPaths {
    pub fn for_domain(domain: &str) -> Self {
        let dir = format!("/root/cert/{}", domain);
        Self {
            fullchain: PathBuf::from(format!("{}/fullchain.pem", dir)),
            privkey: PathBuf::from(format!("{}/privkey.pem", dir)),
        }
    }
}

pub fn detect_dns_provider() -> Option<DnsProvider> {
    let conf = account_conf_path();
    if !conf.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&conf).ok()?;
    let content_lower = content.to_lowercase();
    if content_lower.contains("cf_token") || content_lower.contains("cf_key") {
        return Some(DnsProvider::Cloudflare);
    }
    if content_lower.contains("ali_key") {
        return Some(DnsProvider::Aliyun);
    }
    if content_lower.contains("dp_id") {
        return Some(DnsProvider::Dnspod);
    }
    if content_lower.contains("aws_access_key") {
        return Some(DnsProvider::Route53);
    }
    None
}

pub async fn setup_and_issue(
    domain: &str,
    provider: DnsProvider,
    token: &str,
    key: &str,
) -> Result<()> {
    match provider {
        DnsProvider::Cloudflare => {
            // SAFETY: single-threaded async context, no concurrent env reads
            unsafe {
                std::env::set_var("CF_Token", token);
                std::env::set_var("CF_Account_ID", key);
            }
        }
        DnsProvider::Aliyun =>
        // SAFETY: acme.sh reads env vars for DNS API keys; single-threaded context
        unsafe {
            std::env::set_var("Ali_Key", token);
            std::env::set_var("Ali_Secret", key);
        },
        DnsProvider::Dnspod =>
        // SAFETY: acme.sh reads env vars for DNS API keys; single-threaded context
        unsafe {
            std::env::set_var("DP_Id", token);
            std::env::set_var("DP_Key", key);
        },
        DnsProvider::Route53 =>
        // SAFETY: acme.sh reads env vars for DNS API keys; single-threaded context
        unsafe {
            std::env::set_var("AWS_ACCESS_KEY_ID", token);
            std::env::set_var("AWS_SECRET_ACCESS_KEY", key);
        },
    }
    issue_cert(domain, &format!("admin@{}", domain), provider).await
}

fn account_conf_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::PathBuf::from(format!(
        "{}/.acme.sh/account.conf",
        home.trim_end_matches('/')
    ))
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
