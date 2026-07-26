use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufReader, Cursor, Write},
    net::IpAddr,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
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

#[derive(Clone)]
struct AcmeDownload {
    status: reqwest::StatusCode,
    body: Vec<u8>,
}

#[async_trait]
trait AcmeDownloader: Send + Sync {
    async fn get(&self, url: &str) -> Result<AcmeDownload>;
}

struct HttpsAcmeDownloader;

#[async_trait]
impl AcmeDownloader for HttpsAcmeDownloader {
    async fn get(&self, url: &str) -> Result<AcmeDownload> {
        let response = reqwest::Client::new().get(url).send().await?;
        let status = response.status();
        let body = response.bytes().await?.to_vec();
        Ok(AcmeDownload { status, body })
    }
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

struct FirewallLease<F: AcmeFirewall + 'static> {
    release: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    _firewall: std::marker::PhantomData<F>,
}

impl<F: AcmeFirewall + 'static> FirewallLease<F> {
    async fn acquire(firewall: Arc<F>, port: u16) -> Result<Self> {
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            if firewall.open(port).await.is_err() {
                let _ = opened_tx.send(false);
                return Err(anyhow::anyhow!("failed to open ACME firewall rule"));
            }
            let _ = opened_tx.send(true);
            let _ = release_rx.await;
            if firewall.close(port).await.is_err() {
                log::error!("ACME firewall cleanup failed");
                return Err(anyhow::anyhow!("firewall cleanup failed"));
            }
            Ok(())
        });
        match opened_rx.await {
            Ok(true) => Ok(Self {
                release: Some(release_tx),
                task: Some(task),
                _firewall: std::marker::PhantomData,
            }),
            Ok(false) | Err(_) => {
                task.await
                    .map_err(|_| anyhow::anyhow!("ACME firewall task failed"))??;
                bail!("failed to open ACME firewall rule")
            }
        }
    }

    async fn close(mut self) -> Result<()> {
        self.release.take();
        self.task
            .take()
            .context("ACME firewall task is missing")?
            .await
            .map_err(|_| anyhow::anyhow!("ACME firewall task failed"))?
    }
}

impl<F: AcmeFirewall + 'static> Drop for FirewallLease<F> {
    fn drop(&mut self) {
        self.release.take();
    }
}

pub struct CertificateManager<R: CommandRunner, F: AcmeFirewall> {
    runner: R,
    firewall: Arc<F>,
    acme_sh: PathBuf,
    staging_dir: PathBuf,
    downloader: Arc<dyn AcmeDownloader>,
    #[cfg(test)]
    after_certificate_backup: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl<R: CommandRunner, F: AcmeFirewall + 'static> CertificateManager<R, F> {
    pub fn new(runner: R, firewall: F, acme_sh: PathBuf, staging_dir: PathBuf) -> Self {
        Self {
            runner,
            firewall: Arc::new(firewall),
            acme_sh,
            staging_dir,
            downloader: Arc::new(HttpsAcmeDownloader),
            #[cfg(test)]
            after_certificate_backup: None,
        }
    }

    #[cfg(test)]
    fn with_downloader(mut self, downloader: Arc<dyn AcmeDownloader>) -> Self {
        self.downloader = downloader;
        self
    }

    #[cfg(test)]
    fn with_after_certificate_backup(mut self, hook: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.after_certificate_backup = Some(hook);
        self
    }

    pub async fn issue(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        match self.issue_inner(config).await {
            Ok(certificate) => Ok(certificate),
            Err(operation) => match remove_real_directory(&self.staging_dir) {
                Ok(()) => Err(operation),
                Err(_) => Err(anyhow::anyhow!("{operation}; staging cleanup failed")),
            },
        }
    }

    async fn issue_inner(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        self.ensure_acme_sh().await?;
        let staging_parent = self
            .staging_dir
            .parent()
            .context("staging directory has no parent")?;
        ensure_or_create_real_directory(staging_parent, "staging parent directory")?;
        remove_real_directory(&self.staging_dir).context("failed to reset certificate staging")?;
        fs::create_dir(&self.staging_dir).context("failed to create certificate staging")?;
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
        let lease = if already_open {
            None
        } else {
            Some(FirewallLease::acquire(self.firewall.clone(), 80).await?)
        };
        let issued = self
            .run_checked(&self.acme_sh, &issue_args, ISSUE_TIMEOUT, "issuance")
            .await;
        let cleaned = match lease {
            Some(lease) => lease.close().await,
            None => Ok(()),
        };
        match (issued, cleaned) {
            (Ok(()), Ok(())) => {}
            (Err(issue), Ok(())) => return Err(issue),
            (Ok(()), Err(cleanup)) => return Err(cleanup),
            (Err(issue), Err(_)) => bail!("{issue}; firewall cleanup failed"),
        }

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
        let validated = self.validate(config, &cert_path, &key_path)?;
        fs::set_permissions(&cert_path, fs::Permissions::from_mode(0o644))?;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        Ok(validated)
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
        let response = self
            .downloader
            .get("https://get.acme.sh")
            .await
            .map_err(|_| anyhow::anyhow!("failed to download acme.sh installer"))?;
        if !response.status.is_success() {
            bail!("failed to download acme.sh installer");
        }
        self.install_acme_sh(&response.body).await
    }

    async fn install_acme_sh(&self, body: &[u8]) -> Result<()> {
        let parent = self.acme_sh.parent().context("invalid acme.sh path")?;
        let installer = parent.join(format!(".installer-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(&installer)
            .context("failed to create acme.sh installer")?;
        file.write_all(body)?;
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
        ensure_regular_file(cert_path, "certificate")?;
        ensure_regular_file(key_path, "private key")?;
        let cert_raw = fs::read(cert_path).context("failed to read certificate chain")?;
        let certs = rustls_pemfile::certs(&mut BufReader::new(cert_raw.as_slice()))
            .context("failed to parse certificate chain")?;
        if certs.is_empty() {
            bail!("certificate chain is empty");
        }
        let key_raw = fs::read(key_path).context("failed to read private key")?;
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut BufReader::new(key_raw.as_slice()))
            .context("failed to parse private key")?;
        keys.extend(
            rustls_pemfile::rsa_private_keys(&mut BufReader::new(key_raw.as_slice()))
                .context("failed to parse RSA private key")?,
        );
        keys.extend(
            rustls_pemfile::ec_private_keys(&mut BufReader::new(key_raw.as_slice()))
                .context("failed to parse EC private key")?,
        );
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
        self.validate(config, &staged.cert_path, &staged.key_path)?;
        self.check_promotion_paths(config, &staged)?;
        let cert_parent = config
            .cert_path
            .parent()
            .context("live certificate has no parent directory")?;
        if config.key_path.parent() != Some(cert_parent) {
            bail!("live certificate and key must share a directory");
        }
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
        #[cfg(test)]
        if let Some(hook) = &self.after_certificate_backup {
            hook();
        }
        if had_key
            && let Err(error) = remove_if_exists(&previous_key)
                .and_then(|()| fs::rename(&config.key_path, &previous_key).map_err(Into::into))
        {
            if had_cert {
                fs::rename(&previous_cert, &config.cert_path)
                    .context("failed to restore live certificate")?;
                sync_directory(cert_parent)?;
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
        let identities = (|| -> Result<_> {
            Ok((
                file_identity(&config.cert_path, "live certificate")?,
                file_identity(&config.key_path, "live private key")?,
                had_cert
                    .then(|| file_identity(&previous_cert, "certificate backup"))
                    .transpose()?,
                had_key
                    .then(|| file_identity(&previous_key, "private key backup"))
                    .transpose()?,
            ))
        })();
        let (cert_identity, key_identity, previous_cert_identity, previous_key_identity) =
            match identities {
                Ok(identities) => identities,
                Err(error) => {
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
            };
        Ok(Box::new(PromotionGuard {
            cert_path: config.cert_path.clone(),
            key_path: config.key_path.clone(),
            previous_cert,
            previous_key,
            had_cert,
            had_key,
            cert_identity,
            key_identity,
            previous_cert_identity,
            previous_key_identity,
        }))
    }

    fn check_promotion_paths(
        &self,
        config: &SubscriptionConfig,
        staged: &ValidatedCertificate,
    ) -> Result<()> {
        let cert_parent = config
            .cert_path
            .parent()
            .context("live certificate has no parent directory")?;
        if config.key_path.parent() != Some(cert_parent) {
            bail!("live certificate and key must share a directory");
        }
        if self.staging_dir.parent() != Some(cert_parent)
            || staged.cert_path.parent() != Some(self.staging_dir.as_path())
            || staged.key_path.parent() != Some(self.staging_dir.as_path())
        {
            bail!("certificate paths escape the configured certificate directory");
        }
        ensure_real_directory(cert_parent, "certificate directory")?;
        ensure_real_directory(&self.staging_dir, "staging directory")?;

        let previous_cert = previous_path(&config.cert_path);
        let previous_key = previous_path(&config.key_path);
        let paths = [
            &config.cert_path,
            &config.key_path,
            &previous_cert,
            &previous_key,
            &staged.cert_path,
            &staged.key_path,
        ];
        let mut identities = Vec::new();
        let mut present = [false; 6];
        for (index, path) in paths.iter().enumerate() {
            if paths[..index].contains(path) {
                bail!("certificate paths collide");
            }
            if let Some(metadata) = inspect_regular_file(path, "certificate path")? {
                present[index] = true;
                let identity = FileIdentity::from(&metadata);
                if identities.contains(&identity) {
                    bail!("certificate paths alias the same file");
                }
                identities.push(identity);
            }
        }
        if (!present[0] && present[2]) || (!present[1] && present[3]) {
            bail!("orphaned certificate backup path");
        }
        Ok(())
    }
}

fn ensure_regular_file(path: &Path, category: &str) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {category}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!("{category} must be a regular file");
    }
    Ok(())
}

fn inspect_regular_file(path: &Path, category: &str) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => bail!("{category} must be a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {category}")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl From<&fs::Metadata> for FileIdentity {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

fn ensure_real_directory(path: &Path, category: &str) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to inspect {category}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        bail!("{category} must be a real directory");
    }
    if fs::canonicalize(path)? != path {
        bail!("{category} must not contain aliases or symlinks");
    }
    Ok(())
}

fn ensure_or_create_real_directory(path: &Path, category: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => ensure_real_directory(path, category),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("directory has no parent")?;
            ensure_or_create_real_directory(parent, category)?;
            match fs::create_dir(path) {
                Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            ensure_real_directory(path, category)
        }
        Err(error) => Err(error.into()),
    }
}

fn remove_real_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            if fs::canonicalize(path)? != path {
                bail!("staging directory must not contain aliases or symlinks");
            }
            fs::remove_dir_all(path)?;
            Ok(())
        }
        Ok(_) => bail!("staging directory must be a real directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
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
    cert_identity: FileIdentity,
    key_identity: FileIdentity,
    previous_cert_identity: Option<FileIdentity>,
    previous_key_identity: Option<FileIdentity>,
}

#[async_trait]
impl CertificatePromotion for PromotionGuard {
    async fn commit(self: Box<Self>) -> Result<()> {
        self.verify_paths()?;
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
        self.verify_paths()?;
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

impl PromotionGuard {
    fn verify_paths(&self) -> Result<()> {
        verify_identity(
            &self.cert_path,
            Some(self.cert_identity),
            "live certificate",
        )?;
        verify_identity(&self.key_path, Some(self.key_identity), "live private key")?;
        verify_identity(
            &self.previous_cert,
            self.previous_cert_identity,
            "certificate backup",
        )?;
        verify_identity(
            &self.previous_key,
            self.previous_key_identity,
            "private key backup",
        )
    }
}

fn file_identity(path: &Path, category: &str) -> Result<FileIdentity> {
    inspect_regular_file(path, category)?
        .map(|metadata| FileIdentity::from(&metadata))
        .with_context(|| format!("{category} is missing"))
}

fn verify_identity(path: &Path, expected: Option<FileIdentity>, category: &str) -> Result<()> {
    let actual = inspect_regular_file(path, category)?
        .as_ref()
        .map(FileIdentity::from);
    if actual != expected {
        bail!("{category} changed during certificate promotion");
    }
    Ok(())
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
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, SystemTime},
    };

    use anyhow::{Result, bail};
    use async_trait::async_trait;
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{
        AcmeDownload, AcmeDownloader, AcmeFirewall, CertificateManager, CommandOutput,
        CommandRunner, SystemCommandRunner, ValidatedCertificate, renew_before,
    };
    use crate::core::subscription::config::{CertificateMode, SubscriptionConfig};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedCommand {
        program: PathBuf,
        args: Vec<OsString>,
        timeout: Duration,
    }

    #[derive(Clone, Default)]
    struct RecordingRunner(Arc<Mutex<Vec<RecordedCommand>>>);

    impl RecordingRunner {
        fn joined_args(&self) -> String {
            self.0
                .lock()
                .unwrap()
                .iter()
                .flat_map(|call| call.args.iter())
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        }

        fn clear(&self) {
            self.0.lock().unwrap().clear();
        }

        fn calls(&self) -> Vec<RecordedCommand> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(
            &self,
            program: &Path,
            args: &[OsString],
            timeout: Duration,
        ) -> Result<CommandOutput> {
            self.0.lock().unwrap().push(RecordedCommand {
                program: program.to_owned(),
                args: args.to_vec(),
                timeout,
            });
            if let Some(index) = args.iter().position(|arg| arg == "--fullchain-file") {
                let path = PathBuf::from(&args[index + 1]);
                fs::write(&path, TEST_CERT)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o666))?;
            }
            if let Some(index) = args.iter().position(|arg| arg == "--key-file") {
                let path = PathBuf::from(&args[index + 1]);
                fs::write(&path, TEST_KEY)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o666))?;
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

    #[derive(Clone, Default)]
    struct FailingCloseFirewall(Arc<Mutex<Vec<String>>>);

    impl FailingCloseFirewall {
        fn actions(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AcmeFirewall for FailingCloseFirewall {
        async fn is_open(&self, _port: u16) -> Result<bool> {
            Ok(false)
        }

        async fn open(&self, port: u16) -> Result<()> {
            self.0.lock().unwrap().push(format!("open:{port}"));
            Ok(())
        }

        async fn close(&self, port: u16) -> Result<()> {
            self.0.lock().unwrap().push(format!("close:{port}"));
            bail!("cleanup-secret")
        }
    }

    #[derive(Clone, Default)]
    struct CancellationFirewall {
        actions: Arc<Mutex<Vec<String>>>,
        opened: Arc<tokio::sync::Notify>,
    }

    impl CancellationFirewall {
        fn actions(&self) -> Vec<String> {
            self.actions.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AcmeFirewall for CancellationFirewall {
        async fn is_open(&self, _port: u16) -> Result<bool> {
            Ok(false)
        }

        async fn open(&self, port: u16) -> Result<()> {
            self.actions.lock().unwrap().push(format!("open:{port}"));
            self.opened.notify_one();
            Ok(())
        }

        async fn close(&self, port: u16) -> Result<()> {
            self.actions.lock().unwrap().push(format!("close:{port}"));
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct CancellationDuringOpenFirewall {
        actions: Arc<Mutex<Vec<String>>>,
        opened: Arc<tokio::sync::Notify>,
        complete: Arc<tokio::sync::Notify>,
        finished: Arc<tokio::sync::Notify>,
        succeeds: Arc<AtomicBool>,
    }

    impl CancellationDuringOpenFirewall {
        fn actions(&self) -> Vec<String> {
            self.actions.lock().unwrap().clone()
        }

        fn complete(&self, succeeds: bool) {
            self.succeeds.store(succeeds, Ordering::SeqCst);
            self.complete.notify_one();
        }
    }

    #[async_trait]
    impl AcmeFirewall for CancellationDuringOpenFirewall {
        async fn is_open(&self, _port: u16) -> Result<bool> {
            Ok(false)
        }

        async fn open(&self, port: u16) -> Result<()> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("open-pending:{port}"));
            self.opened.notify_one();
            self.complete.notified().await;
            let result = if self.succeeds.load(Ordering::SeqCst) {
                self.actions.lock().unwrap().push(format!("open:{port}"));
                Ok(())
            } else {
                Err(anyhow::anyhow!("open failed"))
            };
            self.finished.notify_one();
            result
        }

        async fn close(&self, port: u16) -> Result<()> {
            self.actions.lock().unwrap().push(format!("close:{port}"));
            Ok(())
        }
    }

    struct PendingRunner;

    #[async_trait]
    impl CommandRunner for PendingRunner {
        async fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            std::future::pending().await
        }
    }

    #[derive(Clone)]
    struct InstallerRunner {
        acme_path: PathBuf,
        call: Arc<Mutex<Option<RecordedCommand>>>,
    }

    #[async_trait]
    impl CommandRunner for InstallerRunner {
        async fn run(
            &self,
            program: &Path,
            args: &[OsString],
            timeout: Duration,
        ) -> Result<CommandOutput> {
            let installer = PathBuf::from(&args[0]);
            assert_eq!(
                fs::metadata(&installer)?.permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(fs::read(&installer)?, b"#!/bin/sh\nexit 0\n");
            *self.call.lock().unwrap() = Some(RecordedCommand {
                program: program.to_owned(),
                args: args.to_vec(),
                timeout,
            });
            fs::write(&self.acme_path, "installed")?;
            Ok(success_output())
        }
    }

    #[derive(Clone)]
    struct RecordingDownloader {
        outcome: DownloadOutcome,
        urls: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Clone)]
    enum DownloadOutcome {
        Response(AcmeDownload),
        Failure(&'static str),
    }

    impl RecordingDownloader {
        fn response(status: reqwest::StatusCode, body: &[u8]) -> Self {
            Self {
                outcome: DownloadOutcome::Response(AcmeDownload {
                    status,
                    body: body.to_vec(),
                }),
                urls: Arc::default(),
            }
        }

        fn failure(message: &'static str) -> Self {
            Self {
                outcome: DownloadOutcome::Failure(message),
                urls: Arc::default(),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AcmeDownloader for RecordingDownloader {
        async fn get(&self, url: &str) -> Result<AcmeDownload> {
            self.urls.lock().unwrap().push(url.to_owned());
            match &self.outcome {
                DownloadOutcome::Response(response) => Ok(response.clone()),
                DownloadOutcome::Failure(message) => bail!(*message),
            }
        }
    }

    struct SymlinkOutputRunner {
        cert_target: PathBuf,
        key_target: PathBuf,
    }

    #[async_trait]
    impl CommandRunner for SymlinkOutputRunner {
        async fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            use std::os::unix::fs::symlink;

            if let Some(index) = args.iter().position(|arg| arg == "--fullchain-file") {
                symlink(&self.cert_target, PathBuf::from(&args[index + 1]))?;
            }
            if let Some(index) = args.iter().position(|arg| arg == "--key-file") {
                fs::remove_file(PathBuf::from(&args[index + 1]))?;
                symlink(&self.key_target, PathBuf::from(&args[index + 1]))?;
            }
            Ok(success_output())
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
    async fn acme_commands_preserve_exact_program_argument_order_and_timeouts() {
        let runner = RecordingRunner::default();
        let manager = test_manager(runner.clone(), RecordingFirewall::closed());
        let certificate = manager.issue(&domain_config()).await.unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].program.file_name().unwrap(), "acme.sh");
        assert_eq!(
            calls[0].args,
            os_args(&[
                "--server",
                "letsencrypt",
                "--issue",
                "--standalone",
                "-d",
                "sub.example.com",
                "--keylength",
                "ec-256",
            ])
        );
        assert_eq!(calls[0].timeout, Duration::from_secs(300));
        assert_eq!(calls[1].program, calls[0].program);
        assert_eq!(
            calls[1].args,
            vec![
                "--install-cert".into(),
                "--ecc".into(),
                "-d".into(),
                "sub.example.com".into(),
                "--fullchain-file".into(),
                certificate.cert_path.as_os_str().to_owned(),
                "--key-file".into(),
                certificate.key_path.as_os_str().to_owned(),
            ]
        );
        assert_eq!(calls[1].timeout, Duration::from_secs(300));

        runner.clear();
        let ip_certificate = manager.issue(&ip_config_with_ipv6()).await.unwrap();
        let calls = runner.calls();
        assert_eq!(
            calls[0].args,
            os_args(&[
                "--server",
                "letsencrypt",
                "--issue",
                "--standalone",
                "--certificate-profile",
                "shortlived",
                "--force",
                "-d",
                "203.0.113.10",
                "--keylength",
                "ec-256",
                "-d",
                "2001:db8::10",
            ])
        );
        assert_eq!(calls[0].timeout, Duration::from_secs(300));
        assert_eq!(calls[1].program, calls[0].program);
        assert_eq!(
            calls[1].args,
            vec![
                "--install-cert".into(),
                "--ecc".into(),
                "-d".into(),
                "203.0.113.10".into(),
                "--fullchain-file".into(),
                ip_certificate.cert_path.as_os_str().to_owned(),
                "--key-file".into(),
                ip_certificate.key_path.as_os_str().to_owned(),
            ]
        );
        assert_eq!(calls[1].timeout, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn installer_uses_direct_sh_argument_and_bounded_timeout() {
        let root = tempfile::tempdir().unwrap();
        let acme = root.path().join("acme.sh");
        let runner = InstallerRunner {
            acme_path: acme.clone(),
            call: Arc::default(),
        };
        let manager = CertificateManager::new(
            runner.clone(),
            RecordingFirewall::closed(),
            acme,
            root.path().join("staging"),
        );
        manager
            .install_acme_sh(b"#!/bin/sh\nexit 0\n")
            .await
            .unwrap();
        let call = runner.call.lock().unwrap().clone().unwrap();
        assert_eq!(call.program, Path::new("sh"));
        assert_eq!(call.args.len(), 1);
        assert_ne!(call.args[0], "-c");
        assert_eq!(call.timeout, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn missing_acme_downloads_exact_https_url_and_installs_response_body() {
        let root = tempfile::tempdir().unwrap();
        let acme = root.path().join("acme.sh");
        let runner = InstallerRunner {
            acme_path: acme.clone(),
            call: Arc::default(),
        };
        let downloader =
            RecordingDownloader::response(reqwest::StatusCode::OK, b"#!/bin/sh\nexit 0\n");
        let manager = CertificateManager::new(
            runner.clone(),
            RecordingFirewall::closed(),
            acme,
            root.path().join("staging"),
        )
        .with_downloader(Arc::new(downloader.clone()));
        manager.ensure_acme_sh().await.unwrap();
        assert_eq!(downloader.urls(), ["https://get.acme.sh"]);
        assert!(runner.call.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn missing_acme_rejects_http_status_and_redacts_download_failure() {
        for downloader in [
            RecordingDownloader::response(
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                b"status-body-secret",
            ),
            RecordingDownloader::failure("download-error-secret"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let runner = RecordingRunner::default();
            let manager = CertificateManager::new(
                runner.clone(),
                RecordingFirewall::closed(),
                root.path().join("acme.sh"),
                root.path().join("staging"),
            )
            .with_downloader(Arc::new(downloader.clone()));
            let error = manager.ensure_acme_sh().await.unwrap_err().to_string();
            assert!(error.contains("download acme.sh installer"));
            assert!(!error.contains("secret"));
            assert!(runner.calls().is_empty());
            assert_eq!(downloader.urls(), ["https://get.acme.sh"]);
        }
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

    #[tokio::test]
    async fn issuance_and_firewall_cleanup_failures_are_both_reported_safely() {
        let firewall = FailingCloseFirewall::default();
        let error = test_manager(FailingRunner, firewall.clone())
            .issue(&domain_config())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("issuance"));
        assert!(error.contains("firewall cleanup"));
        assert!(!error.contains("cleanup-secret"));
        assert_eq!(firewall.actions(), ["open:80", "close:80"]);
    }

    #[tokio::test]
    async fn cancellation_still_closes_port_80_rule_owned_by_issue() {
        let firewall = CancellationFirewall::default();
        let manager = test_manager(PendingRunner, firewall.clone());
        let task = tokio::spawn(async move { manager.issue(&domain_config()).await });
        firewall.opened.notified().await;
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if firewall.actions().contains(&"close:80".to_owned()) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(firewall.actions(), ["open:80", "close:80"]);
    }

    #[tokio::test]
    async fn cancellation_during_firewall_open_closes_only_after_success() {
        let firewall = CancellationDuringOpenFirewall::default();
        let manager = test_manager(PendingRunner, firewall.clone());
        let task = tokio::spawn(async move { manager.issue(&domain_config()).await });
        firewall.opened.notified().await;
        task.abort();
        let _ = task.await;
        assert_eq!(firewall.actions(), ["open-pending:80"]);
        firewall.complete(true);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if firewall.actions().contains(&"close:80".to_owned()) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            firewall.actions(),
            ["open-pending:80", "open:80", "close:80"]
        );
    }

    #[tokio::test]
    async fn cancellation_during_failed_firewall_open_never_closes_rule() {
        let firewall = CancellationDuringOpenFirewall::default();
        let manager = test_manager(PendingRunner, firewall.clone());
        let task = tokio::spawn(async move { manager.issue(&domain_config()).await });
        firewall.opened.notified().await;
        task.abort();
        let _ = task.await;
        firewall.complete(false);
        tokio::time::timeout(Duration::from_secs(1), firewall.finished.notified())
            .await
            .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(firewall.actions(), ["open-pending:80"]);
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
                fs::metadata(&certificate.cert_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
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

    #[tokio::test]
    async fn issue_rejects_output_symlinks_before_changing_target_modes() {
        let root = tempfile::tempdir().unwrap();
        let cert_target = root.path().join("cert-target.pem");
        let key_target = root.path().join("key-target.pem");
        fs::write(&cert_target, TEST_CERT).unwrap();
        fs::write(&key_target, TEST_KEY).unwrap();
        fs::set_permissions(&cert_target, fs::Permissions::from_mode(0o666)).unwrap();
        fs::set_permissions(&key_target, fs::Permissions::from_mode(0o666)).unwrap();
        let runner = SymlinkOutputRunner {
            cert_target: cert_target.clone(),
            key_target: key_target.clone(),
        };
        assert!(
            test_manager(runner, RecordingFirewall::closed())
                .issue(&domain_config())
                .await
                .is_err()
        );
        assert_eq!(
            fs::metadata(cert_target).unwrap().permissions().mode() & 0o777,
            0o666
        );
        assert_eq!(
            fs::metadata(key_target).unwrap().permissions().mode() & 0o777,
            0o666
        );
    }

    #[tokio::test]
    async fn issue_rejects_staging_directory_symlink_without_removing_it() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("marker"), b"untouched").unwrap();
        let staging = root.path().join("staging");
        symlink(&target, &staging).unwrap();
        let acme = root.path().join("acme.sh");
        fs::write(&acme, "").unwrap();
        let manager = CertificateManager::new(
            RecordingRunner::default(),
            RecordingFirewall::closed(),
            acme,
            staging.clone(),
        );
        assert!(manager.issue(&domain_config()).await.is_err());
        assert!(
            fs::symlink_metadata(staging)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"untouched");
    }

    #[tokio::test]
    async fn issue_securely_creates_missing_staging_parents() {
        let root = tempfile::tempdir().unwrap();
        let acme = root.path().join("acme.sh");
        fs::write(&acme, "").unwrap();
        let staging = root.path().join("certs/staging");
        let manager = CertificateManager::new(
            RecordingRunner::default(),
            RecordingFirewall::closed(),
            acme,
            staging.clone(),
        );
        let certificate = manager.issue(&domain_config()).await.unwrap();
        assert_eq!(certificate.cert_path.parent(), Some(staging.as_path()));
        assert!(root.path().join("certs").is_dir());
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

    #[test]
    fn validation_accepts_acme_sec1_ec_private_key() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        fs::write(&cert, TEST_CERT).unwrap();
        fs::write(&key, TEST_KEY).unwrap();
        manager.validate(&domain_config(), &cert, &key).unwrap();
    }

    #[test]
    fn validation_counts_all_private_key_encodings_together() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        fs::write(&cert, TEST_CERT).unwrap();

        for keys in [
            format!("{TEST_KEY}{PKCS8_KEY}"),
            format!("{TEST_KEY}{RSA_KEY}"),
            format!("{TEST_KEY}{TEST_KEY}"),
            format!("{PKCS8_KEY}{PKCS8_KEY}"),
            format!("{RSA_KEY}{RSA_KEY}"),
            format!("{PKCS8_KEY}{RSA_KEY}"),
        ] {
            fs::write(&key, keys).unwrap();
            let error = manager
                .validate(&domain_config(), &cert, &key)
                .unwrap_err()
                .to_string();
            assert!(error.contains("exactly one private key"));
        }
    }

    #[test]
    fn validation_rejects_empty_certificate_chain() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        fs::write(&cert, "").unwrap();
        fs::write(&key, TEST_KEY).unwrap();
        assert!(manager.validate(&domain_config(), &cert, &key).is_err());
    }

    #[test]
    fn validation_rejects_expired_and_not_yet_valid_certificates() {
        let manager = test_manager(RecordingRunner::default(), RecordingFirewall::closed());
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("cert.pem");
        let key = root.path().join("key.pem");
        fs::write(&key, TEST_KEY).unwrap();

        fs::write(
            &cert,
            certificate_with_validity(b"000101000000Z", b"010101000000Z"),
        )
        .unwrap();
        assert!(manager.validate(&domain_config(), &cert, &key).is_err());
        fs::write(
            &cert,
            certificate_with_validity(b"350101000000Z", b"360101000000Z"),
        )
        .unwrap();
        assert!(manager.validate(&domain_config(), &cert, &key).is_err());
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
        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
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
    async fn promotion_commit_removes_backups_and_keeps_valid_live_pair() {
        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
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
            .promote(&config, validated(staged_cert, staged_key))
            .await
            .unwrap();
        assert!(root.path().join("fullchain.pem.previous").exists());
        assert!(root.path().join("key.pem.previous").exists());
        promotion.commit().await.unwrap();
        assert!(!root.path().join("fullchain.pem.previous").exists());
        assert!(!root.path().join("key.pem.previous").exists());
        manager
            .validate(&config, &config.cert_path, &config.key_path)
            .unwrap();
    }

    #[tokio::test]
    async fn rollback_rejects_replaced_backup_before_touching_live_files() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
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
            .promote(&config, validated(staged_cert, staged_key))
            .await
            .unwrap();
        let previous_cert = root.path().join("fullchain.pem.previous");
        fs::remove_file(&previous_cert).unwrap();
        symlink(root.path().join("missing-target"), &previous_cert).unwrap();
        assert!(promotion.rollback().await.is_err());
        assert!(config.cert_path.is_file());
        assert!(config.key_path.is_file());
        manager
            .validate(&config, &config.cert_path, &config.key_path)
            .unwrap();
        assert!(
            fs::symlink_metadata(previous_cert)
                .unwrap()
                .file_type()
                .is_symlink()
        );
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
    async fn issuance_and_staging_cleanup_failures_are_combined_and_redacted() {
        let root = tempfile::tempdir().unwrap();
        let acme = root.path().join("acme.sh");
        fs::write(&acme, "").unwrap();
        let staging = root.path().join("certs/staging");
        let staging_parent = staging.parent().unwrap().to_owned();
        fs::create_dir(&staging_parent).unwrap();
        let manager = CertificateManager::new(
            CleanupFailingRunner {
                staging_parent: staging_parent.clone(),
            },
            RecordingFirewall::closed(),
            acme,
            staging.clone(),
        );
        let error = manager
            .issue(&domain_config())
            .await
            .unwrap_err()
            .to_string();
        fs::set_permissions(&staging_parent, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(error.contains("issuance"));
        assert!(error.contains("staging cleanup failed"));
        assert!(!error.contains("process-output-secret"));
        assert!(!error.contains(&staging.display().to_string()));
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
        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("key.pem");
        fs::write(&config.cert_path, b"old certificate").unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&staged_cert, b"invalid certificate").unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        assert!(
            manager
                .promote(
                    &config,
                    ValidatedCertificate {
                        cert_path: staged_cert.clone(),
                        key_path: staged_key.clone(),
                        not_after: SystemTime::now(),
                    },
                )
                .await
                .is_err()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old certificate");
        assert_eq!(fs::read(&config.key_path).unwrap(), b"old key");
        assert!(staged_cert.exists());
        assert!(staged_key.exists());
        assert!(!root.path().join("fullchain.pem.previous").exists());
        assert!(!root.path().join("key.pem.previous").exists());
    }

    #[tokio::test]
    async fn key_backup_failure_restores_and_syncs_certificate_directory() {
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

        let key_path = config.key_path.clone();
        let cert_parent = root.path().to_owned();
        let manager =
            promotion_manager(root.path()).with_after_certificate_backup(Arc::new(move || {
                fs::remove_file(&key_path).unwrap();
                fs::set_permissions(&cert_parent, fs::Permissions::from_mode(0o300)).unwrap();
            }));
        let error = manager
            .promote(&config, validated(staged_cert.clone(), staged_key.clone()))
            .await
            .err()
            .unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old certificate");
        assert!(!root.path().join("fullchain.pem.previous").exists());
        assert!(staged_cert.exists());
        assert!(staged_key.exists());
    }

    #[tokio::test]
    async fn promotion_rejects_path_collisions_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&staged_cert, TEST_CERT).unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        let shared = root.path().join("shared.pem");
        fs::write(&shared, b"untouched").unwrap();
        let mut config = domain_config();
        config.cert_path = shared.clone();
        config.key_path = shared.clone();
        let result = manager
            .promote(&config, validated(staged_cert.clone(), staged_key.clone()))
            .await;
        assert!(result.is_err());
        assert_eq!(fs::read(&shared).unwrap(), b"untouched");
        assert!(staged_cert.exists());
        assert!(staged_key.exists());

        config.cert_path = root.path().join("key.pem.previous");
        config.key_path = root.path().join("key.pem");
        fs::write(&config.cert_path, b"cert").unwrap();
        fs::write(&config.key_path, b"key").unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert, staged_key))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"cert");
        assert_eq!(fs::read(&config.key_path).unwrap(), b"key");
    }

    #[tokio::test]
    async fn promotion_rejects_symlinks_and_outside_staging() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let real_cert = staging.join("real-cert.pem");
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&real_cert, TEST_CERT).unwrap();
        symlink(&real_cert, &staged_cert).unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("live-key.pem");
        fs::write(&config.cert_path, b"old cert").unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert.clone(), staged_key.clone()))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old cert");
        assert!(
            fs::symlink_metadata(&staged_cert)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        fs::remove_file(&staged_cert).unwrap();
        let outside = root.path().join("outside-cert.pem");
        fs::write(&outside, TEST_CERT).unwrap();
        assert!(
            manager
                .promote(&config, validated(outside.clone(), staged_key))
                .await
                .is_err()
        );
        assert!(outside.exists());
    }

    #[tokio::test]
    async fn promotion_rejects_live_and_backup_symlinks_and_hardlink_aliases() {
        use std::{fs::hard_link, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&staged_cert, TEST_CERT).unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        let target = root.path().join("target.pem");
        fs::write(&target, b"old certificate").unwrap();
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("live-key.pem");
        symlink(&target, &config.cert_path).unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert.clone(), staged_key.clone()))
                .await
                .is_err()
        );
        assert!(
            fs::symlink_metadata(&config.cert_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        fs::remove_file(&config.cert_path).unwrap();
        fs::write(&config.cert_path, b"old certificate").unwrap();
        let previous_cert = root.path().join("fullchain.pem.previous");
        symlink(&target, &previous_cert).unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert.clone(), staged_key.clone()))
                .await
                .is_err()
        );
        assert!(
            fs::symlink_metadata(&previous_cert)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        fs::remove_file(&previous_cert).unwrap();
        fs::remove_file(&config.cert_path).unwrap();
        fs::write(&config.cert_path, b"shared old file").unwrap();
        fs::remove_file(&config.key_path).unwrap();
        hard_link(&config.cert_path, &config.key_path).unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert, staged_key))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"shared old file");
    }

    #[tokio::test]
    async fn promotion_rejects_broken_backup_symlink_without_removing_it() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let manager = promotion_manager(root.path());
        let staging = root.path().join("staging");
        fs::create_dir(&staging).unwrap();
        let staged_cert = staging.join("fullchain.pem");
        let staged_key = staging.join("key.pem");
        fs::write(&staged_cert, TEST_CERT).unwrap();
        fs::write(&staged_key, TEST_KEY).unwrap();
        let mut config = domain_config();
        config.cert_path = root.path().join("fullchain.pem");
        config.key_path = root.path().join("key.pem");
        fs::write(&config.cert_path, b"old cert").unwrap();
        fs::write(&config.key_path, b"old key").unwrap();
        let previous = root.path().join("fullchain.pem.previous");
        symlink(root.path().join("missing"), &previous).unwrap();
        assert!(
            manager
                .promote(&config, validated(staged_cert, staged_key))
                .await
                .is_err()
        );
        assert!(
            fs::symlink_metadata(previous)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&config.cert_path).unwrap(), b"old cert");
    }

    fn test_manager<R: CommandRunner, F: AcmeFirewall + 'static>(
        runner: R,
        firewall: F,
    ) -> CertificateManager<R, F> {
        let root = tempfile::tempdir().unwrap().keep();
        let acme = root.join("acme.sh");
        std::fs::write(&acme, "").unwrap();
        CertificateManager::new(runner, firewall, acme, root.join("staging"))
    }

    fn promotion_manager(root: &Path) -> CertificateManager<RecordingRunner, RecordingFirewall> {
        let acme = root.join("acme.sh");
        fs::write(&acme, "").unwrap();
        CertificateManager::new(
            RecordingRunner::default(),
            RecordingFirewall::closed(),
            acme,
            root.join("staging"),
        )
    }

    fn validated(cert_path: PathBuf, key_path: PathBuf) -> ValidatedCertificate {
        ValidatedCertificate {
            cert_path,
            key_path,
            not_after: SystemTime::now() + Duration::from_secs(3600),
        }
    }

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn certificate_with_validity(not_before: &[u8; 13], not_after: &[u8; 13]) -> String {
        let mut certs =
            rustls_pemfile::certs(&mut std::io::BufReader::new(TEST_CERT.as_bytes())).unwrap();
        let mut der = certs.remove(0);
        replace_bytes(&mut der, b"260726054253Z", not_before);
        replace_bytes(&mut der, b"360723054253Z", not_after);
        let encoded = STANDARD.encode(der);
        let body = encoded
            .as_bytes()
            .chunks(64)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n")
    }

    fn replace_bytes(source: &mut [u8], from: &[u8], to: &[u8]) {
        let index = source
            .windows(from.len())
            .position(|window| window == from)
            .unwrap();
        source[index..index + from.len()].copy_from_slice(to);
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

    struct CleanupFailingRunner {
        staging_parent: PathBuf,
    }

    #[async_trait]
    impl CommandRunner for CleanupFailingRunner {
        async fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            fs::set_permissions(&self.staging_parent, fs::Permissions::from_mode(0o500))?;
            bail!("process-output-secret")
        }
    }

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
        "MIIBwDCCAWWgAwIBAgIUZYiat7hpwcskbsobyaEycN3FI0YwCgYIKoZIzj0EAwIw\n",
        "GjEYMBYGA1UEAwwPc3ViLmV4YW1wbGUuY29tMB4XDTI2MDcyNjA1NDI1M1oXDTM2\n",
        "MDcyMzA1NDI1M1owGjEYMBYGA1UEAwwPc3ViLmV4YW1wbGUuY29tMFkwEwYHKoZI\n",
        "zj0CAQYIKoZIzj0DAQcDQgAEj2AOhpo4uXlnE7zCh4fhJJeM8E/KJG2mnyryCNeU\n",
        "qGBzBpbOkQm0gZcJO9V54nQkpxgN3RncWINJUMJ7GFNQzaOBiDCBhTAdBgNVHQ4E\n",
        "FgQUySxLgeMPU3unx4JE5o697py83KYwHwYDVR0jBBgwFoAUySxLgeMPU3unx4JE\n",
        "5o697py83KYwDwYDVR0TAQH/BAUwAwEB/zAyBgNVHREEKzApgg9zdWIuZXhhbXBs\n",
        "ZS5jb22HBMsAcQqHECABDbgAAAAAAAAAAAAAABAwCgYIKoZIzj0EAwIDSQAwRgIh\n",
        "AIxXr6v/d9W0zL5LkVGrjAb3uWIFoIVu5Kam3nNiwiNOAiEAqYbvDNrbh/BmYXp6\n",
        "Na69KLsU7VEoLc+BQp7GiKR9a2c=\n",
        "-----END CERTIFICATE-----\n"
    );
    const TEST_KEY: &str = concat!(
        "-----BEGIN EC PRIVATE KEY-----\n",
        "MHcCAQEEIH5iRWHzwLfe90FBzTTD/Wr05IZiiAbnx2rZQGXjGYDOoAoGCCqGSM49\n",
        "AwEHoUQDQgAEj2AOhpo4uXlnE7zCh4fhJJeM8E/KJG2mnyryCNeUqGBzBpbOkQm0\n",
        "gZcJO9V54nQkpxgN3RncWINJUMJ7GFNQzQ==\n",
        "-----END EC PRIVATE KEY-----\n"
    );
    const OTHER_KEY: &str = concat!(
        "-----BEGIN EC PRIVATE KEY-----\n",
        "MHcCAQEEIB/f2ogdlNty9umcSqm6aGjO//GBjGRGSM03wQElINFroAoGCCqGSM49\n",
        "AwEHoUQDQgAEDkvNtcrv7S7PNzK5mmMtfPV0AbLppRwNFIFc3gNHi8nZ08fUOJ7S\n",
        "BcuQdLNmc8b5xJFVNIybWjpMr2zng53oog==\n",
        "-----END EC PRIVATE KEY-----\n"
    );
    const PKCS8_KEY: &str = concat!(
        "-----BEGIN PRIVATE KEY-----\n",
        "MC4CAQAwBQYDK2VwBCIEIOzgRc4h5rLkwez0r/5iady9q7EPnuZTrsHLYyIBGlqH\n",
        "-----END PRIVATE KEY-----\n"
    );
    const RSA_KEY: &str = concat!(
        "-----BEGIN RSA PRIVATE KEY-----\n",
        "MIIBOQIBAAJBAMO5NnK62Bc/zAENcg0HmIn9V8JUHjAJpzpq4nSqfSDumJFPV5Ve\n",
        "jTw00rFXhmQq+V84s3NCcWWR7q+X/JZ0ST8CAwEAAQJAfn0KFSdvU8clHmEEHiuU\n",
        "h0k1GB+oyr7SVkyRQXiVGVw3xSRXqrbsHpsoW6bvbIdNlKT7efHCiwqUYaKCDj8k\n",
        "sQIhAOag3PMSd9FjD4hrAtsRsvwcM3dUZ3um96+8Bf3TMVQdAiEA2UFUtdMqfDqV\n",
        "g8Z2QcSRiCXb//x9+Lm/iu1KeyAknAsCIHXo1E2pqXxxquVR4JnjyKBAQsfFbUq4\n",
        "qHU+KcoFiXi5AiA/u/S36pz6GM2n/N7QaHQxNroVnOLvxr40aWyCNmnHBQIgECZG\n",
        "CENYDY4KxkEUbsJWmMJKXmnCGGtJ1X7EOQfvvRI=\n",
        "-----END RSA PRIVATE KEY-----\n"
    );
}
