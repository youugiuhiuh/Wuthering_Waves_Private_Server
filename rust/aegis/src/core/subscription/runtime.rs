use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
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
        let runtime = Arc::new(Self {
            paths,
            ops,
            state: Mutex::new(RuntimeState::default()),
        });
        let _ = SUBSCRIPTION_RUNTIME.set(runtime.clone());
        runtime
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
        if result.is_err() {
            state.last_error = Some("subscription update failed".into());
        }
        result
    }

    async fn apply_same_socket(
        &self,
        state: &mut RuntimeState,
        candidate: SubscriptionConfig,
    ) -> Result<()> {
        let staged = self.ops.stage_certificate(&candidate).await?;
        let promotion = self.ops.promote_certificate(&staged).await?;
        let live = live_certificate(&candidate, staged.not_after);
        let listener = state.listener.as_deref().context("listener is missing")?;
        if let Err(error) = listener.reload_tls(&live) {
            rollback_and_reload(promotion, listener, state).await?;
            return Err(error.context("failed to reload subscription TLS"));
        }
        if let Err(error) = self.ops.probe_listener(listener, &candidate).await {
            rollback_and_reload(promotion, listener, state).await?;
            return Err(error.context("failed to probe subscription listener"));
        }
        if let Err(error) = self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
        {
            rollback_and_reload(promotion, listener, state).await?;
            return Err(error.context("failed to save subscription config"));
        }
        if let Some(config_tx) = &state.config_tx {
            config_tx.send_replace(Arc::new(candidate.clone()));
        }
        state.config = Some(candidate);
        state.certificate_not_after = Some(staged.not_after);
        state.last_error = None;
        promotion.commit().await
    }

    async fn apply_new_socket(
        &self,
        state: &mut RuntimeState,
        candidate: SubscriptionConfig,
    ) -> Result<()> {
        let staged = self.ops.stage_certificate(&candidate).await?;
        let opened = self.ops.open_port(candidate.port).await?;
        let (config_tx, _) = watch::channel(Arc::new(candidate.clone()));
        let source = SubscriptionSource {
            config_rx: config_tx.subscribe(),
            xray_dir: self.paths.xray_dir.clone(),
            singbox_dir: self.paths.singbox_dir.clone(),
        };
        let bind = SocketAddr::new(self.paths.bind_ip, candidate.port);
        let listener = match self.ops.start_listener(bind, &staged, source).await {
            Ok(listener) => listener,
            Err(error) => {
                if opened {
                    self.ops.close_port(candidate.port).await?;
                }
                return Err(error.context("failed to start subscription listener"));
            }
        };
        if let Err(error) = self.ops.probe_listener(listener.as_ref(), &candidate).await {
            let stopped = listener.shutdown().await;
            let closed = close_if_opened(self.ops.as_ref(), candidate.port, opened).await;
            stopped?;
            closed?;
            return Err(error.context("failed to probe subscription listener"));
        }
        let promotion = match self.ops.promote_certificate(&staged).await {
            Ok(promotion) => promotion,
            Err(error) => {
                let stopped = listener.shutdown().await;
                let closed = close_if_opened(self.ops.as_ref(), candidate.port, opened).await;
                stopped?;
                closed?;
                return Err(error.context("failed to promote subscription certificate"));
            }
        };
        if let Err(error) = self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
        {
            let rolled_back = promotion.rollback().await;
            let stopped = listener.shutdown().await;
            let closed = close_if_opened(self.ops.as_ref(), candidate.port, opened).await;
            rolled_back?;
            stopped?;
            closed?;
            return Err(error.context("failed to save subscription config"));
        }

        config_tx.send_replace(Arc::new(candidate.clone()));
        let committed = promotion.commit().await;
        let old_listener = state.listener.replace(listener);
        let old_port = state.config.as_ref().map(|config| config.port);
        state.config = Some(candidate.clone());
        state.config_tx = Some(config_tx);
        state.certificate_not_after = Some(staged.not_after);
        state.last_error = None;
        let stopped = match old_listener {
            Some(listener) => listener.shutdown().await,
            None => Ok(()),
        };
        let closed = match old_port.filter(|port| *port != candidate.port) {
            Some(port) => self.ops.close_port(port).await,
            None => Ok(()),
        };
        committed?;
        stopped?;
        closed
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
        if let Err(error) = self
            .ops
            .save_config(&disabled, &self.paths.config_file)
            .await
        {
            state.last_error = Some("subscription disable failed".into());
            return Err(error.context("failed to save disabled subscription config"));
        }
        if let Some(config_tx) = &state.config_tx {
            config_tx.send_replace(Arc::new(disabled.clone()));
        }
        let listener = state.listener.take();
        let port = current.port;
        state.config = Some(disabled);
        state.config_tx = None;
        state.certificate_not_after = None;
        state.last_error = None;
        let stopped = match listener {
            Some(listener) => listener.shutdown().await,
            None => Ok(()),
        };
        let closed = self.ops.close_port(port).await;
        stopped?;
        closed
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
        if let Err(error) = self
            .ops
            .save_config(&candidate, &self.paths.config_file)
            .await
        {
            state.last_error = Some("subscription token update failed".into());
            return Err(error.context("failed to save subscription token"));
        }
        if let Some(config_tx) = &state.config_tx {
            config_tx.send_replace(Arc::new(candidate.clone()));
        }
        let standard = format!("{}/sub/{}", candidate.public_base_url(), generated.raw());
        let clash = format!("{standard}/clash");
        state.config = Some(candidate);
        state.last_error = None;
        Ok(NewSubscriptionUrls { standard, clash })
    }

    pub async fn reissue_certificate(self: &Arc<Self>) -> Result<()> {
        let config = self
            .state
            .lock()
            .await
            .config
            .clone()
            .context("subscription is not configured")?;
        if !config.enabled {
            bail!("subscription is disabled");
        }
        self.apply(config).await
    }

    pub async fn status(&self) -> SubscriptionStatus {
        let state = self.state.lock().await;
        match &state.config {
            Some(config) => SubscriptionStatus {
                enabled: config.enabled && state.listener.is_some(),
                public_host: config.public_host.clone(),
                port: config.port,
                certificate_not_after: state.certificate_not_after,
                masked_token: config.masked_token(),
                last_error: state.last_error.clone(),
            },
            None => SubscriptionStatus {
                enabled: false,
                public_host: String::new(),
                port: 0,
                certificate_not_after: None,
                masked_token: String::new(),
                last_error: state.last_error.clone(),
            },
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
        let stopped = match listener {
            Some(listener) => listener.shutdown().await,
            None => Ok(()),
        };
        let closed = match port {
            Some(port) => self.ops.close_port(port).await,
            None => Ok(()),
        };
        stopped?;
        closed
    }
}

async fn close_if_opened(ops: &dyn RuntimeOps, port: u16, opened: bool) -> Result<()> {
    if opened {
        ops.close_port(port).await
    } else {
        Ok(())
    }
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
) -> Result<()> {
    let rolled_back = promotion.rollback().await;
    let reloaded = reload_previous(listener, state);
    rolled_back?;
    reloaded
}
