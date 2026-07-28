use crate::core::{paths::acme, types::DnsProvider};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;
use x509_parser::{pem::Pem, prelude::FromDer};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MIN_VALIDITY: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPaths {
    pub fullchain: PathBuf,
    pub privkey: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XhttpDeployMode {
    Reality,
    Tls {
        domain: String,
        cert_paths: CertPaths,
    },
}

pub struct AcmeManager;

impl AcmeManager {
    pub fn validate_domain(input: &str) -> Result<String> {
        if !input.is_ascii() {
            bail!("domain must contain ASCII characters only");
        }

        let domain = input
            .strip_suffix('.')
            .unwrap_or(input)
            .to_ascii_lowercase();
        if domain.len() > 253 || !domain.contains('.') {
            bail!("domain must be a dotted DNS name of at most 253 characters");
        }

        for label in domain.split('.') {
            if label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                || !label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                || !label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
            {
                bail!("domain contains an invalid DNS label");
            }
        }

        Ok(domain)
    }

    pub fn cert_paths(domain: &str) -> Result<CertPaths> {
        let domain = Self::validate_domain(domain)?;
        let directory = Path::new(acme::CERT_ROOT).join(domain);
        Ok(CertPaths {
            fullchain: directory.join("fullchain.pem"),
            privkey: directory.join("privkey.pem"),
        })
    }

    pub fn configured_provider() -> Option<DnsProvider> {
        let config = fs::read_to_string(acme::ACCOUNT_CONF).ok()?;
        [
            DnsProvider::Cloudflare,
            DnsProvider::Aliyun,
            DnsProvider::Dnspod,
            DnsProvider::Route53,
        ]
        .into_iter()
        .find(|provider| {
            let (first, second) = provider.credential_names();
            has_assignment(&config, first) && has_assignment(&config, second)
        })
    }

    pub async fn ensure_installed() -> Result<PathBuf> {
        if Path::new(acme::BIN).is_file() {
            return Ok(PathBuf::from(acme::BIN));
        }

        run_command("sh", &["-c", "curl -fsSL https://get.acme.sh | sh"], &[]).await?;
        if !Path::new(acme::BIN).is_file() {
            bail!("ACME installer completed without creating {}", acme::BIN);
        }
        run_command(acme::BIN, &["--install-cronjob"], &[]).await?;
        Ok(PathBuf::from(acme::BIN))
    }

    pub fn cert_valid(domain: &str) -> Option<CertPaths> {
        let paths = Self::cert_paths(domain).ok()?;
        if !paths.fullchain.is_file() || !paths.privkey.is_file() {
            return None;
        }

        let pem_data = fs::read(&paths.fullchain).ok()?;
        let (pem, _) = Pem::read(std::io::Cursor::new(pem_data)).ok()?;
        let (_, certificate) =
            x509_parser::certificate::X509Certificate::from_der(&pem.contents).ok()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let minimum = now.checked_add(MIN_VALIDITY)?;
        (certificate.validity().not_after.timestamp() > i64::try_from(minimum).ok()?)
            .then_some(paths)
    }

    pub async fn issue_cert(
        domain: &str,
        provider: DnsProvider,
        credentials: Option<(&str, &str)>,
    ) -> Result<CertPaths> {
        let domain = Self::validate_domain(domain)?;
        let paths = Self::cert_paths(&domain)?;
        let directory = paths
            .fullchain
            .parent()
            .context("certificate path has no parent directory")?;
        tokio::fs::create_dir_all(directory).await?;
        let args = if paths.fullchain.exists() || paths.privkey.exists() {
            Self::renew_args(&domain)?
        } else {
            Self::issue_args(&domain, provider)?
        };
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        let names = provider.credential_names();
        let environment = credentials
            .map(|(first, second)| [(names.0, first), (names.1, second)])
            .unwrap_or_default();

        run_command(acme::BIN, &args, &environment).await?;
        if !paths.fullchain.is_file() || !paths.privkey.is_file() {
            bail!("ACME command completed without writing both certificate files");
        }
        Ok(paths)
    }

    fn issue_args(domain: &str, provider: DnsProvider) -> Result<Vec<String>> {
        Self::command_args("--issue", domain, Some(provider))
    }

    fn renew_args(domain: &str) -> Result<Vec<String>> {
        Self::command_args("--renew", domain, None)
    }

    fn command_args(
        action: &str,
        domain: &str,
        provider: Option<DnsProvider>,
    ) -> Result<Vec<String>> {
        let domain = Self::validate_domain(domain)?;
        let paths = Self::cert_paths(&domain)?;
        let mut args = vec![action.to_string()];
        if let Some(provider) = provider {
            args.extend(["--dns".to_string(), provider.acme_flag().to_string()]);
        }
        args.extend(["-d".to_string(), domain]);
        if provider.is_none() {
            args.push("--force".to_string());
        }
        args.extend([
            "--fullchain-file".to_string(),
            paths.fullchain.to_string_lossy().into_owned(),
            "--key-file".to_string(),
            paths.privkey.to_string_lossy().into_owned(),
        ]);
        Ok(args)
    }
}

fn has_assignment(config: &str, name: &str) -> bool {
    config.lines().any(|line| {
        line.trim()
            .strip_prefix("export ")
            .unwrap_or(line.trim())
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })
}

async fn run_command(program: &str, args: &[&str], environment: &[(&str, &str)]) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    for (name, value) in environment {
        command.env(name, value);
    }

    let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow!("command timed out after 120 seconds"))??;
    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        for (_, value) in environment {
            if !value.is_empty() {
                stderr = stderr.replace(value, "[REDACTED]");
            }
        }
        bail!("command failed: {}", stderr.trim());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::DnsProvider;
    use std::path::PathBuf;

    #[test]
    fn validates_plain_dns_names_only() {
        assert_eq!(
            AcmeManager::validate_domain("Example.COM.").unwrap(),
            "example.com"
        );
        for value in [
            "",
            "localhost",
            "-bad.example",
            "bad_.example",
            "a..example.com",
            "example.com;id",
        ] {
            assert!(
                AcmeManager::validate_domain(value).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn cert_paths_cannot_escape_root() {
        let paths = AcmeManager::cert_paths("example.com").unwrap();
        assert_eq!(
            paths.fullchain,
            PathBuf::from("/root/cert/example.com/fullchain.pem")
        );
        assert!(AcmeManager::cert_paths("../../etc").is_err());
    }

    #[test]
    fn issue_arguments_install_to_expected_paths() {
        let args = AcmeManager::issue_args("example.com", DnsProvider::Cloudflare).unwrap();
        assert_eq!(
            args,
            vec![
                "--issue",
                "--dns",
                "dns_cf",
                "-d",
                "example.com",
                "--fullchain-file",
                "/root/cert/example.com/fullchain.pem",
                "--key-file",
                "/root/cert/example.com/privkey.pem"
            ]
        );
    }

    #[test]
    fn renewal_arguments_force_refresh_existing_domain() {
        let args = AcmeManager::renew_args("example.com").unwrap();
        assert_eq!(
            args,
            vec![
                "--renew",
                "-d",
                "example.com",
                "--force",
                "--fullchain-file",
                "/root/cert/example.com/fullchain.pem",
                "--key-file",
                "/root/cert/example.com/privkey.pem"
            ]
        );
    }
}
