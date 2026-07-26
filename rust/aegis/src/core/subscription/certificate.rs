use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufReader, Cursor, Write},
    net::IpAddr,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rustls::{
    Certificate, ClientConfig, ClientConnection, Error as RustlsError, PrivateKey, ServerConfig,
    ServerConnection,
    client::{ServerCertVerified, ServerCertVerifier, ServerName},
};
use tokio::{process::Command, time::timeout};
use x509_parser::{extensions::GeneralName, prelude::*};

use super::config::{CertificateMode, SubscriptionConfig};
use crate::core::security::firewall::FirewallManager;

const ACME_TIMEOUT: Duration = Duration::from_secs(120);
const ISSUE_TIMEOUT: Duration = Duration::from_secs(300);

pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(
        &self,
        program: &Path,
        args: &[OsString],
        timeout: Duration,
    ) -> Result<CommandOutput>;
}

#[async_trait]
pub trait AcmeFirewall: Send + Sync {
    async fn is_open(&self, port: u16) -> Result<bool>;
    async fn open(&self, port: u16) -> Result<()>;
    async fn close(&self, port: u16) -> Result<()>;
}

#[derive(Debug)]
pub struct ValidatedCertificate {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub not_after: SystemTime,
}

#[async_trait]
pub trait CertificatePromotion: Send {
    async fn commit(self: Box<Self>) -> Result<()>;
    async fn rollback(self: Box<Self>) -> Result<()>;
}

#[derive(Clone, Copy, Default)]
pub struct SystemCommandRunner;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(
        &self,
        program: &Path,
        args: &[OsString],
        timeout_duration: Duration,
    ) -> Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args).kill_on_drop(true);
        let output = timeout(timeout_duration, command.output())
            .await
            .with_context(|| format!("{} timed out", program.display()))?
            .with_context(|| format!("failed to run {}", program.display()))?;
        Ok(CommandOutput {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Clone, Copy, Default)]
pub struct SystemAcmeFirewall;

#[async_trait]
impl AcmeFirewall for SystemAcmeFirewall {
    async fn is_open(&self, port: u16) -> Result<bool> {
        Ok(FirewallManager::list_allowed_ports().await?.contains(&port))
    }

    async fn open(&self, port: u16) -> Result<()> {
        FirewallManager::add_port(port).await
    }

    async fn close(&self, port: u16) -> Result<()> {
        FirewallManager::remove_port(port).await
    }
}

pub struct CertificateManager<R: CommandRunner, F: AcmeFirewall> {
    runner: R,
    firewall: F,
    acme_sh: PathBuf,
    staging_dir: PathBuf,
}

impl<R: CommandRunner, F: AcmeFirewall> CertificateManager<R, F> {
    pub fn new(runner: R, firewall: F, acme_sh: PathBuf, staging_dir: PathBuf) -> Self {
        Self {
            runner,
            firewall,
            acme_sh,
            staging_dir,
        }
    }

    pub async fn issue(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        let result = self.issue_inner(config).await;
        if result.is_err() {
            let _ = fs::remove_dir_all(&self.staging_dir);
        }
        result
    }

    async fn issue_inner(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        self.ensure_acme_sh().await?;
        if self.staging_dir.exists() {
            fs::remove_dir_all(&self.staging_dir).context("failed to reset certificate staging")?;
        }
        fs::create_dir_all(&self.staging_dir).context("failed to create certificate staging")?;
        fs::set_permissions(&self.staging_dir, fs::Permissions::from_mode(0o700))?;
        let cert_path = self.staging_dir.join("fullchain.pem");
        let key_path = self.staging_dir.join("key.pem");
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&key_path)
            .context("failed to prepare staged private key")?;

        let host = config
            .public_host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let mut issue_args = vec![
            "--server".into(),
            "letsencrypt".into(),
            "--issue".into(),
            "--standalone".into(),
        ];
        if config.certificate_mode == CertificateMode::Ip {
            issue_args.extend([
                "--certificate-profile".into(),
                "shortlived".into(),
                "--force".into(),
            ]);
        }
        issue_args.extend(["-d".into(), host.into()]);
        issue_args.extend(["--keylength".into(), "ec-256".into()]);
        if let Some(ipv6) = config.ipv6_san {
            issue_args.extend(["-d".into(), ipv6.to_string().into()]);
        }

        let already_open = self.firewall.is_open(80).await?;
        if !already_open {
            self.firewall.open(80).await?;
        }
        let issued = self
            .run_checked(&self.acme_sh, &issue_args, ISSUE_TIMEOUT, "issuance")
            .await;
        let closed = if already_open {
            Ok(())
        } else {
            self.firewall.close(80).await
        };
        issued?;
        closed.context("failed to restore ACME firewall rule")?;

        let install_args = vec![
            "--install-cert".into(),
            "--ecc".into(),
            "-d".into(),
            host.into(),
            "--fullchain-file".into(),
            cert_path.as_os_str().to_owned(),
            "--key-file".into(),
            key_path.as_os_str().to_owned(),
        ];
        self.run_checked(&self.acme_sh, &install_args, ISSUE_TIMEOUT, "installation")
            .await?;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        self.validate(config, &cert_path, &key_path)
    }

    pub async fn renew(
        &self,
        config: &SubscriptionConfig,
        current: ValidatedCertificate,
    ) -> Result<ValidatedCertificate> {
        if self.should_renew(config.certificate_mode, current.not_after) {
            self.issue(config).await
        } else {
            Ok(current)
        }
    }

    pub fn should_renew(&self, mode: CertificateMode, not_after: SystemTime) -> bool {
        not_after
            .duration_since(SystemTime::now())
            .map_or(true, |remaining| remaining <= renew_before(mode))
    }

    async fn run_checked(
        &self,
        program: &Path,
        args: &[OsString],
        timeout: Duration,
        operation: &str,
    ) -> Result<()> {
        let output =
            self.runner.run(program, args, timeout).await.map_err(|_| {
                anyhow::anyhow!("ACME {operation} failed for {}", program.display())
            })?;
        if !output.status.success() {
            bail!(
                "ACME {operation} failed for {} with status {}",
                program.display(),
                output.status
            );
        }
        Ok(())
    }

    async fn ensure_acme_sh(&self) -> Result<()> {
        if self.acme_sh.exists() {
            return Ok(());
        }
        let parent = self.acme_sh.parent().context("invalid acme.sh path")?;
        fs::create_dir_all(parent).context("failed to create acme.sh directory")?;
        let response = reqwest::Client::new()
            .get("https://get.acme.sh")
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("failed to download acme.sh installer"))?;
        if !response.status().is_success() {
            bail!("failed to download acme.sh installer");
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| anyhow::anyhow!("failed to read acme.sh installer"))?;
        let installer = parent.join(format!(".installer-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(&installer)
            .context("failed to create acme.sh installer")?;
        file.write_all(&body)?;
        file.sync_all()?;
        let result = self
            .run_checked(
                Path::new("sh"),
                &[installer.as_os_str().to_owned()],
                ACME_TIMEOUT,
                "installer",
            )
            .await;
        let _ = fs::remove_file(installer);
        result?;
        if !self.acme_sh.exists() {
            bail!("acme.sh installer did not create the executable");
        }
        Ok(())
    }

    pub fn validate(
        &self,
        config: &SubscriptionConfig,
        cert_path: &Path,
        key_path: &Path,
    ) -> Result<ValidatedCertificate> {
        let cert_raw = fs::read(cert_path).context("failed to read certificate chain")?;
        let certs = rustls_pemfile::certs(&mut BufReader::new(cert_raw.as_slice()))
            .context("failed to parse certificate chain")?;
        if certs.is_empty() {
            bail!("certificate chain is empty");
        }
        let key_raw = fs::read(key_path).context("failed to read private key")?;
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut BufReader::new(key_raw.as_slice()))
            .context("failed to parse private key")?;
        if keys.is_empty() {
            keys = rustls_pemfile::rsa_private_keys(&mut BufReader::new(key_raw.as_slice()))
                .context("failed to parse private key")?;
        }
        if keys.len() != 1 {
            bail!("certificate requires exactly one private key");
        }
        let rustls_certs = certs.iter().cloned().map(Certificate).collect();
        let server_config = ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(rustls_certs, PrivateKey(keys.remove(0)))
            .context("certificate and private key do not match")?;
        prove_key_match(server_config)?;

        let (_, leaf) = X509Certificate::from_der(&certs[0])
            .map_err(|_| anyhow::anyhow!("failed to parse leaf certificate"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time precedes Unix epoch")?
            .as_secs() as i64;
        let now = ASN1Time::from_timestamp(now).context("invalid system time")?;
        if !leaf.validity().is_valid_at(now) {
            bail!("certificate is not currently valid");
        }
        let san = leaf
            .subject_alternative_name()
            .context("failed to parse certificate SAN")?
            .context("certificate has no SAN")?;
        let names = &san.value.general_names;
        let host = config
            .public_host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        if !san_matches(names, host) {
            bail!("certificate SAN does not match configured host");
        }
        if let Some(ipv6) = config.ipv6_san
            && !san_matches(names, &ipv6.to_string())
        {
            bail!("certificate SAN does not include configured IPv6 address");
        }
        let timestamp = leaf.validity().not_after.timestamp();
        let not_after = UNIX_EPOCH
            .checked_add(Duration::from_secs(
                timestamp
                    .try_into()
                    .context("certificate expiration precedes Unix epoch")?,
            ))
            .context("certificate expiration is out of range")?;
        Ok(ValidatedCertificate {
            cert_path: cert_path.to_owned(),
            key_path: key_path.to_owned(),
            not_after,
        })
    }

    pub async fn promote(
        &self,
        config: &SubscriptionConfig,
        staged: ValidatedCertificate,
    ) -> Result<Box<dyn CertificatePromotion>> {
        let cert_parent = config
            .cert_path
            .parent()
            .context("live certificate has no parent directory")?;
        if config.key_path.parent() != Some(cert_parent) {
            bail!("live certificate and key must share a directory");
        }
        fs::create_dir_all(cert_parent).context("failed to create certificate directory")?;
        fs::set_permissions(&staged.cert_path, fs::Permissions::from_mode(0o644))?;
        fs::set_permissions(&staged.key_path, fs::Permissions::from_mode(0o600))?;
        sync_file(&staged.cert_path)?;
        sync_file(&staged.key_path)?;
        let previous_cert = previous_path(&config.cert_path);
        let previous_key = previous_path(&config.key_path);
        let had_cert = config.cert_path.exists();
        let had_key = config.key_path.exists();
        if had_cert {
            remove_if_exists(&previous_cert)?;
            fs::rename(&config.cert_path, &previous_cert)?;
        }
        if had_key
            && let Err(error) = remove_if_exists(&previous_key)
                .and_then(|()| fs::rename(&config.key_path, &previous_key).map_err(Into::into))
        {
            if had_cert {
                fs::rename(&previous_cert, &config.cert_path)
                    .context("failed to restore live certificate")?;
            }
            return Err(error);
        }
        let promoted = (|| -> Result<()> {
            fs::rename(&staged.cert_path, &config.cert_path)?;
            fs::rename(&staged.key_path, &config.key_path)?;
            sync_directory(cert_parent)?;
            self.validate(config, &config.cert_path, &config.key_path)?;
            Ok(())
        })();
        if let Err(error) = promoted {
            restore_previous(
                &config.cert_path,
                &config.key_path,
                &previous_cert,
                &previous_key,
                had_cert,
                had_key,
            )
            .context("failed to restore previous certificate")?;
            return Err(error);
        }
        Ok(Box::new(PromotionGuard {
            cert_path: config.cert_path.clone(),
            key_path: config.key_path.clone(),
            previous_cert,
            previous_key,
            had_cert,
            had_key,
        }))
    }
}

fn san_matches(names: &[GeneralName<'_>], expected: &str) -> bool {
    if let Ok(ip) = expected.parse::<IpAddr>() {
        let expected = match ip {
            IpAddr::V4(ip) => ip.octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
        };
        names
            .iter()
            .any(|name| matches!(name, GeneralName::IPAddress(bytes) if *bytes == expected))
    } else {
        names.iter().any(
            |name| matches!(name, GeneralName::DNSName(name) if name.eq_ignore_ascii_case(expected)),
        )
    }
}

struct HandshakeVerifier;

impl ServerCertVerifier for HandshakeVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> std::result::Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }
}

fn prove_key_match(server_config: ServerConfig) -> Result<()> {
    let client_config = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(HandshakeVerifier))
        .with_no_client_auth();
    let mut client = rustls::Connection::Client(ClientConnection::new(
        Arc::new(client_config),
        ServerName::try_from("validation.invalid").expect("static server name is valid"),
    )?);
    let mut server = rustls::Connection::Server(ServerConnection::new(Arc::new(server_config))?);
    for _ in 0..8 {
        transfer_tls(&mut client, &mut server)?;
        transfer_tls(&mut server, &mut client)?;
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(());
        }
    }
    bail!("certificate and private key do not match")
}

fn transfer_tls(writer: &mut rustls::Connection, reader: &mut rustls::Connection) -> Result<()> {
    let mut bytes = Vec::new();
    while writer.wants_write() {
        writer.write_tls(&mut bytes)?;
    }
    if !bytes.is_empty() {
        reader.read_tls(&mut Cursor::new(bytes))?;
        reader
            .process_new_packets()
            .context("certificate and private key do not match")?;
    }
    Ok(())
}

struct PromotionGuard {
    cert_path: PathBuf,
    key_path: PathBuf,
    previous_cert: PathBuf,
    previous_key: PathBuf,
    had_cert: bool,
    had_key: bool,
}

#[async_trait]
impl CertificatePromotion for PromotionGuard {
    async fn commit(self: Box<Self>) -> Result<()> {
        if self.had_cert {
            remove_if_exists(&self.previous_cert)?;
        }
        if self.had_key {
            remove_if_exists(&self.previous_key)?;
        }
        sync_directory(
            self.cert_path
                .parent()
                .context("live certificate has no parent directory")?,
        )
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        restore_previous(
            &self.cert_path,
            &self.key_path,
            &self.previous_cert,
            &self.previous_key,
            self.had_cert,
            self.had_key,
        )
    }
}

fn previous_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".previous");
    value.into()
}

fn sync_file(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn restore_previous(
    cert_path: &Path,
    key_path: &Path,
    previous_cert: &Path,
    previous_key: &Path,
    had_cert: bool,
    had_key: bool,
) -> Result<()> {
    remove_if_exists(cert_path)?;
    remove_if_exists(key_path)?;
    if had_cert {
        fs::rename(previous_cert, cert_path)?;
    }
    if had_key {
        fs::rename(previous_key, key_path)?;
    }
    sync_directory(
        cert_path
            .parent()
            .context("live certificate has no parent directory")?,
    )
}

impl CertificateManager<SystemCommandRunner, SystemAcmeFirewall> {
    pub fn production() -> Result<Self> {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(Self::new(
            SystemCommandRunner,
            SystemAcmeFirewall,
            PathBuf::from(home).join(".acme.sh/acme.sh"),
            crate::core::paths::subscription::STAGING_DIR.into(),
        ))
    }
}

pub fn renew_before(mode: CertificateMode) -> Duration {
    match mode {
        CertificateMode::Domain => Duration::from_secs(30 * 24 * 3600),
        CertificateMode::Ip => Duration::from_secs(2 * 24 * 3600),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        net::{IpAddr, Ipv6Addr},
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::ExitStatus,
        sync::{Arc, Mutex},
        time::{Duration, SystemTime},
    };

    use anyhow::{Result, bail};
    use async_trait::async_trait;

    use super::{
        AcmeFirewall, CertificateManager, CommandOutput, CommandRunner, SystemCommandRunner,
        ValidatedCertificate, renew_before,
    };
    use crate::core::subscription::config::{CertificateMode, SubscriptionConfig};

    #[derive(Clone, Default)]
    struct RecordingRunner(Arc<Mutex<Vec<Vec<OsString>>>>);

    impl RecordingRunner {
        fn joined_args(&self) -> String {
            self.0
                .lock()
                .unwrap()
                .iter()
                .flat_map(|args| args.iter())
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        }

        fn clear(&self) {
            self.0.lock().unwrap().clear();
        }
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            self.0.lock().unwrap().push(args.to_vec());
            if let Some(index) = args.iter().position(|arg| arg == "--fullchain-file") {
                fs::write(PathBuf::from(&args[index + 1]), TEST_CERT)?;
            }
            if let Some(index) = args.iter().position(|arg| arg == "--key-file") {
                fs::write(PathBuf::from(&args[index + 1]), TEST_KEY)?;
            }
            Ok(success_output())
        }
    }

    #[derive(Clone)]
    struct RecordingFirewall {
        open: bool,
        actions: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingFirewall {
        fn closed() -> Self {
            Self {
                open: false,
                actions: Arc::default(),
            }
        }

        fn open() -> Self {
            Self {
                open: true,
                actions: Arc::default(),
            }
        }

        fn actions(&self) -> Vec<String> {
            self.actions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AcmeFirewall for RecordingFirewall {
        async fn is_open(&self, _port: u16) -> Result<bool> {
            Ok(self.open)
        }

        async fn open(&self, port: u16) -> Result<()> {
            self.actions.lock().unwrap().push(format!("open:{port}"));
            Ok(())
        }

        async fn close(&self, port: u16) -> Result<()> {
            self.actions.lock().unwrap().push(format!("close:{port}"));
            Ok(())
        }
    }

    struct FailingRunner;

    #[async_trait]
    impl CommandRunner for FailingRunner {
        async fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            bail!("command failed")
        }
    }

    #[tokio::test]
    async fn domain_and_ip_modes_build_distinct_letsencrypt_commands() {
        let runner = RecordingRunner::default();
        let manager = test_manager(runner.clone(), RecordingFirewall::closed());
        manager.issue(&domain_config()).await.unwrap();
        assert!(
            runner
                .joined_args()
                .contains("--server letsencrypt --issue --standalone -d sub.example.com")
        );
        assert!(!runner.joined_args().contains("shortlived"));
        assert!(runner.joined_args().contains("--keylength ec-256"));

        runner.clear();
        manager.issue(&ip_config_with_ipv6()).await.unwrap();
        let args = runner.joined_args();
        assert!(args.contains("--certificate-profile shortlived"));
        assert!(args.contains("--force"));
        assert!(args.contains("-d 203.0.113.10"));
        assert!(args.contains("-d 2001:db8::10"));
    }

    #[tokio::test]
    async fn acme_closes_only_the_port_80_rule_it_opened_even_on_failure() {
        let initially_closed = RecordingFirewall::closed();
        assert!(
            test_manager(FailingRunner, initially_closed.clone())
                .issue(&domain_config())
                .await
                .is_err()
        );
        assert_eq!(initially_closed.actions(), ["open:80", "close:80"]);
        let initially_open = RecordingFirewall::open();
        assert!(
            test_manager(FailingRunner, initially_open.clone())
                .issue(&domain_config())
                .await
                .is_err()
        );
        assert!(initially_open.actions().is_empty());
    }

    #[test]
    fn renewal_windows_match_certificate_profiles() {
        assert_eq!(
            renew_before(CertificateMode::Domain),
            Duration::from_secs(30 * 24 * 3600)
        );
        assert_eq!(
            renew_before(CertificateMode::Ip),
            Duration::from_secs(2 * 24 * 3600)
        );
    }

    #[tokio::test]
    async fn issue_validates_sans_and_protects_staged_private_key() {
        for config in [domain_config(), ip_config_with_ipv6()] {
            let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
            let certificate = manager.issue(&config).await.unwrap();
            assert!(certificate.not_after > SystemTime::now());
            assert_eq!(
                fs::metadata(certificate.key_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(certificate.cert_path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn validation_rejects_key_mismatch_and_missing_san() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        fs::write(&cert, TEST_CERT).unwrap();
        fs::write(&key, OTHER_KEY).unwrap();
        assert!(manager.validate(&domain_config(), &cert, &key).is_err());

        fs::write(&key, TEST_KEY).unwrap();
        let mut wrong_host = domain_config();
        wrong_host.public_host = "wrong.example.com".into();
        assert!(manager.validate(&wrong_host, &cert, &key).is_err());
    }

    #[tokio::test]
    async fn renew_issues_only_inside_profile_window() {
        let runner = RecordingRunner::default();
        let manager = test_manager(runner.clone(), RecordingFirewall::closed());
        let current = ValidatedCertificate {
            cert_path: "current-cert".into(),
            key_path: "current-key".into(),
            not_after: SystemTime::now() + Duration::from_secs(31 * 24 * 3600),
        };
        let unchanged = manager.renew(&domain_config(), current).await.unwrap();
        assert_eq!(unchanged.cert_path, Path::new("current-cert"));
        assert!(runner.joined_args().is_empty());

        let due = ValidatedCertificate {
            cert_path: "old-cert".into(),
            key_path: "old-key".into(),
            not_after: SystemTime::now() + Duration::from_secs(30 * 24 * 3600),
        };
        let renewed = manager.renew(&domain_config(), due).await.unwrap();
        assert!(renewed.cert_path.ends_with("fullchain.pem"));
        assert!(runner.joined_args().contains("--issue"));
    }

    #[tokio::test]
    async fn promotion_rollback_restores_live_files_and_permissions() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("key.pem");
        fs::write(&config.cert_path, b"old certificate").unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&staged_cert, TEST_CERT).unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();

        let promotion = manager
            .promote(
                &config,
                ValidatedCertificate {
                    cert_path: staged_cert,
                    key_path: staged_key,
                    not_after: SystemTime::now(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            fs::metadata(&config.cert_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
        assert_eq!(
            fs::metadata(&config.key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        promotion.rollback().await.unwrap();
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old certificate");
        assert_eq!(fs::read(&config.key_path).unwrap(), b"old key");
    }

    #[tokio::test]
    async fn command_failure_is_redacted_and_staging_is_removed() {
        let root = tempfile::tempdir().unwrap().keep();
        let acme = root.join("acme.sh");
        fs::write(&acme, "").unwrap();
        let staging = root.join("staging");
        let manager = CertificateManager::new(
            FailingRunner,
            RecordingFirewall::closed(),
            acme,
            staging.clone(),
        );
        let error = manager
            .issue(&domain_config())
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sub.example.com"));
        assert!(!staging.exists());
    }

    #[tokio::test]
    async fn process_timeout_and_output_do_not_disclose_arguments_or_output() {
        let timeout_error = SystemCommandRunner
            .run(
                Path::new("sh"),
                &["-c".into(), "sleep 1 # argument-secret".into()],
                Duration::from_millis(1),
            )
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(!timeout_error.contains("argument-secret"));

        let manager = test_manager(SensitiveOutputRunner, RecordingFirewall::closed());
        let error = manager
            .issue(&domain_config())
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("output-secret"));
        assert!(error.contains("issuance"));
    }

    #[tokio::test]
    async fn failed_promotion_restores_previous_live_pair() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("key.pem");
        fs::write(&config.cert_path, b"old certificate").unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        let staged_cert = root.path().join("staged-cert.pem");
        let staged_key = root.path().join("staged-key.pem");
        fs::write(&staged_cert, b"invalid certificate").unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        assert!(
            manager
                .promote(
                    &config,
                    ValidatedCertificate {
                        cert_path: staged_cert,
                        key_path: staged_key,
                        not_after: SystemTime::now(),
                    },
                )
                .await
                .is_err()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old certificate");
        assert_eq!(fs::read(&config.key_path).unwrap(), b"old key");
    }

    fn test_manager<R: CommandRunner, F: AcmeFirewall>(
        runner: R,
        firewall: F,
    ) -> CertificateManager<R, F> {
        let root = tempfile::tempdir().unwrap().keep();
        let acme = root.join("acme.sh");
        std::fs::write(&acme, "").unwrap();
        CertificateManager::new(runner, firewall, acme, root.join("staging"))
    }

    fn domain_config() -> SubscriptionConfig {
        let mut config = SubscriptionConfig::new_disabled("00".repeat(32));
        config.public_host = "sub.example.com".into();
        config
    }

    fn ip_config_with_ipv6() -> SubscriptionConfig {
        let mut config = domain_config();
        config.public_host = "203.0.113.10".into();
        config.certificate_mode = CertificateMode::Ip;
        config.ipv6_san = Some(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10,
        )));
        config
    }

    #[cfg(unix)]
    fn success_output() -> CommandOutput {
        use std::os::unix::process::ExitStatusExt;
        CommandOutput {
            status: ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    struct SensitiveOutputRunner;

    #[async_trait]
    impl CommandRunner for SensitiveOutputRunner {
        async fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            Ok(failure_output(b"output-secret"))
        }
    }

    #[cfg(unix)]
    fn failure_output(stderr: &[u8]) -> CommandOutput {
        use std::os::unix::process::ExitStatusExt;
        CommandOutput {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: stderr.to_vec(),
        }
    }

    const TEST_CERT: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIBfzCCATGgAwIBAgIUbSzWIvN1GUvLYtVlCecKbWD3lBkwBQYDK2VwMBoxGDAW\n",
        "BgNVBAMMD3N1Yi5leGFtcGxlLmNvbTAeFw0yNjA3MjYwNTE5MTNaFw0zNjA3MjMw\n",
        "NTE5MTNaMBoxGDAWBgNVBAMMD3N1Yi5leGFtcGxlLmNvbTAqMAUGAytlcAMhAEuM\n",
        "fEw1ifO+hN7/WIzZaSBR9O5oF9rHiqAjM2LP7DdOo4GIMIGFMB0GA1UdDgQWBBS6\n",
        "3Qla3AIor00Nv4BX6tJ6QKVj7zAfBgNVHSMEGDAWgBS63Qla3AIor00Nv4BX6tJ6\n",
        "QKVj7zAPBgNVHRMBAf8EBTADAQH/MDIGA1UdEQQrMCmCD3N1Yi5leGFtcGxlLmNv\n",
        "bYcEywBxCocQIAENuAAAAAAAAAAAAAAAEDAFBgMrZXADQQBT9seg8DXUgm+Ky+CU\n",
        "cH6LiKhDmYVu7YdxCKd4VisaSASSTni/sGwMrMfBtpgdt0PEcWGhofMpRc8BB6ZS\n",
        "I94E\n",
        "-----END CERTIFICATE-----\n"
    );
    const TEST_KEY: &str = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MC4CAQAwBQYDK2VwBCIEIOzgRc4h5rLkwez0r/5iady9q7EPnuZTrsHLYyIBGlqH\n",
        "-----END PRIVATE KEY-----\n"
    );
    const OTHER_KEY: &str = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MC4CAQAwBQYDK2VwBCIEIMpW3GLr1b1nyAJI8Eb3j8bQSvKX54MpxPVkeA53GWRs\n",
        "-----END PRIVATE KEY-----\n"
    );
}
