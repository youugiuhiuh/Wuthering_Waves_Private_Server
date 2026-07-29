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
#[cfg(target_os = "linux")]
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const PROCESS_TOKEN_ENV: &str = "AEGIS_ACME_PROCESS_TOKEN";

const DNS_CREDENTIAL_VARS: &[&str] = &[
    "Ali_Key",
    "Ali_Secret",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "CF_Token",
    "CF_Account_ID",
    "DP_Id",
    "DP_Key",
    "SAVED_Ali_Key",
    "SAVED_Ali_Secret",
    "SAVED_AWS_ACCESS_KEY_ID",
    "SAVED_AWS_SECRET_ACCESS_KEY",
    "SAVED_CF_Token",
    "SAVED_CF_Account_ID",
    "SAVED_DP_Id",
    "SAVED_DP_Key",
];

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

    pub async fn cert_valid(domain: &str) -> Option<CertPaths> {
        let domain = Self::validate_domain(domain).ok()?;
        let paths = Self::cert_paths(&domain).ok()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs()
            .try_into()
            .ok()?;
        certificate_files_valid(&domain, &paths, now)
            .await
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
        )
        .await
        {
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
            .then(|| parse_assignment_value(value))
            .filter(|value| !value.is_empty())
    })
}

fn parse_assignment_value(value: &str) -> &str {
    let value = value.trim();
    if let Some(quote) = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))
    {
        let quoted = &value[quote.len_utf8()..];
        if let Some(end) = quoted.find(quote) {
            return &quoted[..end];
        }
    }
    let comment = value
        .char_indices()
        .find(|(index, character)| {
            *character == '#'
                && (*index == 0
                    || value[..*index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace))
        })
        .map_or(value, |(index, _)| &value[..index]);
    comment.trim_end()
}

async fn certificate_files_valid(domain: &str, paths: &CertPaths, now: i64) -> bool {
    let Ok(domain) = AcmeManager::validate_domain(domain) else {
        return false;
    };
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
        || !certificate_matches_domain(&certificate, &domain)
    {
        return false;
    }

    let (certificate_key, private_key) = tokio::join!(
        public_key(&paths.fullchain, true),
        public_key(&paths.privkey, false)
    );
    certificate_key
        .zip(private_key)
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

async fn public_key(path: &Path, certificate: bool) -> Option<Vec<u8>> {
    public_key_with_program("openssl", path, certificate, COMMAND_TIMEOUT).await
}

async fn public_key_with_program(
    program: &str,
    path: &Path,
    certificate: bool,
    timeout: Duration,
) -> Option<Vec<u8>> {
    let path = path.to_str()?;
    let args = if certificate {
        vec!["x509", "-in", path, "-pubkey", "-noout"]
    } else {
        vec!["pkey", "-in", path, "-pubout"]
    };
    run_command_with_timeout(program, &args, &[], timeout)
        .await
        .ok()
        .map(|output| output.stdout)
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
    let owned_env: Vec<(String, String)> = environment
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let stripped_env: Vec<String> = DNS_CREDENTIAL_VARS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    run_command_inner(program, args, owned_env, &stripped_env, COMMAND_TIMEOUT).await
}

async fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    environment: &[(&str, &str)],
    timeout: Duration,
) -> Result<Output> {
    let owned_env: Vec<(String, String)> = environment
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let stripped_env: Vec<String> = DNS_CREDENTIAL_VARS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    run_command_inner(program, args, owned_env, &stripped_env, timeout).await
}

async fn run_command_inner(
    program: &str,
    args: &[&str],
    environment: Vec<(String, String)>,
    strip_vars: &[String],
    timeout: Duration,
) -> Result<Output> {
    #[cfg(target_os = "linux")]
    let process_token = process_token();
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "kill -STOP $$; exec \"$@\"",
            "aegis-acme-command",
            program,
        ]);
        command.args(args).env(PROCESS_TOKEN_ENV, &process_token);
        command
    };
    #[cfg(not(target_os = "linux"))]
    let mut command = {
        let mut command = Command::new(program);
        command.args(args);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl is called with the documented subreaper option and integer flag.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
            bail!("failed to configure subprocess isolation");
        }
        command.process_group(0);
    }
    for name in strip_vars {
        command.env_remove(name.as_str());
    }
    for (name, value) in &environment {
        command.env(name.as_str(), value.as_str());
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;
    let process_id = child.id().context("subprocess has no process ID")? as i32;
    #[cfg(target_os = "linux")]
    let process_scope = initialize_process_scope(process_id, process_token, &mut child).await?;
    #[cfg(not(target_os = "linux"))]
    let process_scope = ProcessScope { pid: process_id };
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
            stdout_task.abort();
            stderr_task.abort();
            terminate_subprocess_tree(&process_scope, &mut child).await;
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
            stdout_task.abort();
            stderr_task.abort();
            terminate_subprocess_tree(&process_scope, &mut child).await;
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

async fn terminate_subprocess_tree(scope: &ProcessScope, child: &mut tokio::process::Child) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = scope.pid;
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    #[cfg(target_os = "linux")]
    terminate_and_reap_process_scope(scope, child).await;
}

#[cfg(target_os = "linux")]
async fn terminate_and_reap_process_scope(scope: &ProcessScope, child: &mut tokio::process::Child) {
    let scope = scope.clone();
    let descendants = tokio::task::spawn_blocking(move || stop_and_kill_subtree(&scope))
        .await
        .unwrap_or_default();
    let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
    let _ = tokio::time::timeout(
        CLEANUP_TIMEOUT,
        tokio::task::spawn_blocking(move || reap_descendants(descendants)),
    )
    .await;
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProcessIdentity {
    pid: i32,
    start_time: u64,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct ProcessScope {
    root: ProcessIdentity,
    token: String,
}

#[cfg(not(target_os = "linux"))]
struct ProcessScope {
    pid: i32,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct ProcessEntry {
    identity: ProcessIdentity,
    state: char,
}

#[cfg(target_os = "linux")]
async fn initialize_process_scope(
    pid: i32,
    token: String,
    child: &mut tokio::process::Child,
) -> Result<ProcessScope> {
    let root = process_entry(pid).context("failed to establish subprocess identity")?;
    let deadline = tokio::time::Instant::now() + CLEANUP_TIMEOUT;
    loop {
        let current = process_entry(pid);
        if !identity_matches(root.identity, current) {
            bail!("subprocess identity changed during startup");
        }
        if current.is_some_and(|entry| entry.state == 'T') {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            signal_if_same(root.identity, libc::SIGKILL);
            let _ = child.wait().await;
            bail!("subprocess startup timed out");
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    if !signal_if_same(root.identity, libc::SIGCONT) {
        bail!("subprocess identity changed before startup");
    }
    Ok(ProcessScope {
        root: root.identity,
        token,
    })
}

#[cfg(target_os = "linux")]
fn process_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(target_os = "linux")]
fn stop_and_kill_subtree(scope: &ProcessScope) -> Vec<ProcessIdentity> {
    let mut tracked = std::collections::HashMap::from([(scope.root.pid, scope.root)]);
    signal_if_same(scope.root, libc::SIGSTOP);
    let deadline = std::time::Instant::now() + CLEANUP_TIMEOUT;

    loop {
        let added = process_snapshot()
            .into_iter()
            .filter(|entry| process_has_token(entry.identity.pid, &scope.token))
            .filter(|entry| !tracked.contains_key(&entry.identity.pid))
            .map(|entry| entry.identity)
            .collect::<Vec<_>>();
        for identity in &added {
            tracked.insert(identity.pid, *identity);
            signal_if_same(*identity, libc::SIGSTOP);
        }
        if added.is_empty() || std::time::Instant::now() >= deadline {
            break;
        }
    }

    let identities = tracked.into_values().collect::<Vec<_>>();
    for identity in &identities {
        signal_if_same(*identity, libc::SIGKILL);
    }
    identities
}

#[cfg(target_os = "linux")]
fn process_has_token(pid: i32, token: &str) -> bool {
    let expected = format!("{PROCESS_TOKEN_ENV}={token}");
    fs::read(format!("/proc/{pid}/environ"))
        .ok()
        .is_some_and(|environment| {
            environment
                .split(|byte| *byte == 0)
                .any(|value| value == expected.as_bytes())
        })
}

#[cfg(target_os = "linux")]
fn process_snapshot() -> Vec<ProcessEntry> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
        .filter_map(process_entry)
        .collect()
}

#[cfg(target_os = "linux")]
fn process_entry(pid: i32) -> Option<ProcessEntry> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .get(stat.rfind(')')? + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    Some(ProcessEntry {
        identity: ProcessIdentity {
            pid,
            start_time: fields.get(19)?.parse().ok()?,
        },
        state: fields.first()?.chars().next()?,
    })
}

#[cfg(target_os = "linux")]
fn identity_matches(expected: ProcessIdentity, current: Option<ProcessEntry>) -> bool {
    current.is_some_and(|entry| entry.identity == expected)
}

#[cfg(target_os = "linux")]
fn signal_if_same(identity: ProcessIdentity, signal: i32) -> bool {
    if identity_matches(identity, process_entry(identity.pid)) {
        // SAFETY: identity was read from /proc and revalidated immediately before signaling.
        return unsafe { libc::kill(identity.pid, signal) } == 0;
    }
    false
}

#[cfg(target_os = "linux")]
fn reap_descendants(identities: Vec<ProcessIdentity>) {
    let mut pending = identities
        .into_iter()
        .map(|identity| identity.pid)
        .collect::<std::collections::HashSet<_>>();
    let deadline = std::time::Instant::now() + CLEANUP_TIMEOUT;
    while !pending.is_empty() && std::time::Instant::now() < deadline {
        pending.retain(|pid| {
            let mut status = 0;
            // SAFETY: status is writable and waitpid is scoped to a known descendant PID.
            unsafe { libc::waitpid(*pid, &mut status, libc::WNOHANG) == 0 }
        });
        if !pending.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::DnsProvider;
    use std::{path::PathBuf, process::Stdio, sync::Arc};

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

    struct EnvRestore {
        name: &'static str,
        value: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(name);
            // SAFETY: credential environment mutation is serialized by the test attribute.
            unsafe {
                std::env::set_var(name, value);
            }
            Self {
                name,
                value: previous,
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            // SAFETY: credential environment mutation is serialized by the test attribute.
            unsafe {
                if let Some(value) = &self.value {
                    std::env::set_var(self.name, value);
                } else {
                    std::env::remove_var(self.name);
                }
            }
        }
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
    fn ignores_quoted_empty_credentials_followed_by_comments() {
        let config = "SAVED_CF_Token='' # unset\nSAVED_CF_Account_ID='account-value'\n";
        assert_eq!(configured_provider_from(config), None);
    }

    #[test]
    fn detects_non_empty_legacy_provider_credentials() {
        let config = "Ali_Key=legacy-key\nAli_Secret=legacy-secret\n";
        assert_eq!(configured_provider_from(config), Some(DnsProvider::Aliyun));
    }

    #[tokio::test]
    async fn accepts_valid_certificate_material() {
        let (_directory, paths) = generated_certificate("example.com", true);
        assert!(certificate_files_valid("example.com", &paths, unix_now()).await);
    }

    #[tokio::test]
    async fn accepts_matching_common_name_when_san_is_absent() {
        let (_directory, paths) = generated_certificate("example.com", false);
        assert!(certificate_files_valid("example.com", &paths, unix_now()).await);
    }

    #[tokio::test]
    async fn accepts_normalized_domain_for_certificate_matching() {
        let (_directory, paths) = generated_certificate("example.com", true);
        assert!(certificate_files_valid("Example.COM.", &paths, unix_now()).await);
    }

    #[tokio::test]
    async fn rejects_not_yet_valid_certificate() {
        let (_directory, paths) = generated_certificate("example.com", true);
        assert!(!certificate_files_valid("example.com", &paths, 0).await);
    }

    #[tokio::test]
    async fn rejects_certificate_for_another_domain() {
        let (_directory, paths) = generated_certificate("other.example", true);
        assert!(!certificate_files_valid("example.com", &paths, unix_now()).await);
    }

    #[tokio::test]
    async fn rejects_mismatched_private_key() {
        let (_first_directory, mut paths) = generated_certificate("example.com", true);
        let (_second_directory, second_paths) = generated_certificate("example.com", true);
        paths.privkey = second_paths.privkey;
        assert!(!certificate_files_valid("example.com", &paths, unix_now()).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn public_key_command_uses_bounded_runner() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("slow-openssl");
        let key = directory.path().join("key.pem");
        fs::write(&program, "#!/bin/sh\nsleep 2\n").unwrap();
        fs::write(&key, "unused").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let started = tokio::time::Instant::now();
        let result = public_key_with_program(
            program.to_str().unwrap(),
            &key,
            false,
            Duration::from_millis(100),
        )
        .await;
        assert!(result.is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
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

    #[serial_test::serial]
    #[tokio::test]
    async fn child_receives_only_selected_provider_credentials() {
        let _environment = [
            EnvRestore::set("Ali_Key", "parent-ali-key"),
            EnvRestore::set("SAVED_AWS_SECRET_ACCESS_KEY", "parent-aws-secret"),
            EnvRestore::set("SAVED_CF_Token", "parent-saved-token"),
            EnvRestore::set("ACME_OPERATIONAL_TEST", "retained"),
        ];
        run_command_with_timeout(
            "sh",
            &[
                "-c",
                "test -z \"${Ali_Key+x}\" && test -z \"${SAVED_AWS_SECRET_ACCESS_KEY+x}\" && test -z \"${SAVED_CF_Token+x}\" && test \"$ACME_OPERATIONAL_TEST\" = retained && test -n \"$CF_Token\" && test -n \"$CF_Account_ID\"",
            ],
            &[("CF_Token", "selected-token"), ("CF_Account_ID", "selected-account")],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn aliyun_credentials_isolated_from_other_providers() {
        let _environment = [
            EnvRestore::set("CF_Token", "parent-cf-token"),
            EnvRestore::set("SAVED_AWS_SECRET_ACCESS_KEY", "parent-aws-secret"),
            EnvRestore::set("SAVED_CF_Token", "parent-saved-cf"),
            EnvRestore::set("DP_Id", "parent-dp-id"),
            EnvRestore::set("ACME_OPERATIONAL_TEST", "retained"),
        ];
        run_command_with_timeout(
            "sh",
            &[
                "-c",
                "test -z \"${CF_Token+x}\" && test -z \"${SAVED_AWS_SECRET_ACCESS_KEY+x}\" && test -z \"${SAVED_CF_Token+x}\" && test -z \"${DP_Id+x}\" && test \"$ACME_OPERATIONAL_TEST\" = retained && test -n \"$Ali_Key\" && test -n \"$Ali_Secret\"",
            ],
            &[("Ali_Key", "selected-ali-key"), ("Ali_Secret", "selected-ali-secret")],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn dnspod_credentials_isolated_from_other_providers() {
        let _environment = [
            EnvRestore::set("CF_Token", "parent-cf-token"),
            EnvRestore::set("SAVED_AWS_SECRET_ACCESS_KEY", "parent-aws-secret"),
            EnvRestore::set("Ali_Key", "parent-ali-key"),
            EnvRestore::set("ACME_OPERATIONAL_TEST", "retained"),
        ];
        run_command_with_timeout(
            "sh",
            &[
                "-c",
                "test -z \"${CF_Token+x}\" && test -z \"${SAVED_AWS_SECRET_ACCESS_KEY+x}\" && test -z \"${Ali_Key+x}\" && test \"$ACME_OPERATIONAL_TEST\" = retained && test -n \"$DP_Id\" && test -n \"$DP_Key\"",
            ],
            &[("DP_Id", "selected-dp-id"), ("DP_Key", "selected-dp-key")],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn route53_credentials_isolated_from_other_providers() {
        let _environment = [
            EnvRestore::set("CF_Token", "parent-cf-token"),
            EnvRestore::set("Ali_Key", "parent-ali-key"),
            EnvRestore::set("SAVED_CF_Token", "parent-saved-cf"),
            EnvRestore::set("DP_Id", "parent-dp-id"),
            EnvRestore::set("ACME_OPERATIONAL_TEST", "retained"),
        ];
        run_command_with_timeout(
            "sh",
            &[
                "-c",
                "test -z \"${CF_Token+x}\" && test -z \"${Ali_Key+x}\" && test -z \"${SAVED_CF_Token+x}\" && test -z \"${DP_Id+x}\" && test \"$ACME_OPERATIONAL_TEST\" = retained && test -n \"$AWS_ACCESS_KEY_ID\" && test -n \"$AWS_SECRET_ACCESS_KEY\"",
            ],
            &[("AWS_ACCESS_KEY_ID", "selected-aws-key"), ("AWS_SECRET_ACCESS_KEY", "selected-aws-secret")],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_command_future_terminates_child() {
        let directory = tempfile::tempdir().unwrap();
        let ready_arc = Arc::new(directory.path().join("ready"));
        let leaked_arc = Arc::new(directory.path().join("cancelled-child-survived"));
        let ready_arc2 = ready_arc.clone();
        let leaked_arc2 = leaked_arc.clone();
        let task = tokio::spawn(async move {
            run_command_with_timeout(
                "sh",
                &[
                    "-c",
                    "printf ready > \"$1\"; sleep 60; printf leaked > \"$2\"",
                    "sh",
                    ready_arc.to_str().unwrap(),
                    leaked_arc.to_str().unwrap(),
                ],
                &[],
                Duration::from_secs(5),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ready_arc2.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!leaked_arc2.exists());
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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn timeout_terminates_detached_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("detached-descendant-survived");
        let result = tokio::time::timeout(
            Duration::from_millis(600),
            run_command_with_timeout(
                "sh",
                &[
                    "-c",
                    "setsid sh -c '(sleep 1; printf leaked > \"$1\")' sh \"$1\" & wait",
                    "sh",
                    marker.to_str().unwrap(),
                ],
                &[],
                Duration::from_millis(100),
            ),
        )
        .await;
        let bounded =
            matches!(result, Ok(Err(ref error)) if error.to_string().contains("timed out"));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(bounded, "runner exceeded its timeout");
        assert!(!marker.exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn timeout_tracks_detached_descendant_after_root_exits() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("orphaned-descendant-survived");
        let result = tokio::time::timeout(
            Duration::from_millis(600),
            run_command_with_timeout(
                "sh",
                &[
                    "-c",
                    "setsid sh -c '(sleep 1; printf leaked > \"$1\")' sh \"$1\" &",
                    "sh",
                    marker.to_str().unwrap(),
                ],
                &[],
                Duration::from_millis(100),
            ),
        )
        .await;
        let bounded =
            matches!(result, Ok(Err(ref error)) if error.to_string().contains("timed out"));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(bounded, "runner exceeded its timeout");
        assert!(!marker.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_identity_rejects_reused_pid() {
        let expected = ProcessIdentity {
            pid: 42,
            start_time: 100,
        };
        let same = ProcessEntry {
            identity: expected,
            state: 'S',
        };
        let reused = ProcessEntry {
            identity: ProcessIdentity {
                pid: 42,
                start_time: 101,
            },
            state: 'S',
        };
        assert!(identity_matches(expected, Some(same)));
        assert!(!identity_matches(expected, Some(reused)));
        assert!(!identity_matches(expected, None));
    }
}
