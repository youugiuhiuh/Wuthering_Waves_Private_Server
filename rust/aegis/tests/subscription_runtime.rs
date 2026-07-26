use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use aegis::core::subscription::{
    certificate::{CertificatePromotion, ValidatedCertificate},
    config::{CertificateMode, SubscriptionConfig},
    runtime::{ManagedListener, RuntimeOps, SubscriptionPaths, SubscriptionRuntime},
    server::{SubscriptionSource, TlsFiles, TlsState, probe_https, spawn_listener},
};
use anyhow::{Result, bail};
use async_trait::async_trait;
use tempfile::TempDir;
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Certificate,
    Firewall,
    Bind,
    Probe,
    ConfigSave,
    Shutdown,
    Rollback,
}

#[derive(Default)]
struct RecordedState {
    actions: Vec<String>,
    firewall: HashSet<u16>,
    listeners: HashSet<u16>,
    live_certificate: Option<u16>,
    failure: Option<Failure>,
}

#[derive(Clone, Default)]
struct RecordingOps(Arc<Mutex<RecordedState>>);

impl RecordingOps {
    fn action(&self, action: impl Into<String>) {
        self.0.lock().unwrap().actions.push(action.into());
    }

    fn actions(&self) -> Vec<String> {
        self.0.lock().unwrap().actions.clone()
    }

    fn clear_actions(&self) {
        self.0.lock().unwrap().actions.clear();
    }

    fn fail(&self, failure: Failure) {
        self.0.lock().unwrap().failure = Some(failure);
    }
}

struct RecordingListener {
    port: u16,
    ops: RecordingOps,
}

#[async_trait]
impl ManagedListener for RecordingListener {
    fn local_addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
    }

    fn reload_tls(&self, _certificate: &ValidatedCertificate) -> Result<()> {
        self.ops.action(format!("tls:reload:{}", self.port));
        Ok(())
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        self.ops.action(format!("listener:stop:{}", self.port));
        self.ops.0.lock().unwrap().listeners.remove(&self.port);
        if self.ops.0.lock().unwrap().failure == Some(Failure::Shutdown) {
            bail!("shutdown failed");
        }
        Ok(())
    }
}

struct RecordingPromotion {
    ops: RecordingOps,
    old: Option<u16>,
}

struct NoopPromotion;

#[async_trait]
impl CertificatePromotion for NoopPromotion {
    async fn commit(self: Box<Self>) -> Result<()> {
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct LocalOps {
    certificate: ValidatedCertificate,
    firewall: Arc<Mutex<HashSet<u16>>>,
    delayed_port: u16,
    probe_started: Arc<Notify>,
    release_probe: Arc<Notify>,
}

#[async_trait]
impl RuntimeOps for LocalOps {
    async fn stage_certificate(
        &self,
        _config: &SubscriptionConfig,
    ) -> Result<ValidatedCertificate> {
        Ok(ValidatedCertificate {
            cert_path: self.certificate.cert_path.clone(),
            key_path: self.certificate.key_path.clone(),
            not_after: self.certificate.not_after,
        })
    }

    async fn promote_certificate(
        &self,
        _staged: &ValidatedCertificate,
    ) -> Result<Box<dyn CertificatePromotion>> {
        Ok(Box::new(NoopPromotion))
    }

    async fn open_port(&self, port: u16) -> Result<bool> {
        Ok(self.firewall.lock().unwrap().insert(port))
    }

    async fn close_port(&self, port: u16) -> Result<()> {
        self.firewall.lock().unwrap().remove(&port);
        Ok(())
    }

    async fn start_listener(
        &self,
        bind: SocketAddr,
        certificate: &ValidatedCertificate,
        source: SubscriptionSource,
    ) -> Result<Box<dyn ManagedListener>> {
        let tls = TlsState::load(TlsFiles::new(
            certificate.cert_path.clone(),
            certificate.key_path.clone(),
        ))?;
        Ok(Box::new(spawn_listener(bind, tls, source).await?))
    }

    async fn probe_listener(
        &self,
        listener: &dyn ManagedListener,
        config: &SubscriptionConfig,
    ) -> Result<()> {
        if listener.local_addr().port() == self.delayed_port {
            self.probe_started.notify_one();
            self.release_probe.notified().await;
        }
        probe_https(
            listener.local_addr(),
            &config.public_host,
            &self.certificate.cert_path,
        )
        .await
    }
}

#[async_trait]
impl CertificatePromotion for RecordingPromotion {
    async fn commit(self: Box<Self>) -> Result<()> {
        self.ops.action("certificate:commit");
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        self.ops.action("certificate:rollback");
        self.ops.0.lock().unwrap().live_certificate = self.old;
        if self.ops.0.lock().unwrap().failure == Some(Failure::Rollback) {
            bail!("rollback failed");
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeOps for RecordingOps {
    async fn stage_certificate(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        self.action("certificate:stage");
        if self.0.lock().unwrap().failure == Some(Failure::Certificate) {
            bail!("certificate failed");
        }
        Ok(ValidatedCertificate {
            cert_path: PathBuf::from(format!("staged-{}.pem", config.port)),
            key_path: PathBuf::from(format!("staged-{}.key", config.port)),
            not_after: SystemTime::now() + Duration::from_secs(3600),
        })
    }

    async fn promote_certificate(
        &self,
        staged: &ValidatedCertificate,
    ) -> Result<Box<dyn CertificatePromotion>> {
        self.action("certificate:promote");
        let new = staged_port(staged);
        let mut state = self.0.lock().unwrap();
        let old = state.live_certificate.replace(new);
        Ok(Box::new(RecordingPromotion {
            ops: self.clone(),
            old,
        }))
    }

    async fn open_port(&self, port: u16) -> Result<bool> {
        self.action(format!("firewall:open:{port}"));
        if self.0.lock().unwrap().failure == Some(Failure::Firewall) {
            bail!("firewall failed");
        }
        Ok(self.0.lock().unwrap().firewall.insert(port))
    }

    async fn close_port(&self, port: u16) -> Result<()> {
        self.action(format!("firewall:close:{port}"));
        self.0.lock().unwrap().firewall.remove(&port);
        Ok(())
    }

    async fn start_listener(
        &self,
        bind: SocketAddr,
        _certificate: &ValidatedCertificate,
        _source: SubscriptionSource,
    ) -> Result<Box<dyn ManagedListener>> {
        self.action(format!("listener:start:{}", bind.port()));
        if self.0.lock().unwrap().failure == Some(Failure::Bind) {
            bail!("bind failed");
        }
        self.0.lock().unwrap().listeners.insert(bind.port());
        Ok(Box::new(RecordingListener {
            port: bind.port(),
            ops: self.clone(),
        }))
    }

    async fn probe_listener(
        &self,
        listener: &dyn ManagedListener,
        _config: &SubscriptionConfig,
    ) -> Result<()> {
        self.action(format!("listener:probe:{}", listener.local_addr().port()));
        if self.0.lock().unwrap().failure == Some(Failure::Probe) {
            bail!("probe failed");
        }
        Ok(())
    }

    async fn save_config(&self, config: &SubscriptionConfig, path: &Path) -> Result<()> {
        self.action(format!("config:save:{}", config.port));
        if matches!(
            self.0.lock().unwrap().failure,
            Some(Failure::ConfigSave | Failure::Rollback)
        ) {
            bail!("save failed");
        }
        config.save_atomic(path)
    }
}

struct RuntimeFixture {
    _dir: TempDir,
    runtime: Arc<SubscriptionRuntime>,
    ops: RecordingOps,
    config_path: PathBuf,
}

impl RuntimeFixture {
    async fn running(config: SubscriptionConfig) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("subscription.json");
        config.save_atomic(&config_path).unwrap();
        let paths = SubscriptionPaths::new(
            config_path.clone(),
            dir.path().join("xray"),
            dir.path().join("singbox"),
        )
        .with_bind_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let ops = RecordingOps::default();
        let runtime = SubscriptionRuntime::new(paths, Arc::new(ops.clone()));
        runtime.start_from_disk().await.unwrap();
        ops.clear_actions();
        Self {
            _dir: dir,
            runtime,
            ops,
            config_path,
        }
    }

    fn saved_config(&self) -> SubscriptionConfig {
        SubscriptionConfig::load_from(&self.config_path)
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn apply_keeps_old_service_until_new_probe_and_commit() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.runtime.apply(new_port_config()).await.unwrap();
    assert_eq!(
        fixture.ops.actions(),
        [
            "certificate:stage",
            "firewall:open:8443",
            "listener:start:8443",
            "listener:probe:8443",
            "certificate:promote",
            "config:save:8443",
            "certificate:commit",
            "listener:stop:443",
            "firewall:close:443",
        ]
    );
    assert_eq!(fixture.saved_config().port, 8443);
}

#[tokio::test]
async fn certificate_firewall_and_listener_failures_preserve_old_state() {
    for failure in [
        Failure::Certificate,
        Failure::Firewall,
        Failure::Bind,
        Failure::Probe,
        Failure::ConfigSave,
    ] {
        let fixture = RuntimeFixture::running(old_config()).await;
        fixture.ops.fail(failure);
        assert!(fixture.runtime.apply(new_port_config()).await.is_err());
        assert_eq!(fixture.saved_config(), old_config());
        let state = fixture.ops.0.lock().unwrap();
        assert!(state.listeners.contains(&443), "failure={failure:?}");
        assert!(state.firewall.contains(&443), "failure={failure:?}");
        assert!(!state.firewall.contains(&8443), "failure={failure:?}");
        assert_eq!(state.live_certificate, Some(443), "failure={failure:?}");
    }
}

#[tokio::test]
async fn same_socket_reissue_reloads_tls_without_listener_restart() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.runtime.reissue_certificate().await.unwrap();
    let actions = fixture.ops.actions();
    assert!(actions.iter().any(|action| action == "tls:reload:443"));
    assert!(
        !actions
            .iter()
            .any(|action| action.starts_with("listener:stop"))
    );
    assert!(
        !actions
            .iter()
            .any(|action| action.starts_with("listener:start"))
    );
}

#[tokio::test]
async fn same_socket_save_failure_rolls_back_and_reloads_old_tls() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.ops.fail(Failure::ConfigSave);
    assert!(fixture.runtime.reissue_certificate().await.is_err());
    assert_eq!(fixture.saved_config(), old_config());
    assert_eq!(
        fixture.ops.actions(),
        [
            "certificate:stage",
            "certificate:promote",
            "tls:reload:443",
            "listener:probe:443",
            "config:save:443",
            "certificate:rollback",
            "tls:reload:443",
        ]
    );
}

#[tokio::test]
async fn same_socket_rollback_failure_still_reloads_old_tls() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.ops.fail(Failure::Rollback);
    assert!(fixture.runtime.reissue_certificate().await.is_err());
    assert_eq!(
        fixture
            .ops
            .actions()
            .iter()
            .filter(|action| action.as_str() == "tls:reload:443")
            .count(),
        2
    );
}

#[tokio::test]
async fn token_regeneration_persists_only_hash_and_publishes_new_urls_once() {
    let fixture = RuntimeFixture::running(old_config()).await;
    let old_hash = fixture.saved_config().token_hash;
    let urls = fixture.runtime.regenerate_token().await.unwrap();
    let saved = fixture.saved_config();
    assert_ne!(saved.token_hash, old_hash);
    assert!(!urls.standard.contains(&saved.token_hash));
    assert!(
        urls.standard
            .starts_with("https://old.example.com:443/sub/")
    );
    assert!(urls.clash.ends_with("/clash"));
    let status = fixture.runtime.status().await;
    assert_eq!(status.masked_token, saved.masked_token());
}

#[tokio::test]
async fn failed_token_regeneration_keeps_old_hash_and_redacts_status_error() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.ops.fail(Failure::ConfigSave);
    assert!(fixture.runtime.regenerate_token().await.is_err());
    assert_eq!(fixture.saved_config(), old_config());
    assert_eq!(
        fixture.runtime.status().await.last_error.as_deref(),
        Some("subscription token update failed")
    );
}

#[tokio::test]
async fn disable_persists_before_stopping_listener_and_closing_firewall() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.runtime.disable().await.unwrap();
    assert_eq!(
        fixture.ops.actions(),
        ["config:save:443", "listener:stop:443", "firewall:close:443"]
    );
    assert!(!fixture.saved_config().enabled);
}

#[tokio::test]
async fn failed_disable_preserves_running_service() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.ops.fail(Failure::ConfigSave);
    assert!(fixture.runtime.disable().await.is_err());
    let state = fixture.ops.0.lock().unwrap();
    assert!(state.listeners.contains(&443));
    assert!(state.firewall.contains(&443));
    assert!(fixture.saved_config().enabled);
}

#[tokio::test]
async fn shutdown_still_closes_firewall_when_listener_reports_failure() {
    let fixture = RuntimeFixture::running(old_config()).await;
    fixture.ops.fail(Failure::Shutdown);
    assert!(fixture.runtime.shutdown().await.is_err());
    assert!(!fixture.ops.0.lock().unwrap().firewall.contains(&443));
}

#[tokio::test]
async fn real_listener_overlaps_during_probe_and_reads_node_changes_per_request() {
    let dir = tempfile::tempdir().unwrap();
    let xray = dir.path().join("xray");
    let singbox = dir.path().join("singbox");
    std::fs::create_dir_all(&xray).unwrap();
    std::fs::create_dir_all(&singbox).unwrap();
    write_node(&xray.join("first.json"), "first");
    write_node(&xray.join("second.json"), "second");
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, TEST_CERT).unwrap();
    std::fs::write(&key_path, TEST_KEY).unwrap();
    let old_port = reserve_port();
    let mut new_port = reserve_port();
    while new_port == old_port {
        new_port = reserve_port();
    }
    let raw_token = "runtime-test-token";
    let mut old = local_config(old_port, raw_token, &cert_path, &key_path);
    let config_path = dir.path().join("subscription.json");
    old.save_atomic(&config_path).unwrap();
    let probe_started = Arc::new(Notify::new());
    let release_probe = Arc::new(Notify::new());
    let ops = Arc::new(LocalOps {
        certificate: ValidatedCertificate {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
            not_after: SystemTime::now() + Duration::from_secs(3600),
        },
        firewall: Arc::default(),
        delayed_port: new_port,
        probe_started: probe_started.clone(),
        release_probe: release_probe.clone(),
    });
    let runtime = SubscriptionRuntime::new(
        SubscriptionPaths::new(config_path, xray.clone(), singbox)
            .with_bind_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ops,
    );
    runtime.start_from_disk().await.unwrap();

    old.port = new_port;
    let applying = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.apply(old).await }
    });
    tokio::time::timeout(Duration::from_secs(2), probe_started.notified())
        .await
        .unwrap();
    probe_https(localhost(old_port), "first.local", &cert_path)
        .await
        .unwrap();
    applying.abort();
    let _ = applying.await;
    release_probe.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.status().await.port != new_port {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    probe_https(localhost(new_port), "first.local", &cert_path)
        .await
        .unwrap();
    assert!(
        probe_https(localhost(old_port), "first.local", &cert_path)
            .await
            .is_err()
    );

    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(TEST_CERT.as_bytes()).unwrap())
        .resolve("first.local", localhost(new_port))
        .build()
        .unwrap();
    let url = format!("https://first.local:{new_port}/sub/{raw_token}");
    let first = client.get(&url).send().await.unwrap().text().await.unwrap();
    write_node(&xray.join("first.json"), "edited");
    let second = client.get(&url).send().await.unwrap().text().await.unwrap();
    std::fs::remove_file(xray.join("second.json")).unwrap();
    let third = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert_ne!(first, second);
    assert_ne!(second, third);
    runtime.shutdown().await.unwrap();
}

fn staged_port(certificate: &ValidatedCertificate) -> u16 {
    certificate
        .cert_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("staged-")
        .parse()
        .unwrap()
}

fn old_config() -> SubscriptionConfig {
    config(443, "old.example.com", "11")
}

fn new_port_config() -> SubscriptionConfig {
    config(8443, "new.example.com", "22")
}

fn config(port: u16, host: &str, hash_byte: &str) -> SubscriptionConfig {
    SubscriptionConfig {
        enabled: true,
        port,
        public_host: host.into(),
        ipv6_san: None,
        token_hash: hash_byte.repeat(32),
        certificate_mode: CertificateMode::Domain,
        cert_path: PathBuf::from("live.pem"),
        key_path: PathBuf::from("live.key"),
    }
}

fn local_config(
    port: u16,
    raw_token: &str,
    cert_path: &Path,
    key_path: &Path,
) -> SubscriptionConfig {
    use sha2::{Digest, Sha256};

    SubscriptionConfig {
        enabled: true,
        port,
        public_host: "first.local".into(),
        ipv6_san: None,
        token_hash: hex::encode(Sha256::digest(raw_token.as_bytes())),
        certificate_mode: CertificateMode::Domain,
        cert_path: cert_path.to_owned(),
        key_path: key_path.to_owned(),
    }
}

fn reserve_port() -> u16 {
    std::net::TcpListener::bind(localhost(0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn localhost(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn write_node(path: &Path, name: &str) {
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({"inbounds": [{
            "port": 443,
            "protocol": "vless",
            "settings": {"clients": [{"id": "123e4567-e89b-12d3-a456-426614174000", "email": name, "flow": "xtls-rprx-vision"}]},
            "streamSettings": {"network": "tcp", "security": "reality", "realitySettings": {
                "serverNames": ["example.com"],
                "privateKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "shortIds": ["0123456789abcdef"]
            }}
        }]}))
        .unwrap(),
    )
    .unwrap();
}

const TEST_CERT: &str = concat!(
    "-----BEGIN CERTIFICATE-----\n",
    "MIIBfTCCAS+gAwIBAgIUTcO+/IRlal0NTkIHxFMuSywy/v0wBQYDK2VwMBYxFDAS\n",
    "BgNVBAMMC2ZpcnN0LmxvY2FsMB4XDTI2MDcyNjA0NTAzN1oXDTM2MDcyMzA0NTAz\n",
    "N1owFjEUMBIGA1UEAwwLZmlyc3QubG9jYWwwKjAFBgMrZXADIQBWRWUDYUYSmCkF\n",
    "50N+Z0hQ1+hF1NhzypcBwUht7UjaaaOBjjCBizAdBgNVHQ4EFgQUdBBxl73LFuFF\n",
    "eBO6hQZDLBS9tXAwHwYDVR0jBBgwFoAUdBBxl73LFuFFeBO6hQZDLBS9tXAwFgYD\n",
    "VR0RBA8wDYILZmlyc3QubG9jYWwwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMC\n",
    "B4AwEwYDVR0lBAwwCgYIKwYBBQUHAwEwBQYDK2VwA0EAQZd/ONEQVrTd+0GLPjtO\n",
    "+grnzm+fQALcjD7G1H+z6m0QYwv8WXdnL+UL40HbK1EXv97ZW8bRRNCwhoyJYfTD\n",
    "BA==\n",
    "-----END CERTIFICATE-----\n"
);

const TEST_KEY: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n",
    "MC4CAQAwBQYDK2VwBCIEIAXOm7FeRbdfdtN927lNMSX0geEm9nYauCCSWnVo8pxr\n",
    "-----END PRIVATE KEY-----\n"
);
