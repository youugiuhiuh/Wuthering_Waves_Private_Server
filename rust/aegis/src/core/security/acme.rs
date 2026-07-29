use crate::core::{paths::acme, types::DnsProvider};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use x509_parser::{extensions::GeneralName, pem::Pem, prelude::FromDer};

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
        configured_provider_from(&config)
    }

    pub async fn ensure_installed() -> Result<PathBuf> {
        if Path::new(acme::BIN).is_file() {
            tighten_acme_permissions()?;
            return Ok(PathBuf::from(acme::BIN));
        }

        run_command("sh", &["-c", "curl -fsSL https://get.acme.sh | sh"], &[]).await?;
        if !Path::new(acme::BIN).is_file() {
            bail!("ACME installer completed without creating {}", acme::BIN);
        }
        run_command(acme::BIN, &["--install-cronjob"], &[]).await?;
        tighten_acme_permissions()?;
        Ok(PathBuf::from(acme::BIN))
    }

    pub fn cert_valid(domain: &str) -> Option<CertPaths> {
        let paths = Self::cert_paths(domain).ok()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs()
            .try_into()
            .ok()?;
        certificate_files_valid(domain, &paths, now).then_some(paths)
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
        tighten_cert_permissions(&paths)?;
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

        let command_result = run_command(acme::BIN, &args, &environment).await;
        tighten_acme_permissions()?;
        tighten_cert_permissions(&paths)?;
        command_result?;
        if !certificate_files_valid(
            &domain,
            &paths,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64,
        ) {
            bail!("ACME command produced invalid certificate material");
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

fn configured_provider_from(config: &str) -> Option<DnsProvider> {
    [
        DnsProvider::Cloudflare,
        DnsProvider::Aliyun,
        DnsProvider::Dnspod,
        DnsProvider::Route53,
    ]
    .into_iter()
    .find(|provider| {
        let (first, second) = provider.credential_names();
        has_non_empty_assignment(config, first) && has_non_empty_assignment(config, second)
    })
}

fn has_non_empty_assignment(config: &str, name: &str) -> bool {
    [format!("SAVED_{name}"), name.to_string()]
        .iter()
        .any(|candidate| assignment_value(config, candidate).is_some())
}

fn assignment_value<'a>(config: &'a str, name: &str) -> Option<&'a str> {
    config.lines().find_map(|line| {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let (key, value) = line.split_once('=')?;
        (key.trim() == name)
            .then(|| {
                let value = value.trim();
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
                    .or_else(|| {
                        value
                            .strip_prefix('"')
                            .and_then(|value| value.strip_suffix('"'))
                    })
                    .unwrap_or(value)
            })
            .filter(|value| !value.is_empty())
    })
}

fn certificate_files_valid(domain: &str, paths: &CertPaths, now: i64) -> bool {
    if !paths.fullchain.is_file() || !paths.privkey.is_file() {
        return false;
    }

    let Ok(pem_data) = fs::read(&paths.fullchain) else {
        return false;
    };
    let Ok((pem, _)) = Pem::read(std::io::Cursor::new(pem_data)) else {
        return false;
    };
    let Ok((_, certificate)) = x509_parser::certificate::X509Certificate::from_der(&pem.contents)
    else {
        return false;
    };
    let Some(minimum_expiry) = now.checked_add(MIN_VALIDITY as i64) else {
        return false;
    };
    if certificate.validity().not_before.timestamp() > now
        || certificate.validity().not_after.timestamp() <= minimum_expiry
        || !certificate_matches_domain(&certificate, domain)
    {
        return false;
    }

    public_key(&paths.fullchain, true)
        .zip(public_key(&paths.privkey, false))
        .is_some_and(|(certificate_key, private_key)| certificate_key == private_key)
}

fn certificate_matches_domain(
    certificate: &x509_parser::certificate::X509Certificate<'_>,
    domain: &str,
) -> bool {
    match certificate.subject_alternative_name() {
        Ok(Some(names)) => names.value.general_names.iter().any(|name| {
            matches!(name, GeneralName::DNSName(pattern) if dns_name_matches(pattern, domain))
        }),
        Ok(None) => certificate
            .subject()
            .iter_common_name()
            .filter_map(|name| name.as_str().ok())
            .any(|pattern| dns_name_matches(pattern, domain)),
        Err(_) => false,
    }
}

fn dns_name_matches(pattern: &str, domain: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();
    if pattern == domain {
        return true;
    }
    pattern
        .strip_prefix("*.")
        .and_then(|suffix| domain.strip_suffix(suffix))
        .is_some_and(|prefix| prefix.ends_with('.') && !prefix[..prefix.len() - 1].contains('.'))
}

fn public_key(path: &Path, certificate: bool) -> Option<Vec<u8>> {
    let mut command = std::process::Command::new("openssl");
    if certificate {
        command
            .args(["x509", "-in"])
            .arg(path)
            .args(["-pubkey", "-noout"]);
    } else {
        command.args(["pkey", "-in"]).arg(path).args(["-pubout"]);
    }
    let output = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

#[cfg(unix)]
fn set_mode_if_exists(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if path.exists() {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_mode_if_exists(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn tighten_acme_permissions() -> Result<()> {
    set_mode_if_exists(Path::new(acme::HOME), 0o700)?;
    set_mode_if_exists(Path::new(acme::ACCOUNT_CONF), 0o600)
}

fn tighten_cert_permissions(paths: &CertPaths) -> Result<()> {
    if let Some(directory) = paths.fullchain.parent() {
        set_mode_if_exists(directory, 0o700)?;
    }
    set_mode_if_exists(&paths.fullchain, 0o600)?;
    set_mode_if_exists(&paths.privkey, 0o600)
}

async fn run_command(program: &str, args: &[&str], environment: &[(&str, &str)]) -> Result<Output> {
    run_command_with_timeout(program, args, environment, COMMAND_TIMEOUT).await
}

async fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    environment: &[(&str, &str)],
    timeout: Duration,
) -> Result<Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    {
        // A subreaper can wait for descendants orphaned when the process group is killed.
        // SAFETY: prctl is called with the documented subreaper option and integer flag.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
            bail!("failed to configure subprocess isolation");
        }
        command.process_group(0);
    }
    for (name, value) in environment {
        command.env(name, value);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;
    let process_group = child.id().context("subprocess has no process ID")? as i32;
    let mut stdout = child
        .stdout
        .take()
        .context("subprocess stdout unavailable")?;
    let mut stderr = child
        .stderr
        .take()
        .context("subprocess stderr unavailable")?;
    let mut stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let mut stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let deadline = tokio::time::Instant::now() + timeout;
    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(status) => status.context("failed to wait for subprocess")?,
        Err(_) => {
            terminate_subprocess_tree(process_group, &mut child).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            bail!("command timed out after {} seconds", timeout.as_secs());
        }
    };
    let output = tokio::time::timeout_at(deadline, async {
        let stdout = (&mut stdout_task)
            .await
            .context("stdout reader task failed")??;
        let stderr = (&mut stderr_task)
            .await
            .context("stderr reader task failed")??;
        Ok::<_, anyhow::Error>((stdout, stderr))
    })
    .await;
    let (stdout, stderr) = match output {
        Ok(output) => output?,
        Err(_) => {
            terminate_subprocess_tree(process_group, &mut child).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            bail!("command timed out after {} seconds", timeout.as_secs());
        }
    };
    if !status.success() {
        bail!("command {program} failed with status {status}");
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

async fn terminate_subprocess_tree(process_group: i32, child: &mut tokio::process::Child) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = process_group;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    #[cfg(target_os = "linux")]
    terminate_and_reap_process_group(process_group, child).await;
}

#[cfg(target_os = "linux")]
async fn terminate_and_reap_process_group(process_group: i32, child: &mut tokio::process::Child) {
    // Negative PID targets every process in the isolated process group.
    // SAFETY: the negative PID is the child-owned process group created before spawn.
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.wait().await;
    let _ = tokio::task::spawn_blocking(move || {
        let mut status = 0;
        loop {
            // SAFETY: status is writable and waitpid is restricted to the killed process group.
            let result = unsafe { libc::waitpid(-process_group, &mut status, 0) };
            if result <= 0 {
                break;
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::DnsProvider;
    use std::{path::PathBuf, process::Stdio};

    fn generated_certificate(domain: &str, with_san: bool) -> (tempfile::TempDir, CertPaths) {
        let directory = tempfile::tempdir().unwrap();
        let fullchain = directory.path().join("fullchain.pem");
        let privkey = directory.path().join("privkey.pem");
        let mut command = std::process::Command::new("openssl");
        command.args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            privkey.to_str().unwrap(),
            "-out",
            fullchain.to_str().unwrap(),
            "-days",
            "90",
            "-subj",
            &format!("/CN={domain}"),
        ]);
        if with_san {
            command.args(["-addext", &format!("subjectAltName=DNS:{domain}")]);
        }
        assert!(
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        (directory, CertPaths { fullchain, privkey })
    }

    fn unix_now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

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

    #[test]
    fn detects_non_empty_saved_provider_credentials() {
        let config = "SAVED_CF_Token='token-value'\nSAVED_CF_Account_ID='account-value'\n";
        assert_eq!(
            configured_provider_from(config),
            Some(DnsProvider::Cloudflare)
        );
    }

    #[test]
    fn ignores_empty_saved_provider_credentials() {
        let config = "SAVED_CF_Token=''\nSAVED_CF_Account_ID='account-value'\n";
        assert_eq!(configured_provider_from(config), None);
    }

    #[test]
    fn detects_non_empty_legacy_provider_credentials() {
        let config = "Ali_Key=legacy-key\nAli_Secret=legacy-secret\n";
        assert_eq!(configured_provider_from(config), Some(DnsProvider::Aliyun));
    }

    #[test]
    fn accepts_valid_certificate_material() {
        let (_directory, paths) = generated_certificate("example.com", true);
        assert!(certificate_files_valid("example.com", &paths, unix_now()));
    }

    #[test]
    fn accepts_matching_common_name_when_san_is_absent() {
        let (_directory, paths) = generated_certificate("example.com", false);
        assert!(certificate_files_valid("example.com", &paths, unix_now()));
    }

    #[test]
    fn rejects_not_yet_valid_certificate() {
        let (_directory, paths) = generated_certificate("example.com", true);
        assert!(!certificate_files_valid("example.com", &paths, 0));
    }

    #[test]
    fn rejects_certificate_for_another_domain() {
        let (_directory, paths) = generated_certificate("other.example", true);
        assert!(!certificate_files_valid("example.com", &paths, unix_now()));
    }

    #[test]
    fn rejects_mismatched_private_key() {
        let (_first_directory, mut paths) = generated_certificate("example.com", true);
        let (_second_directory, second_paths) = generated_certificate("example.com", true);
        paths.privkey = second_paths.privkey;
        assert!(!certificate_files_valid("example.com", &paths, unix_now()));
    }

    #[cfg(unix)]
    #[test]
    fn tightens_certificate_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, paths) = generated_certificate("example.com", true);
        tighten_cert_permissions(&paths).unwrap();
        assert_eq!(
            fs::metadata(paths.fullchain).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(paths.privkey).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn command_failure_does_not_return_stderr() {
        let error = run_command_with_timeout(
            "sh",
            &["-c", "printf transformed-secret >&2; exit 7"],
            &[],
            Duration::from_secs(1),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("status"));
        assert!(!error.contains("transformed-secret"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn timeout_terminates_spawned_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("descendant-survived");
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            run_command_with_timeout(
                "sh",
                &[
                    "-c",
                    "(sleep 1; printf leaked > \"$1\") &",
                    "sh",
                    marker.to_str().unwrap(),
                ],
                &[],
                Duration::from_millis(100),
            ),
        )
        .await;
        let error = result
            .expect("runner exceeded its timeout")
            .expect_err("descendant unexpectedly survived command timeout");
        assert!(error.to_string().contains("timed out"));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!marker.exists());
    }
}
