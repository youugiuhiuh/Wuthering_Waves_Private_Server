use crate::core::{paths::acme, types::DnsProvider};
use anyhow::{Context, Result, bail};
use percent_encoding::percent_decode_str;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use x509_parser::{extensions::GeneralName, pem::Pem, prelude::FromDer};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MIN_VALIDITY: u64 = 30 * 24 * 60 * 60;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const ACME_SERVER: &str = "letsencrypt";
// Two passes cover direct and once-wrapped provider output without unbounded decoding.
const CREDENTIAL_ENCODING_DEPTH: usize = 2;
#[cfg(target_os = "linux")]
const PROCESS_TOKEN_ENV: &str = "AEGIS_ACME_PROCESS_TOKEN";

const DNS_CREDENTIAL_VARS: &[&str] = &[
    "Ali_Key",
    "Ali_Secret",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "CF_Token",
    "CF_Zone_ID",
    "CF_Account_ID",
    "DP_Id",
    "DP_Key",
    "SAVED_Ali_Key",
    "SAVED_Ali_Secret",
    "SAVED_AWS_ACCESS_KEY_ID",
    "SAVED_AWS_SECRET_ACCESS_KEY",
    "SAVED_CF_Token",
    "SAVED_CF_Zone_ID",
    "SAVED_CF_Account_ID",
    "SAVED_DP_Id",
    "SAVED_DP_Key",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPaths {
    pub fullchain: PathBuf,
    pub privkey: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcmeFailureKind {
    Authentication,
    Scope,
    Dns,
    Network,
    Timeout,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcmeCommandError {
    kind: AcmeFailureKind,
}

impl AcmeCommandError {
    pub fn new(kind: AcmeFailureKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> AcmeFailureKind {
        self.kind
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            AcmeFailureKind::Authentication => "ACME-AUTH",
            AcmeFailureKind::Scope => "ACME-SCOPE",
            AcmeFailureKind::Dns => "ACME-DNS",
            AcmeFailureKind::Network => "ACME-NETWORK",
            AcmeFailureKind::Timeout => "ACME-TIMEOUT",
            AcmeFailureKind::Unknown => "ACME-UNKNOWN",
        }
    }
}

impl fmt::Display for AcmeCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AcmeCommandError {}

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
        let primary_args = if paths.fullchain.exists() || paths.privkey.exists() {
            Self::renew_args(&domain)?
        } else {
            Self::issue_args(&domain, provider)?
        };
        let commands = vec![primary_args, Self::install_args(&domain)?];
        let names = provider.credential_names();
        let environment = credentials
            .map(|(first, second)| {
                vec![
                    (names.0.to_string(), first.to_string()),
                    (names.1.to_string(), second.to_string()),
                ]
            })
            .unwrap_or_default();

        let command_result =
            execute_acme_sequence(commands, environment, |args, environment| async move {
                let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
                let env_refs = environment
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str()))
                    .collect::<Vec<_>>();
                run_command(acme::BIN, &arg_refs, &env_refs)
                    .await
                    .map(|_| ())
            })
            .await;
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
        let domain = Self::validate_domain(domain)?;
        Ok(vec![
            "--issue".to_string(),
            "--server".to_string(),
            ACME_SERVER.to_string(),
            "--ecc".to_string(),
            "--dns".to_string(),
            provider.acme_flag().to_string(),
            "-d".to_string(),
            domain,
        ])
    }

    fn renew_args(domain: &str) -> Result<Vec<String>> {
        let domain = Self::validate_domain(domain)?;
        Ok(vec![
            "--renew".to_string(),
            "--server".to_string(),
            ACME_SERVER.to_string(),
            "--ecc".to_string(),
            "-d".to_string(),
            domain,
            "--force".to_string(),
        ])
    }

    fn install_args(domain: &str) -> Result<Vec<String>> {
        let domain = Self::validate_domain(domain)?;
        let paths = Self::cert_paths(&domain)?;
        Ok(vec![
            "--install-cert".to_string(),
            "--ecc".to_string(),
            "-d".to_string(),
            domain,
            "--fullchain-file".to_string(),
            paths.fullchain.to_string_lossy().into_owned(),
            "--key-file".to_string(),
            paths.privkey.to_string_lossy().into_owned(),
        ])
    }
}

async fn execute_acme_sequence<F, Fut>(
    commands: Vec<Vec<String>>,
    environment: Vec<(String, String)>,
    mut execute: F,
) -> Result<()>
where
    F: FnMut(Vec<String>, Vec<(String, String)>) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut first_environment = Some(environment);
    for args in commands {
        execute(args, first_environment.take().unwrap_or_default()).await?;
    }
    Ok(())
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

fn classify_acme_failure(
    stdout: &[u8],
    stderr: &[u8],
    environment: &[(String, String)],
) -> AcmeFailureKind {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let mut values = environment
        .iter()
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    values.dedup();

    let mut representations = values
        .into_iter()
        .flat_map(|value| {
            std::iter::once(normalize_percent_escapes(value.to_string()))
                .chain(canonicalized_views(value))
        })
        .collect::<Vec<_>>();
    representations
        .sort_unstable_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    representations.dedup();
    let texts = canonicalized_views(&text)
        .into_iter()
        .map(|mut text| {
            for representation in &representations {
                text = text.replace(representation, "[REDACTED]");
            }
            text.to_ascii_lowercase()
        })
        .collect::<Vec<_>>();

    let contains = |signatures: &[&str]| {
        signatures
            .iter()
            .any(|signature| texts.iter().all(|text| text.contains(signature)))
    };
    if contains(&[
        "invalid access token",
        "invalid api token",
        "authentication error",
        "unauthorized",
        "\"code\":9109",
        "\"code\":10000",
        "signaturedoesnotmatch",
        "invalidclienttokenid",
        "invalidaccesskeyid",
    ]) {
        AcmeFailureKind::Authentication
    } else if contains(&[
        "invalid domain",
        "permission denied",
        "forbidden",
        "not authorized",
        "accessdenied",
        "zone not found",
        "no matching zone",
        "domain not found",
    ]) {
        AcmeFailureKind::Scope
    } else if contains(&[
        "add txt record error",
        "error adding txt",
        "can not get record id",
        "delete record error",
        "dns problem",
        "dns validation error",
    ]) {
        AcmeFailureKind::Dns
    } else if contains(&[
        "could not resolve host",
        "connection timed out",
        "connection refused",
        "network is unreachable",
        "ssl connect error",
        "tls connect error",
        "order status",
        "rate limit",
    ]) {
        AcmeFailureKind::Network
    } else {
        AcmeFailureKind::Unknown
    }
}

fn normalize_percent_escapes(text: String) -> String {
    let mut bytes = text.into_bytes();
    for index in 0..bytes.len().saturating_sub(2) {
        if bytes[index] == b'%'
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            bytes[index + 1].make_ascii_uppercase();
            bytes[index + 2].make_ascii_uppercase();
        }
    }
    String::from_utf8(bytes).expect("percent-escape normalization preserves UTF-8")
}

fn canonicalized_views(value: &str) -> Vec<String> {
    let raw = normalize_percent_escapes(value.to_string());
    let mut percent = raw.clone();
    let mut form = raw;
    for _ in 0..CREDENTIAL_ENCODING_DEPTH {
        percent = normalize_percent_escapes(
            percent_decode_str(&percent)
                .decode_utf8_lossy()
                .into_owned(),
        );
        form = normalize_percent_escapes(
            percent_decode_str(&form.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned(),
        );
    }
    let mut views = vec![percent];
    if views[0] != form {
        views.push(form);
    }
    views
}

fn acme_command_failure(
    is_acme_program: bool,
    timed_out: bool,
    stdout: &[u8],
    stderr: &[u8],
    environment: &[(String, String)],
) -> Option<AcmeCommandError> {
    if !is_acme_program {
        return None;
    }
    let kind = if timed_out {
        AcmeFailureKind::Timeout
    } else {
        classify_acme_failure(stdout, stderr, environment)
    };
    Some(AcmeCommandError::new(kind))
}

fn is_acme_program(program: &str) -> bool {
    program == acme::BIN
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
    run_command_inner(
        program,
        args,
        owned_env,
        &stripped_env,
        COMMAND_TIMEOUT,
        is_acme_program(program),
    )
    .await
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
    run_command_inner(
        program,
        args,
        owned_env,
        &stripped_env,
        timeout,
        is_acme_program(program),
    )
    .await
}

async fn run_command_inner(
    program: &str,
    args: &[&str],
    environment: Vec<(String, String)>,
    strip_vars: &[String],
    timeout: Duration,
    is_acme_program: bool,
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

    let child = command
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;
    let mut cleanup = CommandCleanup::new(child);
    let process_id = cleanup
        .child_mut()
        .id()
        .context("subprocess has no process ID")? as i32;
    #[cfg(target_os = "linux")]
    let process_scope =
        initialize_process_scope(process_id, process_token, cleanup.child_mut()).await?;
    #[cfg(not(target_os = "linux"))]
    let process_scope = ProcessScope { pid: process_id };
    cleanup.set_scope(process_scope);
    let mut stdout = cleanup
        .child_mut()
        .stdout
        .take()
        .context("subprocess stdout unavailable")?;
    let mut stderr = cleanup
        .child_mut()
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
    let status = match tokio::time::timeout_at(deadline, cleanup.child_mut().wait()).await {
        Ok(status) => status.context("failed to wait for subprocess")?,
        Err(_) => {
            stdout_task.abort();
            stderr_task.abort();
            cleanup.cleanup().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            if let Some(error) = acme_command_failure(is_acme_program, true, b"", b"", &environment)
            {
                return Err(error.into());
            }
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
            cleanup.cleanup().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            if let Some(error) = acme_command_failure(is_acme_program, true, b"", b"", &environment)
            {
                return Err(error.into());
            }
            bail!("command timed out after {} seconds", timeout.as_secs());
        }
    };
    cleanup.disarm();
    if !status.success() {
        if let Some(error) =
            acme_command_failure(is_acme_program, false, &stdout, &stderr, &environment)
        {
            return Err(error.into());
        }
        bail!("command {program} failed with status {status}");
    }
    Ok(Output {
        status,
        stdout: if is_acme_program { Vec::new() } else { stdout },
        stderr: if is_acme_program { Vec::new() } else { stderr },
    })
}

async fn terminate_subprocess_tree(scope: &ProcessScope, child: &mut tokio::process::Child) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = scope.pid;
        let _ = child.start_kill();
        let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
    }
    #[cfg(target_os = "linux")]
    terminate_and_reap_process_scope(scope, child).await;
}

#[cfg(target_os = "linux")]
async fn terminate_and_reap_process_scope(scope: &ProcessScope, child: &mut tokio::process::Child) {
    let scope = scope.clone();
    let descendants = tokio::time::timeout(
        CLEANUP_TIMEOUT * 2,
        tokio::task::spawn_blocking(move || stop_and_kill_subtree(&scope)),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
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

struct CommandCleanup {
    child: Option<tokio::process::Child>,
    scope: Option<ProcessScope>,
    runtime: tokio::runtime::Handle,
    #[cfg(all(test, target_os = "linux"))]
    cleanup_pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
}

impl CommandCleanup {
    fn new(child: tokio::process::Child) -> Self {
        Self {
            child: Some(child),
            scope: None,
            runtime: tokio::runtime::Handle::current(),
            #[cfg(all(test, target_os = "linux"))]
            cleanup_pause: None,
        }
    }

    fn child_mut(&mut self) -> &mut tokio::process::Child {
        self.child.as_mut().expect("child is present until cleanup")
    }

    fn set_scope(&mut self, scope: ProcessScope) {
        self.scope = Some(scope);
    }

    fn start_cleanup(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let mut child = self.child.take()?;
        let scope = self.scope.take();
        #[cfg(all(test, target_os = "linux"))]
        let cleanup_pause = self.cleanup_pause.take();
        Some(self.runtime.spawn(async move {
            #[cfg(all(test, target_os = "linux"))]
            if let Some((started, resume)) = cleanup_pause {
                let _ = started.send(());
                let _ = resume.await;
            }
            cleanup_owned_child(&mut child, scope.as_ref()).await;
        }))
    }

    async fn cleanup(&mut self) {
        if let Some(task) = self.start_cleanup()
            && let Err(error) = task.await
        {
            log::error!("command cleanup task failed: {error}");
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    fn pause_cleanup(
        &mut self,
        started: tokio::sync::oneshot::Sender<()>,
        resume: tokio::sync::oneshot::Receiver<()>,
    ) {
        self.cleanup_pause = Some((started, resume));
    }

    fn disarm(&mut self) {
        self.scope = None;
        self.child = None;
    }
}

impl Drop for CommandCleanup {
    fn drop(&mut self) {
        drop(self.start_cleanup());
    }
}

async fn cleanup_owned_child(child: &mut tokio::process::Child, scope: Option<&ProcessScope>) {
    if let Some(scope) = scope {
        terminate_subprocess_tree(scope, child).await;
    } else {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
    }
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
            let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
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
    use std::{
        path::PathBuf,
        process::Stdio,
        sync::{Arc, Mutex},
    };

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
    fn classifies_safe_acme_failures() {
        let cases = [
            (
                r#"{\"errors\":[{\"code\":9109,\"message\":\"Invalid access token\"}]}"#,
                AcmeFailureKind::Authentication,
            ),
            ("invalid domain", AcmeFailureKind::Scope),
            ("Add txt record error.", AcmeFailureKind::Dns),
            ("curl: (6) Could not resolve host", AcmeFailureKind::Network),
            ("unrecognized provider response", AcmeFailureKind::Unknown),
        ];
        for (stderr, expected) in cases {
            assert_eq!(classify_acme_failure(b"", stderr.as_bytes(), &[]), expected);
        }
    }

    #[test]
    fn command_error_never_contains_credentials_or_raw_output() {
        let environment = vec![
            ("CF_Token".to_string(), "token-secret/value".to_string()),
            ("CF_Account_ID".to_string(), "account-secret".to_string()),
        ];
        let kind = classify_acme_failure(
            b"",
            b"unrecognized token-secret/value account-secret private detail",
            &environment,
        );
        let error = AcmeCommandError::new(kind).to_string();
        assert_eq!(error, "ACME-UNKNOWN");
        assert!(!error.contains("token-secret"));
        assert!(!error.contains("account-secret"));
        assert!(!error.contains("private detail"));
    }

    #[test]
    fn credential_redaction_handles_overlapping_values() {
        let environment = vec![
            ("SHORT".to_string(), "prefix-".to_string()),
            ("LONG".to_string(), "prefix-unauthorized".to_string()),
        ];
        assert_eq!(
            classify_acme_failure(b"", b"prefix-unauthorized", &environment),
            AcmeFailureKind::Unknown
        );
    }

    #[test]
    fn credential_redaction_handles_case_equivalent_percent_escapes() {
        let environment = vec![("TOKEN".to_string(), "a/:unauthorized".to_string())];
        assert_eq!(
            classify_acme_failure(b"", b"a%2f%3Aunauthorized", &environment),
            AcmeFailureKind::Unknown
        );
    }

    #[test]
    fn credential_redaction_handles_form_encoded_spaces() {
        let environment = vec![("TOKEN".to_string(), "secret unauthorized".to_string())];
        assert_eq!(
            classify_acme_failure(b"", b"secret+unauthorized", &environment),
            AcmeFailureKind::Unknown
        );
    }

    #[test]
    fn credential_redaction_handles_mixed_percent_encoding() {
        let environment = vec![("TOKEN".to_string(), "a/:unauthorized".to_string())];
        assert_eq!(
            classify_acme_failure(b"", b"a/%3Aunauthorized", &environment),
            AcmeFailureKind::Unknown
        );
    }

    #[test]
    fn credential_redaction_handles_two_encoding_levels() {
        let environment = vec![("TOKEN".to_string(), "a/:unauthorized".to_string())];
        assert_eq!(
            classify_acme_failure(b"", b"a%252F%3Aunauthorized", &environment),
            AcmeFailureKind::Unknown
        );
    }

    #[test]
    fn malformed_percent_escapes_remain_classifiable() {
        assert_eq!(
            classify_acme_failure(b"", b"%GG unauthorized", &[]),
            AcmeFailureKind::Authentication
        );
    }

    #[test]
    fn canonicalization_handles_fully_encoded_and_malformed_values() {
        let credential = canonicalized_views("a/:unauthorized");
        let fully_encoded = canonicalized_views("%61%2F%3A%75%6E%61%75%74%68%6F%72%69%7A%65%64");

        assert!(fully_encoded.iter().any(|view| credential.contains(view)));
        assert_eq!(canonicalized_views("%GG"), vec!["%GG"]);
    }

    #[test]
    fn converts_acme_command_failures_to_typed_errors() {
        assert!(is_acme_program(acme::BIN));
        assert!(!is_acme_program("sh"));
        let classified =
            acme_command_failure(true, false, b"", b"Add txt record error.", &[]).unwrap();
        assert_eq!(classified.kind(), AcmeFailureKind::Dns);

        let timed_out = acme_command_failure(true, true, b"", b"", &[]).unwrap();
        assert_eq!(timed_out.kind(), AcmeFailureKind::Timeout);
        assert!(acme_command_failure(false, false, b"", b"invalid domain", &[]).is_none());
        assert!(acme_command_failure(false, true, b"", b"", &[]).is_none());
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
    fn issue_arguments_use_letsencrypt_without_destination_paths() {
        assert_eq!(
            AcmeManager::issue_args("example.com", DnsProvider::Cloudflare).unwrap(),
            vec![
                "--issue",
                "--server",
                "letsencrypt",
                "--ecc",
                "--dns",
                "dns_cf",
                "-d",
                "example.com",
            ]
        );
    }

    #[test]
    fn renewal_arguments_use_letsencrypt_and_force() {
        assert_eq!(
            AcmeManager::renew_args("example.com").unwrap(),
            vec![
                "--renew",
                "--server",
                "letsencrypt",
                "--ecc",
                "-d",
                "example.com",
                "--force",
            ]
        );
    }

    #[test]
    fn install_arguments_target_production_certificate_paths() {
        assert_eq!(
            AcmeManager::install_args("example.com").unwrap(),
            vec![
                "--install-cert",
                "--ecc",
                "-d",
                "example.com",
                "--fullchain-file",
                "/root/cert/example.com/fullchain.pem",
                "--key-file",
                "/root/cert/example.com/privkey.pem",
            ]
        );
    }

    #[tokio::test]
    async fn acme_sequence_stops_before_install_after_primary_failure() {
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let recorded = Arc::clone(&calls);
        let commands = vec![
            vec!["--issue".to_string()],
            vec!["--install-cert".to_string()],
        ];

        let result = execute_acme_sequence(
            commands,
            vec![("CF_Token".to_string(), "secret".to_string())],
            move |args, environment| {
                let recorded = Arc::clone(&recorded);
                async move {
                    recorded.lock().unwrap().push(args);
                    assert_eq!(environment.len(), 1);
                    bail!("primary failed")
                }
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(*calls.lock().unwrap(), vec![vec!["--issue".to_string()]]);
    }

    #[tokio::test]
    async fn acme_sequence_installs_without_provider_credentials() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        execute_acme_sequence(
            vec![
                vec!["--issue".to_string()],
                vec!["--install-cert".to_string()],
            ],
            vec![
                ("CF_Token".to_string(), "token".to_string()),
                ("CF_Zone_ID".to_string(), "zone".to_string()),
            ],
            move |args, environment| {
                let recorded = Arc::clone(&recorded);
                async move {
                    recorded.lock().unwrap().push((args, environment));
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1.len(), 2);
        assert!(calls[1].1.is_empty());
    }

    #[tokio::test]
    async fn acme_sequence_completes_only_after_install_succeeds() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        execute_acme_sequence(
            vec![
                vec!["--issue".to_string()],
                vec!["--install-cert".to_string()],
            ],
            Vec::new(),
            move |args, _| {
                let recorded = Arc::clone(&recorded);
                async move {
                    recorded.lock().unwrap().push(args.clone());
                    if args[0] == "--install-cert" {
                        bail!("install failed");
                    }
                    Ok(())
                }
            },
        )
        .await
        .unwrap_err();

        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                vec!["--issue".to_string()],
                vec!["--install-cert".to_string()],
            ]
        );
    }

    #[test]
    fn every_ca_command_is_letsencrypt_only() {
        for args in [
            AcmeManager::issue_args("example.com", DnsProvider::Cloudflare).unwrap(),
            AcmeManager::renew_args("example.com").unwrap(),
        ] {
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--server", "letsencrypt"])
            );
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.to_ascii_lowercase().contains("zerossl"))
            );
        }
    }

    #[test]
    fn detects_non_empty_saved_provider_credentials() {
        let config = "SAVED_CF_Token='token-value'\nSAVED_CF_Zone_ID='zone-value'\n";
        assert_eq!(
            configured_provider_from(config),
            Some(DnsProvider::Cloudflare)
        );
    }

    #[test]
    fn cloudflare_account_id_is_not_a_supported_contract() {
        let config = "SAVED_CF_Token='token-value'\nSAVED_CF_Account_ID='account-value'\n";
        assert_eq!(configured_provider_from(config), None);
    }

    #[test]
    fn ignores_empty_saved_provider_credentials() {
        let config = "SAVED_CF_Token=''\nSAVED_CF_Zone_ID='zone-value'\n";
        assert_eq!(configured_provider_from(config), None);
    }

    #[test]
    fn ignores_quoted_empty_credentials_followed_by_comments() {
        let config = "SAVED_CF_Token='' # unset\nSAVED_CF_Zone_ID='zone-value'\n";
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

    #[tokio::test]
    async fn acme_like_success_discards_captured_output_at_process_boundary() {
        let output = run_command_inner(
            "sh",
            &[
                "-c",
                "printf success-secret; printf success-error-secret >&2",
            ],
            Vec::new(),
            &[],
            Duration::from_secs(1),
            true,
        )
        .await
        .unwrap();

        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    #[tokio::test]
    async fn acme_like_nonzero_returns_typed_error_at_process_boundary() {
        let error = run_command_inner(
            "sh",
            &["-c", "printf 'Add txt record error. private' >&2; exit 7"],
            Vec::new(),
            &[],
            Duration::from_secs(1),
            true,
        )
        .await
        .unwrap_err();

        assert_eq!(
            error
                .downcast_ref::<AcmeCommandError>()
                .map(|error| error.kind()),
            Some(AcmeFailureKind::Dns)
        );
        assert_eq!(error.to_string(), "ACME-DNS");
    }

    #[tokio::test]
    async fn non_acme_success_preserves_captured_output() {
        let output = run_command_inner(
            "sh",
            &["-c", "printf ordinary-output; printf ordinary-error >&2"],
            Vec::new(),
            &[],
            Duration::from_secs(1),
            false,
        )
        .await
        .unwrap();

        assert_eq!(output.stdout, b"ordinary-output");
        assert_eq!(output.stderr, b"ordinary-error");
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn child_receives_only_selected_provider_credentials() {
        let _environment = [
            EnvRestore::set("Ali_Key", "parent-ali-key"),
            EnvRestore::set("SAVED_AWS_SECRET_ACCESS_KEY", "parent-aws-secret"),
            EnvRestore::set("SAVED_CF_Token", "parent-saved-token"),
            EnvRestore::set("CF_Account_ID", "parent-account-id"),
            EnvRestore::set("SAVED_CF_Account_ID", "parent-saved-account-id"),
            EnvRestore::set("ACME_OPERATIONAL_TEST", "retained"),
        ];
        run_command_with_timeout(
            "sh",
            &[
                "-c",
                "test -z \"${Ali_Key+x}\" && test -z \"${SAVED_AWS_SECRET_ACCESS_KEY+x}\" && test -z \"${SAVED_CF_Token+x}\" && test -z \"${CF_Account_ID+x}\" && test -z \"${SAVED_CF_Account_ID+x}\" && test \"$ACME_OPERATIONAL_TEST\" = retained && test -n \"$CF_Token\" && test -n \"$CF_Zone_ID\"",
            ],
            &[("CF_Token", "selected-token"), ("CF_Zone_ID", "selected-zone")],
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
    async fn startup_cleanup_terminates_self_stopped_wrapper() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "kill -STOP $$; exec sleep 60"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id().unwrap() as i32;
        let cleanup = CommandCleanup::new(child);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !process_entry(pid).is_some_and(|entry| entry.state == 'T') {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        drop(cleanup);

        tokio::time::timeout(Duration::from_secs(2), async {
            while process_entry(pid).is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("startup cleanup left the self-stopped wrapper alive");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_command_future_terminates_child() {
        let directory = tempfile::tempdir().unwrap();
        let root_pid = Arc::new(directory.path().join("root-pid"));
        let descendant_pid = Arc::new(directory.path().join("descendant-pid"));
        let root_pid_for_task = root_pid.clone();
        let descendant_pid_for_task = descendant_pid.clone();
        let task = tokio::spawn(async move {
            run_command_with_timeout(
                "sh",
                &[
                    "-c",
                    "printf %s $$ > \"$1\"; setsid sh -c 'printf %s $$ > \"$1\"; exec sleep 60' sh \"$2\" & wait",
                    "sh",
                    root_pid_for_task.to_str().unwrap(),
                    descendant_pid_for_task.to_str().unwrap(),
                ],
                &[],
                Duration::from_secs(60),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !root_pid.exists() || !descendant_pid.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let root_pid = fs::read_to_string(&*root_pid)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let descendant_pid = fs::read_to_string(&*descendant_pid)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_entry(root_pid).is_some() || process_entry(descendant_pid).is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("cancelled command left its child or detached descendant alive");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_explicit_cleanup_keeps_process_tree_cleanup_running() {
        let directory = tempfile::tempdir().unwrap();
        let root_pid_path = directory.path().join("root-pid");
        let descendant_pid_path = directory.path().join("descendant-pid");
        let token = process_token();
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "kill -STOP $$; exec \"$@\"",
                "aegis-cleanup-test",
                "sh",
                "-c",
                "printf %s $$ > \"$1\"; setsid sh -c 'printf %s $$ > \"$1\"; exec sleep 60' sh \"$2\" & wait",
                "sh",
                root_pid_path.to_str().unwrap(),
                descendant_pid_path.to_str().unwrap(),
            ])
            .env(PROCESS_TOKEN_ENV, &token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id().unwrap() as i32;
        let mut cleanup = CommandCleanup::new(child);
        let scope = initialize_process_scope(pid, token, cleanup.child_mut())
            .await
            .unwrap();
        let rescue_scope = scope.clone();
        cleanup.set_scope(scope);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !root_pid_path.exists() || !descendant_pid_path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let root_pid = fs::read_to_string(root_pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let descendant_pid = fs::read_to_string(descendant_pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        cleanup.pause_cleanup(started_tx, resume_rx);
        let task = tokio::spawn(async move {
            cleanup.cleanup().await;
        });
        started_rx.await.unwrap();

        task.abort();
        let _ = task.await;
        let _ = resume_tx.send(());

        let reaped = tokio::time::timeout(Duration::from_secs(2), async {
            while process_entry(root_pid).is_some() || process_entry(descendant_pid).is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        if reaped.is_err() {
            let identities =
                tokio::task::spawn_blocking(move || stop_and_kill_subtree(&rescue_scope))
                    .await
                    .unwrap();
            tokio::task::spawn_blocking(move || reap_descendants(identities))
                .await
                .unwrap();
        }
        assert!(
            reaped.is_ok(),
            "cancelling explicit cleanup left its child or detached descendant alive"
        );
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
