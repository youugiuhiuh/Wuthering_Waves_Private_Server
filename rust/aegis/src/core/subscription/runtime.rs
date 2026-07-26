use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use tokio::sync::{Mutex, watch};

use super::{
    certificate::{
        CertificateManager, CertificatePromotion, SystemAcmeFirewall, SystemCommandRunner,
        ValidatedCertificate,
    },
    config::{GeneratedToken, SubscriptionConfig},
    server::{ListenerHandle, SubscriptionSource, TlsFiles, TlsState, probe_https, spawn_listener},
};
use crate::core::{paths, security::firewall::FirewallManager};

static SUBSCRIPTION_RUNTIME: OnceCell<Arc<SubscriptionRuntime>> = OnceCell::new();

#[derive(Clone)]
pub struct SubscriptionPaths {
    pub config_file: PathBuf,
    pub xray_dir: PathBuf,
    pub singbox_dir: PathBuf,
    pub bind_ip: IpAddr,
}

impl SubscriptionPaths {
    pub fn new(config_file: PathBuf, xray_dir: PathBuf, singbox_dir: PathBuf) -> Self {
        Self {
            config_file,
            xray_dir,
            singbox_dir,
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }

    pub fn production() -> Self {
        Self::new(
            paths::subscription::CONFIG_FILE.into(),
            paths::xray::CONF_DIR.into(),
            paths::singbox::CONF_DIR.into(),
        )
    }

    pub fn with_bind_ip(mut self, bind_ip: IpAddr) -> Self {
        self.bind_ip = bind_ip;
        self
    }
}

#[async_trait]
pub trait ManagedListener: Send + Sync {
    fn local_addr(&self) -> SocketAddr;
    fn reload_tls(&self, certificate: &ValidatedCertificate) -> Result<()>;
    async fn shutdown(self: Box<Self>) -> Result<()>;
}

#[async_trait]
impl ManagedListener for ListenerHandle {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    fn reload_tls(&self, certificate: &ValidatedCertificate) -> Result<()> {
        self.tls.reload(TlsFiles::new(
            certificate.cert_path.clone(),
            certificate.key_path.clone(),
        ))
    }

    async fn shutdown(self: Box<Self>) -> Result<()> {
        (*self).shutdown().await
    }
}

#[async_trait]
pub trait RuntimeOps: Send + Sync {
    async fn stage_certificate(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate>;
    async fn promote_certificate(
        &self,
        staged: &ValidatedCertificate,
    ) -> Result<Box<dyn CertificatePromotion>>;
    async fn open_port(&self, port: u16) -> Result<bool>;
    async fn close_port(&self, port: u16) -> Result<()>;
    async fn discard_certificate(&self, _staged: &ValidatedCertificate) -> Result<()> {
        Ok(())
    }
    async fn start_listener(
        &self,
        bind: SocketAddr,
        certificate: &ValidatedCertificate,
        source: SubscriptionSource,
    ) -> Result<Box<dyn ManagedListener>>;
    async fn probe_listener(
        &self,
        listener: &dyn ManagedListener,
        config: &SubscriptionConfig,
    ) -> Result<()>;

    async fn save_config(&self, config: &SubscriptionConfig, path: &Path) -> Result<()> {
        config.save_atomic(path)
    }
}

struct PendingCertificate {
    config: SubscriptionConfig,
    cert_path: PathBuf,
}

pub struct ProductionRuntimeOps {
    certificates: CertificateManager<SystemCommandRunner, SystemAcmeFirewall>,
    pending: Mutex<Option<PendingCertificate>>,
}

impl ProductionRuntimeOps {
    pub fn new() -> Result<Self> {
        Ok(Self {
            certificates: CertificateManager::production()?,
            pending: Mutex::new(None),
        })
    }
}

#[async_trait]
impl RuntimeOps for ProductionRuntimeOps {
    async fn stage_certificate(&self, config: &SubscriptionConfig) -> Result<ValidatedCertificate> {
        let certificate = self.certificates.issue(config).await?;
        *self.pending.lock().await = Some(PendingCertificate {
            config: config.clone(),
            cert_path: certificate.cert_path.clone(),
        });
        Ok(certificate)
    }

    async fn promote_certificate(
        &self,
        staged: &ValidatedCertificate,
    ) -> Result<Box<dyn CertificatePromotion>> {
        let pending = self
            .pending
            .lock()
            .await
            .take()
            .context("no staged subscription certificate")?;
        self.certificates
            .promote(
                &pending.config,
                ValidatedCertificate {
                    cert_path: staged.cert_path.clone(),
                    key_path: staged.key_path.clone(),
                    not_after: staged.not_after,
                },
            )
            .await
    }

    async fn open_port(&self, port: u16) -> Result<bool> {
        if FirewallManager::list_allowed_ports().await?.contains(&port) {
            return Ok(false);
        }
        FirewallManager::add_port(port).await?;
        Ok(true)
    }

    async fn close_port(&self, port: u16) -> Result<()> {
        FirewallManager::remove_port(port).await
    }

    async fn discard_certificate(&self, staged: &ValidatedCertificate) -> Result<()> {
        *self.pending.lock().await = None;
        let mut failures = Vec::new();
        if remove_file_if_present(&staged.cert_path).is_err() {
            failures.push("staged certificate");
        }
        if remove_file_if_present(&staged.key_path).is_err() {
            failures.push("staged private key");
        }
        if let Some(parent) = staged.cert_path.parent() {
            match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(_) => failures.push("staging directory"),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(operation_error("certificate discard", &failures))
        }
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
        let address = listener.local_addr();
        let pending = self.pending.lock().await;
        let cert_path = pending
            .as_ref()
            .filter(|pending| pending.cert_path.exists())
            .map_or(config.cert_path.as_path(), |pending| {
                pending.cert_path.as_path()
            });
        let server_name = config
            .public_host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        probe_https(address, server_name, cert_path).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionStatus {
    pub enabled: bool,
    pub public_host: String,
    pub port: u16,
    pub certificate_not_after: Option<SystemTime>,
    pub token_hash: String,
    pub masked_token: String,
    pub last_error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NewSubscriptionUrls {
    pub standard: String,
    pub clash: String,
}

#[derive(Default)]
struct RuntimeState {
    config: Option<SubscriptionConfig>,
    config_tx: Option<watch::Sender<Arc<SubscriptionConfig>>>,
    listener: Option<Box<dyn ManagedListener>>,
    certificate_not_after: Option<SystemTime>,
    last_error: Option<String>,
}

pub struct SubscriptionRuntime {
    paths: SubscriptionPaths,
    ops: Arc<dyn RuntimeOps>,
    state: Mutex<RuntimeState>,
}

pub fn subscription_runtime() -> Option<Arc<SubscriptionRuntime>> {
    SUBSCRIPTION_RUNTIME.get().cloned()
}

impl SubscriptionRuntime {
    pub fn new(paths: SubscriptionPaths, ops: Arc<dyn RuntimeOps>) -> Arc<Self> {
        SUBSCRIPTION_RUNTIME
            .get_or_init(|| Self::build(paths, ops))
            .clone()
    }

    #[doc(hidden)]
    pub fn new_isolated_for_test(paths: SubscriptionPaths, ops: Arc<dyn RuntimeOps>) -> Arc<Self> {
        Self::build(paths, ops)
    }

    fn build(paths: SubscriptionPaths, ops: Arc<dyn RuntimeOps>) -> Arc<Self> {
        Arc::new(Self {
            paths,
            ops,
            state: Mutex::new(RuntimeState::default()),
        })
    }

    pub async fn start_from_disk(self: &Arc<Self>) -> Result<()> {
        let Some(config) = SubscriptionConfig::load_from(&self.paths.config_file)? else {
            return Ok(());
        };
        if config.enabled {
            self.apply(config).await
        } else {
            self.state.lock().await.config = Some(config);
            Ok(())
        }
    }

    pub async fn apply(self: &Arc<Self>, candidate: SubscriptionConfig) -> Result<()> {
        let runtime = self.clone();
        tokio::spawn(async move { runtime.apply_inner(candidate).await })
            .await
            .context("subscription update task failed")?
    }

    async fn apply_inner(&self, candidate: SubscriptionConfig) -> Result<()> {
        if !candidate.enabled {
            return self.disable_inner().await;
        }
        candidate.validate()?;
        let mut state = self.state.lock().await;
        state.last_error = None;
        let same_socket = state.listener.is_some()
            && state
                .config
                .as_ref()
                .is_some_and(|current| current.port == candidate.port);
        let result = if same_socket {
            self.apply_same_socket(&mut state, candidate).await
        } else {
            self.apply_new_socket(&mut state, candidate).await
        };
        if result.is_err() && state.last_error.is_none() {
            state.last_error = Some("subscription update failed".into());
        }
        result
    }

    async fn apply_same_socket(
        &self,
        state: &mut RuntimeState,
        candidate: SubscriptionConfig,
    ) -> Result<()> {
        let staged = self
            .ops
            .stage_certificate(&candidate)
            .await
            .map_err(|_| operation_error("certificate stage", &[]))?;
        let Some(config_tx) = active_config_sender(state) else {
            let cleanup = discard_staged(self.ops.as_ref(), &staged).await;
            return Err(operation_error("config publication", &cleanup));
        };
        let promotion = match self.ops.promote_certificate(&staged).await {
            Ok(promotion) => promotion,
            Err(_) => {
                let cleanup = discard_staged(self.ops.as_ref(), &staged).await;
                return Err(operation_error("certificate promotion", &cleanup));
            }
        };
        let live = live_certificate(&candidate, staged.not_after);
        let listener = state.listener.as_deref().context("listener is missing")?;
        if listener.reload_tls(&live).is_err() {
            let mut cleanup = rollback_and_reload(promotion, listener, state).await;
            if !cleanup.is_empty() {
                cleanup.extend(deactivate_running(self.ops.as_ref(), state).await);
            }
            return Err(operation_error("TLS reload", &cleanup));
        }
        if self.ops.probe_listener(listener, &candidate).await.is_err() {
            let mut cleanup = rollback_and_reload(promotion, listener, state).await;
            if !cleanup.is_empty() {
                cleanup.extend(deactivate_running(self.ops.as_ref(), state).await);
            }
            return Err(operation_error("listener probe", &cleanup));
        }
        if self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
            .is_err()
        {
            let mut cleanup = rollback_and_reload(promotion, listener, state).await;
            if !cleanup.is_empty() {
                cleanup.extend(deactivate_running(self.ops.as_ref(), state).await);
            }
            return Err(operation_error("config save", &cleanup));
        }
        if config_tx.send(Arc::new(candidate.clone())).is_err() {
            let mut cleanup = Vec::new();
            if self
                .ops
                .save_config(
                    state.config.as_ref().expect("running config is present"),
                    &self.paths.config_file,
                )
                .await
                .is_err()
            {
                cleanup.push("config restore");
            }
            cleanup.extend(rollback_and_reload(promotion, listener, state).await);
            if cleanup
                .iter()
                .any(|failure| matches!(*failure, "certificate rollback" | "old TLS reload"))
            {
                cleanup.extend(deactivate_running(self.ops.as_ref(), state).await);
            }
            return Err(operation_error("config publication", &cleanup));
        }
        state.config = Some(candidate);
        state.certificate_not_after = Some(staged.not_after);
        let mut cleanup = Vec::new();
        if promotion.commit().await.is_err() {
            cleanup.push("certificate finalization");
        }
        finish_committed(state, cleanup)
    }

    async fn apply_new_socket(
        &self,
        state: &mut RuntimeState,
        candidate: SubscriptionConfig,
    ) -> Result<()> {
        let staged = self
            .ops
            .stage_certificate(&candidate)
            .await
            .map_err(|_| operation_error("certificate stage", &[]))?;
        let opened = match self.ops.open_port(candidate.port).await {
            Ok(opened) => opened,
            Err(_) => {
                let cleanup = discard_staged(self.ops.as_ref(), &staged).await;
                return Err(operation_error("firewall open", &cleanup));
            }
        };
        let (config_tx, _) = watch::channel(Arc::new(candidate.clone()));
        let source = SubscriptionSource {
            config_rx: config_tx.subscribe(),
            xray_dir: self.paths.xray_dir.clone(),
            singbox_dir: self.paths.singbox_dir.clone(),
        };
        let bind = SocketAddr::new(self.paths.bind_ip, candidate.port);
        let listener = match self.ops.start_listener(bind, &staged, source).await {
            Ok(listener) => listener,
            Err(_) => {
                let cleanup = cleanup_staged_transition(
                    self.ops.as_ref(),
                    None,
                    candidate.port,
                    opened,
                    &staged,
                )
                .await;
                return Err(operation_error("listener start", &cleanup));
            }
        };
        if self
            .ops
            .probe_listener(listener.as_ref(), &candidate)
            .await
            .is_err()
        {
            let cleanup = cleanup_staged_transition(
                self.ops.as_ref(),
                Some(listener),
                candidate.port,
                opened,
                &staged,
            )
            .await;
            return Err(operation_error("listener probe", &cleanup));
        }
        if config_tx.receiver_count() == 0 {
            let cleanup = cleanup_staged_transition(
                self.ops.as_ref(),
                Some(listener),
                candidate.port,
                opened,
                &staged,
            )
            .await;
            return Err(operation_error("config publication", &cleanup));
        }
        let promotion = match self.ops.promote_certificate(&staged).await {
            Ok(promotion) => promotion,
            Err(_) => {
                let cleanup = cleanup_staged_transition(
                    self.ops.as_ref(),
                    Some(listener),
                    candidate.port,
                    opened,
                    &staged,
                )
                .await;
                return Err(operation_error("certificate promotion", &cleanup));
            }
        };
        if self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
            .is_err()
        {
            let cleanup = cleanup_promoted_transition(
                self.ops.as_ref(),
                promotion,
                listener,
                candidate.port,
                opened,
            )
            .await;
            return Err(operation_error("config save", &cleanup));
        }
        if config_tx.send(Arc::new(candidate.clone())).is_err() {
            let old_config = state.config.clone();
            let mut cleanup = Vec::new();
            if let Some(old_config) = &old_config
                && self
                    .ops
                    .save_config(old_config, &self.paths.config_file)
                    .await
                    .is_err()
            {
                cleanup.push("config restore");
            }
            cleanup.extend(
                cleanup_promoted_transition(
                    self.ops.as_ref(),
                    promotion,
                    listener,
                    candidate.port,
                    opened,
                )
                .await,
            );
            return Err(operation_error("config publication", &cleanup));
        }

        let old_listener = state.listener.replace(listener);
        let old_port = state.config.as_ref().map(|config| config.port);
        state.config = Some(candidate);
        state.config_tx = Some(config_tx);
        state.certificate_not_after = Some(staged.not_after);
        let mut cleanup = Vec::new();
        if promotion.commit().await.is_err() {
            cleanup.push("certificate finalization");
        }
        if let Some(listener) = old_listener
            && listener.shutdown().await.is_err()
        {
            cleanup.push("listener retirement");
        }
        if let Some(port) =
            old_port.filter(|port| Some(*port) != state.config.as_ref().map(|c| c.port))
            && self.ops.close_port(port).await.is_err()
        {
            cleanup.push("firewall retirement");
        }
        finish_committed(state, cleanup)
    }

    pub async fn disable(self: &Arc<Self>) -> Result<()> {
        let runtime = self.clone();
        tokio::spawn(async move { runtime.disable_inner().await })
            .await
            .context("subscription disable task failed")?
    }

    async fn disable_inner(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let Some(current) = state.config.clone() else {
            bail!("subscription is not configured");
        };
        if !current.enabled {
            return Ok(());
        }
        let mut disabled = current.clone();
        disabled.enabled = false;
        if self
            .ops
            .save_config(&disabled, &self.paths.config_file)
            .await
            .is_err()
        {
            state.last_error = Some("subscription disable failed".into());
            return Err(operation_error("disable config save", &[]));
        }
        if let Some(config_tx) = &state.config_tx {
            let _ = config_tx.send(Arc::new(disabled.clone()));
        }
        let listener = state.listener.take();
        let port = current.port;
        state.config = Some(disabled);
        state.config_tx = None;
        state.certificate_not_after = None;
        let mut cleanup = Vec::new();
        if let Some(listener) = listener
            && listener.shutdown().await.is_err()
        {
            cleanup.push("listener retirement");
        }
        if self.ops.close_port(port).await.is_err() {
            cleanup.push("firewall retirement");
        }
        finish_committed(&mut state, cleanup)
    }

    pub async fn regenerate_token(self: &Arc<Self>) -> Result<NewSubscriptionUrls> {
        let runtime = self.clone();
        tokio::spawn(async move { runtime.regenerate_token_inner().await })
            .await
            .context("subscription token task failed")?
    }

    async fn regenerate_token_inner(&self) -> Result<NewSubscriptionUrls> {
        let mut state = self.state.lock().await;
        let current = state
            .config
            .as_ref()
            .context("subscription is not configured")?;
        let generated = GeneratedToken::new();
        let mut candidate = current.clone();
        candidate.token_hash = generated.hash().to_owned();
        let config_tx = if state.listener.is_some() {
            Some(active_config_sender(&state).context("active config receiver is missing")?)
        } else {
            None
        };
        if self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
            .is_err()
        {
            state.last_error = Some("subscription token update failed".into());
            return Err(operation_error("token config save", &[]));
        }
        if let Some(config_tx) = config_tx
            && config_tx.send(Arc::new(candidate.clone())).is_err()
        {
            let mut cleanup = Vec::new();
            if self
                .ops
                .save_config(current, &self.paths.config_file)
                .await
                .is_err()
            {
                cleanup.push("config restore");
            }
            state.last_error = Some("subscription token update failed".into());
            return Err(operation_error("token publication", &cleanup));
        }
        let standard = format!("{}/sub/{}", candidate.public_base_url(), generated.raw());
        let clash = format!("{standard}/clash");
        state.config = Some(candidate);
        state.last_error = None;
        Ok(NewSubscriptionUrls { standard, clash })
    }

    pub async fn reissue_certificate(self: &Arc<Self>) -> Result<()> {
        let runtime = self.clone();
        tokio::spawn(async move { runtime.reissue_inner().await })
            .await
            .context("subscription reissue task failed")?
    }

    async fn reissue_inner(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.last_error = None;
        let config = state
            .config
            .clone()
            .context("subscription is not configured")?;
        if !config.enabled {
            bail!("subscription is disabled");
        }
        let result = self.apply_same_socket(&mut state, config).await;
        if result.is_err() && state.last_error.is_none() {
            state.last_error = Some("subscription update failed".into());
        }
        result
    }

    pub async fn status(&self) -> SubscriptionStatus {
        let state = self.state.lock().await;
        match &state.config {
            Some(config) => SubscriptionStatus {
                enabled: config.enabled && state.listener.is_some(),
                public_host: config.public_host.clone(),
                port: config.port,
                certificate_not_after: state.certificate_not_after,
                token_hash: config.token_hash.clone(),
                masked_token: config.masked_token(),
                last_error: state.last_error.clone(),
            },
            None => SubscriptionStatus {
                enabled: false,
                public_host: String::new(),
                port: 0,
                certificate_not_after: None,
                token_hash: String::new(),
                masked_token: String::new(),
                last_error: state.last_error.clone(),
            },
        }
    }

    pub async fn renew_if_due(self: &Arc<Self>) -> Result<()> {
        let state = self.state.lock().await;
        let Some(not_after) = state.certificate_not_after else {
            return Ok(());
        };
        let remaining = not_after
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        if remaining <= Duration::from_secs(30 * 24 * 3600) {
            drop(state);
            self.reissue_certificate().await
        } else {
            Ok(())
        }
    }

    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        let runtime = self.clone();
        tokio::spawn(async move { runtime.shutdown_inner().await })
            .await
            .context("subscription shutdown task failed")?
    }

    async fn shutdown_inner(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let listener = state.listener.take();
        let port = state
            .config
            .as_ref()
            .filter(|config| config.enabled)
            .map(|config| config.port);
        state.config_tx = None;
        let mut failures = Vec::new();
        if let Some(listener) = listener
            && listener.shutdown().await.is_err()
        {
            failures.push("listener shutdown");
        }
        if let Some(port) = port
            && self.ops.close_port(port).await.is_err()
        {
            failures.push("firewall close");
        }
        if failures.is_empty() {
            state.last_error = None;
            Ok(())
        } else {
            let error = operation_error("shutdown", &failures);
            state.last_error = Some(error.to_string());
            Err(error)
        }
    }
}

fn active_config_sender(state: &RuntimeState) -> Option<watch::Sender<Arc<SubscriptionConfig>>> {
    state
        .listener
        .as_ref()
        .and(state.config_tx.as_ref())
        .filter(|sender| sender.receiver_count() > 0)
        .cloned()
}

fn operation_error(operation: &str, cleanup: &[&str]) -> anyhow::Error {
    if cleanup.is_empty() {
        anyhow::anyhow!("subscription {operation} failed")
    } else {
        anyhow::anyhow!(
            "subscription {operation} failed; cleanup failed: {}",
            cleanup.join(", ")
        )
    }
}

fn finish_committed(state: &mut RuntimeState, cleanup: Vec<&str>) -> Result<()> {
    if cleanup.is_empty() {
        state.last_error = None;
        Ok(())
    } else {
        let message = format!(
            "subscription transition committed; cleanup failed: {}",
            cleanup.join(", ")
        );
        state.last_error = Some(message.clone());
        Err(anyhow::anyhow!(message))
    }
}

async fn discard_staged(ops: &dyn RuntimeOps, staged: &ValidatedCertificate) -> Vec<&'static str> {
    if ops.discard_certificate(staged).await.is_err() {
        vec!["certificate discard"]
    } else {
        Vec::new()
    }
}

async fn cleanup_staged_transition(
    ops: &dyn RuntimeOps,
    listener: Option<Box<dyn ManagedListener>>,
    port: u16,
    opened: bool,
    staged: &ValidatedCertificate,
) -> Vec<&'static str> {
    let mut failures = Vec::new();
    if let Some(listener) = listener
        && listener.shutdown().await.is_err()
    {
        failures.push("listener cleanup");
    }
    if opened && ops.close_port(port).await.is_err() {
        failures.push("firewall cleanup");
    }
    if ops.discard_certificate(staged).await.is_err() {
        failures.push("certificate discard");
    }
    failures
}

async fn cleanup_promoted_transition(
    ops: &dyn RuntimeOps,
    promotion: Box<dyn CertificatePromotion>,
    listener: Box<dyn ManagedListener>,
    port: u16,
    opened: bool,
) -> Vec<&'static str> {
    let mut failures = Vec::new();
    if promotion.rollback().await.is_err() {
        failures.push("certificate rollback");
    }
    if listener.shutdown().await.is_err() {
        failures.push("listener cleanup");
    }
    if opened && ops.close_port(port).await.is_err() {
        failures.push("firewall cleanup");
    }
    failures
}

async fn deactivate_running(ops: &dyn RuntimeOps, state: &mut RuntimeState) -> Vec<&'static str> {
    let listener = state.listener.take();
    let port = state.config.as_ref().map(|config| config.port);
    state.config_tx = None;
    state.certificate_not_after = None;
    let mut failures = Vec::new();
    if let Some(listener) = listener
        && listener.shutdown().await.is_err()
    {
        failures.push("listener deactivation");
    }
    if let Some(port) = port
        && ops.close_port(port).await.is_err()
    {
        failures.push("firewall deactivation");
    }
    failures
}

fn live_certificate(config: &SubscriptionConfig, not_after: SystemTime) -> ValidatedCertificate {
    ValidatedCertificate {
        cert_path: config.cert_path.clone(),
        key_path: config.key_path.clone(),
        not_after,
    }
}

fn reload_previous(listener: &dyn ManagedListener, state: &RuntimeState) -> Result<()> {
    let config = state
        .config
        .as_ref()
        .context("previous subscription config is missing")?;
    listener.reload_tls(&live_certificate(
        config,
        state
            .certificate_not_after
            .unwrap_or(SystemTime::UNIX_EPOCH),
    ))
}

async fn rollback_and_reload(
    promotion: Box<dyn CertificatePromotion>,
    listener: &dyn ManagedListener,
    state: &RuntimeState,
) -> Vec<&'static str> {
    let mut failures = Vec::new();
    if promotion.rollback().await.is_err() {
        failures.push("certificate rollback");
    }
    if reload_previous(listener, state).is_err() {
        failures.push("old TLS reload");
    }
    failures
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
