# Aegis Installer And Upgrade Rollback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Sing-box installation, wwps-core upgrade, and Aegis self-upgrade single-flight transactions that publish verified binaries atomically, prove bounded runtime health, and restore the previous binary and runtime on failure.

**Architecture:** Add one small filesystem transaction module for same-directory staging, backup, atomic publish, rollback, cleanup, combined errors, and per-component in-process single-flight. Sing-box and wwps-core run activation and bounded health checks in-process; Aegis delegates restart observation to a detached transient systemd unit running the old image, and the restarted process writes a post-exec acknowledgement only after integrity verification, configuration loading, and adapter construction succeed, before runtime gateway supervision starts.

**Tech Stack:** Rust 2024, Tokio, `anyhow`, `serde`, existing `run_cmd_checked`/`run_cmd_status`, Linux/systemd; no new dependency.

## Files Touched

- Create: `rust/aegis/src/core/system/upgrade_transaction.rs` - shared binary transaction and single-flight primitives.
- Modify: `rust/aegis/src/core/system/mod.rs` - register the shared module.
- Modify: `rust/aegis/src/core/singbox/installer.rs` - transactional Sing-box publish, activation, health, and rollback.
- Modify: `rust/aegis/src/core/system/core_upgrade.rs` - transactional wwps-core publish, restart, health, and rollback.
- Modify: `rust/aegis/src/core/system/upgrade.rs` - Aegis transaction record, detached observer, rollback, and result reporting.
- Create: `rust/aegis/src/core/system/upgrade_observer.rs` - detached old-image observer and post-exec handshake.
- Modify: `rust/aegis/src/main/cli.rs` - hidden observer CLI mode.
- Modify: `rust/aegis/src/main.rs` - post-exec acknowledgement/result gate before runtime starts.
- Create: `rust/aegis/tests/test_upgrade_transaction_linux.rs` - Linux same-filesystem hard-link/publish/restore integration proof.

## Global Constraints

- Existing download, origin, digest, Minisign, trusted-comment, asset-name, size, and archive validation code and tests remain unchanged.
- Each component has its own process-wide single-flight; a duplicate request returns a component-specific busy error and performs no download, filesystem, or service action.
- Candidate staging and backup are siblings of the destination binary, therefore on the destination filesystem. Backup uses a same-directory hard link; publish and restore use same-directory `rename`.
- Candidate files are mode `0755`, file-synced before publish, and the parent directory is synced after backup creation, publish, restore, and cleanup.
- A fixed stale backup blocks a new operation with an operator-recovery error. A stale staging file is removed before a new verified candidate is copied.
- Success removes staging, backup, and transaction metadata. Pre-publish failure removes staging and leaves the destination/runtime untouched. Post-publish failure atomically restores the backup (or removes a first-install destination), then restores and health-checks the prior runtime.
- Returned/logged failure text contains an `operation failure:` field; failed pre-publish cleanup additionally contains a `cleanup failure:` field, while failed restoration additionally contains a `rollback failure:` field. No secondary failure may replace or discard the original operation failure.
- Sing-box and wwps-core health use ten `systemctl is-active --quiet` attempts, one second apart, with a 5-second timeout per attempt.
- Aegis never sends success from the replacing process. `systemd-run` starts a detached old-image observer; `wwps-aegis.service` (`Restart=always`) starts the published image; only a matching post-exec acknowledgement lets the observer commit and the restarted process report success.
- Changes stay inside Phase 4. Do not redesign commands, release sources, service units, archive parsing, or add a generic deployment framework.

## Execution Order

Execute the numbered tasks in numeric order. Task 1 defines the interfaces consumed by Tasks 2-4; Task 5 is verification-only. The detailed sections are intentionally independent and may be reviewed separately.

---

### Task 1: Shared Binary Transaction Primitive

**Files:**
- Create: `rust/aegis/src/core/system/upgrade_transaction.rs`
- Modify: `rust/aegis/src/core/system/mod.rs`

**Interfaces:**
- Produces: `SingleFlight::new(component: &'static str)` and `SingleFlight::try_enter(&'static self) -> Result<SingleFlightGuard>`.
- Produces: `stage_binary(candidate: &Path, destination: &Path) -> Result<PathBuf>`.
- Produces: `backup_path(destination: &Path) -> Result<PathBuf>`.
- Produces: `publish_binary(staged: &Path, destination: &Path) -> Result<PublishedBinary>`.
- Produces: `rollback_binary(published: &PublishedBinary) -> Result<()>` and `commit_binary(published: &PublishedBinary) -> Result<()>`.
- Produces: `OperationRollbackError::new(operation, rollback)` preserving both `anyhow::Error` values.
- Produces: `OperationCleanupError::new(operation, cleanup)` preserving a failed operation and failed cleanup without relabeling cleanup as rollback.

- [ ] **Step 1: Register the module and add failing unit tests**

Append `pub mod upgrade_transaction;` to `src/core/system/mod.rs`. Create `src/core/system/upgrade_transaction.rs` with this test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    static TEST_FLIGHT: SingleFlight = SingleFlight::new("test-component");

    #[test]
    fn single_flight_rejects_overlap_and_releases_on_drop() {
        let first = TEST_FLIGHT.try_enter().unwrap();
        let error = TEST_FLIGHT.try_enter().unwrap_err();
        assert!(error.to_string().contains("test-component upgrade already in progress"));
        drop(first);
        TEST_FLIGHT.try_enter().unwrap();
    }

    #[tokio::test]
    async fn stage_is_sibling_and_replaces_stale_stage() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        tokio::fs::write(dir.path().join(".component.stage"), b"stale")
            .await
            .unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        assert_eq!(staged.parent(), destination.parent());
        assert_eq!(tokio::fs::read(staged).await.unwrap(), b"new");
    }

    #[tokio::test]
    async fn publication_failure_preserves_destination() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("component");
        tokio::fs::write(&destination, b"old").await.unwrap();
        let missing = dir.path().join("missing-stage");
        assert!(publish_binary(&missing, &destination).await.is_err());
        assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"old");
        assert!(!backup_path(&destination).unwrap().exists());
    }

    #[tokio::test]
    async fn rollback_restores_old_binary_atomically() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        tokio::fs::write(&destination, b"old").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        rollback_binary(&published).await.unwrap();
        assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"old");
        assert!(!published.backup.exists());
    }

    #[tokio::test]
    async fn first_install_rollback_removes_candidate() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        rollback_binary(&published).await.unwrap();
        assert!(!destination.exists());
    }

    #[test]
    fn combined_error_displays_original_and_rollback() {
        let error = OperationRollbackError::new(
            anyhow::anyhow!("health timeout"),
            anyhow::anyhow!("old service failed"),
        );
        let text = error.to_string();
        assert!(text.contains("operation failure: health timeout"));
        assert!(text.contains("rollback failure: old service failed"));
        assert_eq!(error.source().unwrap().to_string(), "health timeout");
    }

    #[test]
    fn publish_and_cleanup_errors_are_both_observable() {
        let error = operation_with_cleanup(
            anyhow::anyhow!("atomically publish candidate: rename denied"),
            Err(anyhow::anyhow!("remove rollback backup: unlink denied")),
        );
        let text = error.to_string();
        assert!(text.contains("operation failure: atomically publish candidate: rename denied"));
        assert!(text.contains("cleanup failure: remove rollback backup: unlink denied"));
        let combined = error.downcast_ref::<OperationCleanupError>().unwrap();
        assert_eq!(
            combined.source().unwrap().to_string(),
            "atomically publish candidate: rename denied"
        );
    }
}
```

- [ ] **Step 2: Run focused tests to verify RED**

Run from `rust/aegis`:

```bash
cargo test --lib core::system::upgrade_transaction::tests -- --test-threads=1
```

Expected: FAIL with unresolved `SingleFlight`, `stage_binary`, `publish_binary`, `rollback_binary`, `backup_path`, `OperationCleanupError`, and `OperationRollbackError`.

- [ ] **Step 3: Implement the minimal transaction primitive**

Place this code above the test module in `src/core/system/upgrade_transaction.rs`:

```rust
use anyhow::{Context, Result};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs;

#[derive(Debug)]
pub struct SingleFlight {
    component: &'static str,
    active: AtomicBool,
}

impl SingleFlight {
    pub const fn new(component: &'static str) -> Self {
        Self {
            component,
            active: AtomicBool::new(false),
        }
    }

    pub fn try_enter(&'static self) -> Result<SingleFlightGuard> {
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            anyhow::bail!("{} upgrade already in progress", self.component);
        }
        Ok(SingleFlightGuard { flight: self })
    }
}

#[derive(Debug)]
pub struct SingleFlightGuard {
    flight: &'static SingleFlight,
}

impl Drop for SingleFlightGuard {
    fn drop(&mut self) {
        self.flight.active.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone)]
pub struct PublishedBinary {
    pub destination: PathBuf,
    pub backup: PathBuf,
    pub had_original: bool,
}

#[derive(Debug)]
pub struct OperationRollbackError {
    operation: anyhow::Error,
    rollback: anyhow::Error,
}

impl OperationRollbackError {
    pub fn new(operation: anyhow::Error, rollback: anyhow::Error) -> Self {
        Self { operation, rollback }
    }
}

impl fmt::Display for OperationRollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "operation failure: {}; rollback failure: {}",
            self.operation, self.rollback
        )
    }
}

impl Error for OperationRollbackError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.operation.as_ref())
    }
}

#[derive(Debug)]
pub struct OperationCleanupError {
    operation: anyhow::Error,
    cleanup: anyhow::Error,
}

impl OperationCleanupError {
    pub fn new(operation: anyhow::Error, cleanup: anyhow::Error) -> Self {
        Self { operation, cleanup }
    }
}

impl fmt::Display for OperationCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "operation failure: {}; cleanup failure: {}",
            self.operation, self.cleanup
        )
    }
}

impl Error for OperationCleanupError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.operation.as_ref())
    }
}

fn operation_with_cleanup(operation: anyhow::Error, cleanup: Result<()>) -> anyhow::Error {
    match cleanup {
        Ok(()) => operation,
        Err(cleanup) => OperationCleanupError::new(operation, cleanup).into(),
    }
}

fn sibling_path(destination: &Path, suffix: &str) -> Result<PathBuf> {
    let parent = destination.parent().context("binary destination has no parent")?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .context("binary destination has no UTF-8 file name")?;
    Ok(parent.join(format!(".{name}.{suffix}")))
}

pub fn stage_path(destination: &Path) -> Result<PathBuf> {
    sibling_path(destination, "stage")
}

pub fn backup_path(destination: &Path) -> Result<PathBuf> {
    sibling_path(destination, "rollback")
}

async fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

async fn sync_parent(path: &Path) -> Result<()> {
    let parent = path.parent().context("binary path has no parent")?.to_owned();
    tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all())
        .await
        .context("join directory sync")??;
    Ok(())
}

pub async fn stage_binary(candidate: &Path, destination: &Path) -> Result<PathBuf> {
    let staged = stage_path(destination)?;
    remove_file_if_exists(&staged).await?;
    let result = async {
        fs::copy(candidate, &staged)
            .await
            .with_context(|| format!("stage {}", destination.display()))?;
        #[cfg(unix)]
        fs::set_permissions(
            &staged,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .await
        .context("set staged binary mode")?;
        fs::File::open(&staged).await?.sync_all().await?;
        sync_parent(&staged).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if let Err(error) = result {
        let _ = remove_file_if_exists(&staged).await;
        return Err(error);
    }
    Ok(staged)
}

pub async fn publish_binary(staged: &Path, destination: &Path) -> Result<PublishedBinary> {
    let backup = backup_path(destination)?;
    if fs::try_exists(&backup).await? {
        anyhow::bail!(
            "stale rollback backup requires operator recovery: {}",
            backup.display()
        );
    }
    let had_original = fs::try_exists(destination).await?;
    if had_original {
        fs::hard_link(destination, &backup)
            .await
            .context("create same-directory rollback backup")?;
        if let Err(error) = sync_parent(destination).await {
            let operation = error.context("sync rollback backup");
            let cleanup = remove_file_if_exists(&backup).await;
            return Err(operation_with_cleanup(operation, cleanup));
        }
    }
    if let Err(operation) = fs::rename(staged, destination).await {
        let operation = anyhow::Error::new(operation).context("atomically publish candidate");
        let cleanup = remove_file_if_exists(&backup).await;
        return Err(operation_with_cleanup(operation, cleanup));
    }
    let published = PublishedBinary {
        destination: destination.to_owned(),
        backup,
        had_original,
    };
    if let Err(operation) = sync_parent(destination).await {
        return match rollback_binary(&published).await {
            Ok(()) => Err(operation).context("sync published candidate; prior binary restored"),
            Err(rollback) => Err(OperationRollbackError::new(operation, rollback).into()),
        };
    }
    Ok(published)
}

pub async fn rollback_binary(published: &PublishedBinary) -> Result<()> {
    if published.had_original {
        fs::rename(&published.backup, &published.destination)
            .await
            .context("atomically restore rollback backup")?;
    } else {
        remove_file_if_exists(&published.destination).await?;
    }
    sync_parent(&published.destination).await
}

pub async fn commit_binary(published: &PublishedBinary) -> Result<()> {
    remove_file_if_exists(&published.backup).await?;
    sync_parent(&published.destination).await
}
```

Add the lifecycle wrapper below `commit_binary`; the separate rollback callbacks are required because a failed first install must stop the new service rather than restart a nonexistent old binary:

```rust
pub async fn activate_or_rollback<A, AF, H, HF, R, RF, RH, RHF>(
    published: &PublishedBinary,
    mut activate: A,
    mut health: H,
    mut restore_runtime: R,
    mut restored_health: RH,
) -> Result<()>
where
    A: FnMut() -> AF,
    AF: std::future::Future<Output = Result<()>>,
    H: FnMut() -> HF,
    HF: std::future::Future<Output = Result<()>>,
    R: FnMut(bool) -> RF,
    RF: std::future::Future<Output = Result<()>>,
    RH: FnMut(bool) -> RHF,
    RHF: std::future::Future<Output = Result<()>>,
{
    let operation = match activate().await {
        Ok(()) => health().await,
        Err(error) => Err(error),
    };
    if let Err(operation) = operation {
        let rollback = async {
            rollback_binary(published).await?;
            restore_runtime(published.had_original).await?;
            restored_health(published.had_original).await
        }
        .await;
        return match rollback {
            Ok(()) => Err(operation).context("upgrade failed and prior runtime was restored"),
            Err(rollback) => Err(OperationRollbackError::new(operation, rollback).into()),
        };
    }
    commit_binary(published).await
}
```

Add these tests to the same test module:

```rust
    #[tokio::test]
    async fn restart_failure_restores_old_binary() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        tokio::fs::write(&destination, b"old").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        let error = activate_or_rollback(
            &published,
            || async { anyhow::bail!("restart failed") },
            || async { Ok(()) },
            |_| async { Ok(()) },
            |_| async { Ok(()) },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("restart failed"));
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"old");
    }

    #[tokio::test]
    async fn health_failure_restores_old_binary() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        tokio::fs::write(&destination, b"old").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        activate_or_rollback(
            &published,
            || async { Ok(()) },
            || async { anyhow::bail!("health timeout") },
            |_| async { Ok(()) },
            |_| async { Ok(()) },
        )
        .await
        .unwrap_err();
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"old");
    }

    #[tokio::test]
    async fn rollback_failure_keeps_both_errors() {
        let dir = tempdir().unwrap();
        let candidate = dir.path().join("candidate");
        let destination = dir.path().join("component");
        tokio::fs::write(&candidate, b"new").await.unwrap();
        tokio::fs::write(&destination, b"old").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        let error = activate_or_rollback(
            &published,
            || async { anyhow::bail!("restart failed") },
            || async { Ok(()) },
            |_| async { anyhow::bail!("rollback restart failed") },
            |_| async { Ok(()) },
        )
        .await
        .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("operation failure: restart failed"));
        assert!(text.contains("rollback failure: rollback restart failed"));
    }
```

- [ ] **Step 4: Run focused tests to verify GREEN**

Run: `cargo test --lib core::system::upgrade_transaction::tests -- --test-threads=1`

Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
git add src/core/system/mod.rs src/core/system/upgrade_transaction.rs
git commit -m "feat: add atomic upgrade transaction primitive"
```

---


### Task 2: Transactional Sing-box Installation

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`

**Interfaces:**
- Consumes: Task 1 `SingleFlight`, `stage_binary`, `publish_binary`, and `activate_or_rollback`.
- Keeps unchanged: `fetch_release`, `download_verified_archive`, `extract_candidate`, `parse_singbox_version`, all trust constants, and all release/archive verification tests.
- Produces: `bounded_health(probe)`, `activate_service`, `restore_service`, and `verify_service_active`.

- [ ] **Step 1: Add failing single-flight and bounded-health tests**

Add to `installer.rs` tests:

```rust
    #[test]
    fn singbox_install_is_single_flight() {
        let first = SINGBOX_INSTALL.try_enter().unwrap();
        let error = SINGBOX_INSTALL.try_enter().unwrap_err();
        assert!(error.to_string().contains("sing-box upgrade already in progress"));
        drop(first);
        SINGBOX_INSTALL.try_enter().unwrap();
    }

    #[tokio::test]
    async fn singbox_health_retries_until_active() {
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probe_attempts = attempts.clone();
        bounded_health(move || {
            let attempt = probe_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if attempt < 2 {
                    anyhow::bail!("inactive")
                }
                Ok(())
            }
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn singbox_health_is_bounded() {
        let error = bounded_health(|| async { anyhow::bail!("inactive") })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("after 10 attempts"));
    }
```

- [ ] **Step 2: Run focused tests to verify RED**

```bash
cargo test --lib core::singbox::installer::tests::singbox_install_is_single_flight -- --exact --test-threads=1
cargo test --lib core::singbox::installer::tests::singbox_health -- --test-threads=1
```

Expected: FAIL because `SINGBOX_INSTALL` and `bounded_health` do not exist.

- [ ] **Step 3: Add single-flight and deterministic service helpers**

Add imports and constants:

```rust
use std::future::Future;
use std::time::Duration;
use crate::core::cmd_async::{run_cmd_checked, run_cmd_status};
use crate::core::system::upgrade_transaction::{
    SingleFlight, activate_or_rollback, publish_binary, stage_binary,
};

static SINGBOX_INSTALL: SingleFlight = SingleFlight::new("sing-box");
const SERVICE: &str = "wwps-box.service";
const HEALTH_ATTEMPTS: usize = 10;
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
```

Add before `impl SingBoxInstaller`:

```rust
async fn bounded_health<F, Fut>(mut probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut last = None;
    for attempt in 0..HEALTH_ATTEMPTS {
        match probe().await {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        if attempt + 1 < HEALTH_ATTEMPTS {
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    }
    let last = last.context("health probe did not run")?;
    Err(last).context(format!(
        "sing-box service unhealthy after {HEALTH_ATTEMPTS} attempts"
    ))
}
```

Replace `create_service`, `reload_service`, and `stop_service` with these exact helpers; retain the current `service_content` literal unchanged inside `write_service_file`:

```rust
    async fn write_service_file() -> Result<()> {
        if !Path::new("/run/systemd/system").exists() {
            anyhow::bail!("systemd is required for transactional Sing-box activation");
        }
        let service_content = r#"[Unit]
Description=WWPS-Box Service
After=network.target

[Service]
Type=simple
ExecStart=/etc/wwps/wwps-box/wwps-box run -C /etc/wwps/wwps-box/conf
Restart=always
RestartSec=5
LimitNOFILE=51200

[Install]
WantedBy=multi-user.target
"#;
        fs::write("/etc/systemd/system/wwps-box.service", service_content)
            .await
            .context("创建服务文件失败")
    }

    async fn activate_service() -> Result<()> {
        Self::write_service_file().await?;
        crate::core::singbox::SingBoxConfigManager::ensure_base_config()
            .await
            .context("创建基础配置失败")?;
        run_cmd_checked("systemctl", &["daemon-reload"], COMMAND_TIMEOUT).await?;
        run_cmd_checked("systemctl", &["enable", SERVICE], COMMAND_TIMEOUT).await?;
        run_cmd_checked("systemctl", &["restart", SERVICE], COMMAND_TIMEOUT).await?;
        Ok(())
    }

    async fn restore_service(had_original: bool) -> Result<()> {
        if had_original {
            run_cmd_checked("systemctl", &["restart", SERVICE], COMMAND_TIMEOUT).await?;
        } else {
            run_cmd_checked("systemctl", &["stop", SERVICE], COMMAND_TIMEOUT).await?;
            run_cmd_checked("systemctl", &["disable", SERVICE], COMMAND_TIMEOUT).await?;
            fs::remove_file("/etc/systemd/system/wwps-box.service")
                .await
                .context("remove first-install service during rollback")?;
            run_cmd_checked("systemctl", &["daemon-reload"], COMMAND_TIMEOUT).await?;
        }
        Ok(())
    }

    async fn verify_service_active() -> Result<()> {
        bounded_health(|| async {
            let status = run_cmd_status(
                "systemctl",
                &["is-active", "--quiet", SERVICE],
                COMMAND_TIMEOUT,
            )
            .await?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!("{SERVICE} is inactive")
            }
        })
        .await
    }

    async fn verify_prior_runtime(had_original: bool) -> Result<()> {
        if had_original {
            Self::verify_service_active().await
        } else {
            let status = run_cmd_status(
                "systemctl",
                &["is-active", "--quiet", SERVICE],
                COMMAND_TIMEOUT,
            )
            .await?;
            if status.success() {
                anyhow::bail!("first-install rollback left {SERVICE} active")
            }
            Ok(())
        }
    }
```

- [ ] **Step 4: Publish only after existing verification, then activate/health/rollback**

At the first line of `install`, acquire the guard:

```rust
        let _flight = SINGBOX_INSTALL.try_enter()?;
```

Replace the call to `deploy_candidate` and all service mutation through the end of `install` with:

```rust
        let published = Self::deploy_candidate(
            &candidate,
            &release,
            Path::new(singbox::BIN),
        )
        .await?;
        activate_or_rollback(
            &published,
            Self::activate_service,
            Self::verify_service_active,
            Self::restore_service,
            Self::verify_prior_runtime,
        )
        .await?;

        Self::cleanup_legacy_service().await?;
        Ok(())
```

Replace `deploy_candidate` with this version. The version execution and exact comparison are unchanged; only staging/publication delegates to Task 1:

```rust
    async fn deploy_candidate(
        candidate: &Path,
        release: &SingBoxRelease,
        dest: &Path,
    ) -> Result<crate::core::system::upgrade_transaction::PublishedBinary> {
        let output = tokio::process::Command::new(candidate)
            .arg("version")
            .output()
            .await
            .context("failed to execute sing-box version")?;
        let reported = parse_singbox_version(&output.stdout)?;
        if !output.status.success() || reported != release.version {
            anyhow::bail!("Sing-box candidate version mismatch");
        }

        let staged = stage_binary(candidate, dest).await?;
        publish_binary(&staged, dest).await
    }

    async fn cleanup_legacy_service() -> Result<()> {
        let old_service_path = Path::new("/etc/systemd/system/sing-box.service");
        if !fs::try_exists(old_service_path).await? {
            return Ok(());
        }
        run_cmd_checked(
            "systemctl",
            &["disable", "--now", "sing-box.service"],
            COMMAND_TIMEOUT,
        )
        .await?;
        fs::remove_file(old_service_path)
            .await
            .context("remove legacy sing-box service")?;
        run_cmd_checked("systemctl", &["daemon-reload"], COMMAND_TIMEOUT).await?;
        Ok(())
    }
```

Keep `restart_service()` public, but make it deterministic and health-gated:

```rust
    pub async fn restart_service() -> Result<()> {
        run_cmd_checked("systemctl", &["restart", SERVICE], COMMAND_TIMEOUT).await?;
        Self::verify_service_active().await
    }
```

Update `test_deploy_candidate_accepts_matching_version` after `published` is returned:

```rust
        let published = SingBoxInstaller::deploy_candidate(&candidate, &release, &target)
            .await
            .expect("deploy should succeed");
        assert_eq!(std::fs::read(&target).unwrap(), std::fs::read(&candidate).unwrap());
        assert_eq!(std::fs::read(&published.backup).unwrap(), b"old content");
        crate::core::system::upgrade_transaction::commit_binary(&published)
            .await
            .unwrap();
        assert!(!published.backup.exists());
```

- [ ] **Step 5: Run focused and unchanged release-verification tests to verify GREEN**

```bash
cargo test --lib core::singbox::installer::tests -- --test-threads=1
cargo test --lib core::network::release_api::tests
```

Expected: both commands PASS; all pre-existing Sing-box metadata, bounded download, SHA256, archive traversal/type/size, extraction, and version tests remain present and green.

- [ ] **Step 6: Commit**

```bash
git add src/core/singbox/installer.rs
git commit -m "fix: roll back failed Sing-box installation"
```

---



### Task 3: Transactional wwps-core Upgrade

**Files:**
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`

**Interfaces:**
- Consumes: Task 1 transaction interfaces.
- Keeps unchanged: fixed XTLS repository/asset selection, API digest plus `.dgst` three-way verification, download, and archive extraction.
- Produces: `deploy_core(unpack_dir) -> Result<PublishedBinary>`, bounded `verify_service_active`, and `restore_prior_runtime`.
- Produces: `TempPathGuard`, which removes partially downloaded files and partially extracted directories on every early return.

- [ ] **Step 1: Add failing single-flight, bounded-health, and backup-location tests**

Add to `core_upgrade.rs` tests:

```rust
    #[test]
    fn wwps_core_upgrade_is_single_flight() {
        let first = WWPS_CORE_UPGRADE.try_enter().unwrap();
        let error = WWPS_CORE_UPGRADE.try_enter().unwrap_err();
        assert!(error.to_string().contains("wwps-core upgrade already in progress"));
        drop(first);
        WWPS_CORE_UPGRADE.try_enter().unwrap();
    }

    #[tokio::test]
    async fn wwps_core_health_is_bounded() {
        let error = bounded_health(|| async { anyhow::bail!("inactive") })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("after 10 attempts"));
    }

    #[tokio::test]
    async fn deploy_core_keeps_backup_beside_binary_until_health_commit() {
        let tmp = tempdir().unwrap();
        let install = tmp.path().join("install");
        let unpack = tmp.path().join("unpack");
        tokio::fs::create_dir_all(&install).await.unwrap();
        tokio::fs::create_dir_all(&unpack).await.unwrap();
        tokio::fs::write(install.join("wwps-core"), b"old").await.unwrap();
        tokio::fs::write(unpack.join("xray"), b"new").await.unwrap();
        let manager = test_manager(&install, tmp.path());
        let published = manager.deploy_core(&unpack).await.unwrap();
        assert_eq!(published.backup.parent(), Some(install.as_path()));
        assert_eq!(tokio::fs::read(&published.backup).await.unwrap(), b"old");
        rollback_binary(&published).await.unwrap();
        assert_eq!(tokio::fs::read(install.join("wwps-core")).await.unwrap(), b"old");
    }

    #[tokio::test]
    async fn failed_extract_removes_partial_unpack_directory() {
        let tmp = tempdir().unwrap();
        let install = tmp.path().join("install");
        tokio::fs::create_dir_all(&install).await.unwrap();
        let manager = test_manager(&install, tmp.path());
        tokio::fs::create_dir_all(&manager.config.temp_dir).await.unwrap();
        let invalid = tmp.path().join("invalid.zip");
        tokio::fs::write(&invalid, b"not a zip").await.unwrap();
        assert!(manager.extract_archive(&invalid).await.is_err());
        assert_eq!(
            std::fs::read_dir(&manager.config.temp_dir).unwrap().count(),
            0
        );
    }
```

Add this complete test helper inside the test module:

```rust
    fn test_manager(install: &Path, root: &Path) -> WwpsCoreUpgradeManager {
        WwpsCoreUpgradeManager::new(WwpsCoreUpgradeConfig::new(
            "wwps-core",
            install.to_owned(),
            root.join("unused-backup"),
            root.join("temp"),
            CpuArch::Amd64,
        ))
        .unwrap()
    }
```

- [ ] **Step 2: Run focused tests to verify RED**

Run: `cargo test --lib core::system::core_upgrade::tests -- --test-threads=1`

Expected: FAIL because `WWPS_CORE_UPGRADE`, `bounded_health`, `deploy_core`, and imported `rollback_binary` do not exist.

- [ ] **Step 3: Add single-flight, bounded health, and transactional deployment**

Replace `chrono::Utc` usage only where it names transaction staging; retain it for the existing archival backup and temporary download names. Add:

```rust
use std::future::Future;
use crate::core::system::upgrade_transaction::{
    PublishedBinary, SingleFlight, activate_or_rollback, publish_binary, rollback_binary,
    stage_binary,
};

static WWPS_CORE_UPGRADE: SingleFlight = SingleFlight::new("wwps-core");
const HEALTH_ATTEMPTS: usize = 10;
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);
const HEALTH_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

struct TempPathGuard {
    path: PathBuf,
    is_dir: bool,
    armed: bool,
}

impl TempPathGuard {
    fn file(path: PathBuf) -> Self {
        Self { path, is_dir: false, armed: true }
    }

    fn directory(path: PathBuf) -> Self {
        Self { path, is_dir: true, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.is_dir {
            let _ = std::fs::remove_dir_all(&self.path);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn bounded_health<F, Fut>(mut probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut last = None;
    for attempt in 0..HEALTH_ATTEMPTS {
        match probe().await {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        if attempt + 1 < HEALTH_ATTEMPTS {
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    }
    Err(last.context("health probe did not run")?).context(format!(
        "wwps-core service unhealthy after {HEALTH_ATTEMPTS} attempts"
    ))
}
```

In `download_release`, add `let mut temp_cleanup = TempPathGuard::file(temp_file.clone());` immediately after `temp_file` is built. Immediately before `Ok(temp_file)`, add:

```rust
        temp_cleanup.disarm();
```

In `extract_archive`, add `let mut target_cleanup = TempPathGuard::directory(target.clone());` immediately after `create_dir_all(&target)` succeeds. Immediately before `Ok(target)`, add:

```rust
        target_cleanup.disarm();
```

These guards contain complete cleanup behavior; every existing `?` path before disarm removes the partial path without changing download, digest, or archive validation.

Delete `replace_core` and add:

```rust
    pub async fn deploy_core(&self, unpack_dir: &Path) -> Result<PublishedBinary> {
        let new_core = unpack_dir.join("xray");
        if !new_core.exists() {
            anyhow::bail!("解压目录中未找到 xray 可执行文件");
        }
        let destination = self.config.install_dir.join("wwps-core");
        let staged = stage_binary(&new_core, &destination).await?;
        publish_binary(&staged, &destination).await
    }
```

Replace `verify_service_active` with:

```rust
    pub async fn verify_service_active(&self) -> Result<()> {
        let unit = format!("{}.service", self.config.service_name);
        bounded_health(|| async {
            let status = run_cmd_status(
                "systemctl",
                &["is-active", "--quiet", &unit],
                HEALTH_COMMAND_TIMEOUT,
            )
            .await
            .context("执行 systemctl is-active 失败")?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!("{} 未在运行", unit)
            }
        })
        .await
    }

    async fn restore_prior_runtime(&self, had_original: bool) -> Result<()> {
        if !had_original {
            anyhow::bail!("wwps-core rollback has no prior binary");
        }
        self.restart_service().await
    }

    async fn verify_restored_runtime(&self, had_original: bool) -> Result<()> {
        if !had_original {
            anyhow::bail!("wwps-core rollback has no prior runtime");
        }
        self.verify_service_active().await
    }
```

- [ ] **Step 4: Make `run_upgrade` cleanup and rollback deterministic**

Acquire the guard as the first statement of `run_upgrade`, before sending a message or reading environment:

```rust
        let _flight = WWPS_CORE_UPGRADE.try_enter()?;
```

Replace the block beginning at the current `let unpack_dir = manager.extract_archive(&archive_path).await?;` and ending at the final `Ok(())` in `run_upgrade` with:

```rust
        let unpack_dir = match manager.extract_archive(&archive_path).await {
            Ok(path) => path,
            Err(error) => {
                manager.cleanup_paths(std::slice::from_ref(&archive_path)).await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_backing_up").to_string(),
                    markup: None,
                },
            )
            .await;
        let archival_backup = match manager.backup_current_core().await {
            Ok(path) => path,
            Err(error) => {
                manager.cleanup_paths(&[archive_path, unpack_dir]).await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_replacing").to_string(),
                    markup: None,
                },
            )
            .await;
        let published = match manager.deploy_core(&unpack_dir).await {
            Ok(value) => value,
            Err(error) => {
                manager
                    .cleanup_paths(&[archive_path, unpack_dir, archival_backup])
                    .await;
                return Err(error);
            }
        };

        let _ = adapter
            .edit_message(
                target,
                &status_msg_id,
                MessageContent {
                    text: t!("upgrade.core_restarting").to_string(),
                    markup: None,
                },
            )
            .await;
        let activation = activate_or_rollback(
            &published,
            || manager.restart_service(),
            || manager.verify_service_active(),
            |had_original| manager.restore_prior_runtime(had_original),
            |had_original| manager.verify_restored_runtime(had_original),
        )
        .await;
        manager
            .cleanup_paths(&[archive_path, unpack_dir])
            .await;
        if let Err(error) = activation {
            manager
                .cleanup_paths(std::slice::from_ref(&archival_backup))
                .await;
            return Err(error);
        }

        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!(
                        "upgrade.core_updated",
                        "0" => release.tag_name.as_str(),
                        "1" => archival_backup.display().to_string().as_str()
                    )
                    .to_string(),
                    markup: None,
                },
            )
            .await?;
        Ok(())
```

This retains the existing timestamped archival backup only on success for the existing user-visible contract. The transaction-critical rollback backup is always same-directory and is removed only by `commit_binary` after health.

- [ ] **Step 5: Run focused and unchanged release-verification tests to verify GREEN**

```bash
cargo test --lib core::system::core_upgrade::tests -- --test-threads=1
cargo test --lib core::network::release_api::tests
```

Expected: PASS; fixed XTLS paths, exact assets, API/`.dgst`/download three-way SHA256 equality, and failed-verification cleanup tests remain green.

- [ ] **Step 6: Commit**

```bash
git add src/core/system/core_upgrade.rs
git commit -m "fix: roll back failed wwps-core upgrades"
```

---





### Task 4: Aegis Detached Supervisor And Post-Exec Observation

**Files:**
- Create: `rust/aegis/src/core/system/upgrade_observer.rs`
- Modify: `rust/aegis/src/core/system/mod.rs`
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Modify: `rust/aegis/src/main/cli.rs`
- Modify: `rust/aegis/src/main.rs`

**Interfaces:**
- Consumes: Task 1 staging/publish/rollback/commit and `OperationRollbackError`.
- Keeps unchanged: Aegis fixed repository/asset, API digest, Minisign key/signature, trusted-comment, download, and verification code.
- Produces: `prepare_observer(new_version, target, destination) -> Result<UpgradeRecord>`, `observe(nonce, parent_pid) -> Result<()>`, `cancel_observer(record)`, and `post_exec_checkpoint(adapter) -> Result<()>`.
- Protocol: a transient `systemd-run` unit executes the already-running old image in a separate cgroup; the main `wwps-aegis.service` remains the restart supervisor; candidate/rollback processes acknowledge only after `verify_integrity`, config load, and adapter construction. This checkpoint precedes `main::runtime::run` and does not prove ongoing gateway liveness.

- [ ] **Step 1: Add failing protocol tests**

Create `upgrade_observer.rs` with the following tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn observer_command_is_detached_transient_systemd_unit() {
        let executable = Path::new("/etc/wwps/aegis/aegis");
        let args = observer_command(executable, "abc123", 42);
        assert_eq!(args[0], "--unit=wwps-aegis-upgrade-abc123");
        assert!(args.contains(&"--collect".to_string()));
        assert!(args.contains(&"--property=Type=exec".to_string()));
        assert!(args.contains(&"--observe-aegis-upgrade".to_string()));
        assert!(args.contains(&"42".to_string()));
    }

    #[tokio::test]
    async fn matching_post_exec_ack_is_accepted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ack.json");
        write_json(
            &path,
            &UpgradeAck {
                nonce: "abc".into(),
                version: "3.5.0".into(),
            },
        )
        .await
        .unwrap();
        wait_for_ack(&path, "abc", "3.5.0", 1, Duration::ZERO)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mismatched_ack_times_out_boundedly() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ack.json");
        write_json(
            &path,
            &UpgradeAck {
                nonce: "other".into(),
                version: "9.9.9".into(),
            },
        )
        .await
        .unwrap();
        let error = wait_for_ack(&path, "abc", "3.5.0", 3, Duration::ZERO)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("post-exec acknowledgement timeout"));
    }

    #[tokio::test]
    async fn candidate_success_commits_backup_only_after_ack() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("aegis");
        let candidate = dir.path().join("candidate");
        tokio::fs::write(&destination, b"old").await.unwrap();
        tokio::fs::write(&candidate, b"new").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        let ack = dir.path().join("ack.json");
        write_json(
            &ack,
            &UpgradeAck {
                nonce: "abc".into(),
                version: "3.5.0".into(),
            },
        )
        .await
        .unwrap();
        finish_candidate(&published, &ack, "abc", "3.5.0", 1, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"new");
        assert!(!published.backup.exists());
    }

    #[tokio::test]
    async fn candidate_timeout_restores_old_binary() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("aegis");
        let candidate = dir.path().join("candidate");
        tokio::fs::write(&destination, b"old").await.unwrap();
        tokio::fs::write(&candidate, b"new").await.unwrap();
        let staged = stage_binary(&candidate, &destination).await.unwrap();
        let published = publish_binary(&staged, &destination).await.unwrap();
        let missing_ack = dir.path().join("ack.json");
        assert!(
            finish_candidate(
                &published,
                &missing_ack,
                "abc",
                "3.5.0",
                2,
                Duration::ZERO,
            )
            .await
            .is_err()
        );
        rollback_binary(&published).await.unwrap();
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"old");
    }
}
```

- [ ] **Step 2: Run protocol tests to verify RED**

Run: `cargo test --lib core::system::upgrade_observer::tests -- --test-threads=1`

Expected: FAIL because the observer module and protocol types/functions do not exist.

- [ ] **Step 3: Define durable protocol records and atomic metadata I/O**

Add `pub mod upgrade_observer;` to `src/core/system/mod.rs`. Add this code above the tests in `upgrade_observer.rs`:

```rust
use crate::adapters::common::{BotAdapter, MessageContent, TargetId};
use crate::core::cmd_async::run_cmd_checked;
use crate::core::system::upgrade_transaction::{
    OperationRollbackError, PublishedBinary, backup_path, commit_binary, rollback_binary,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

const STATE_DIR: &str = "/etc/wwps/aegis/upgrade-transaction";
const RECORD_FILE: &str = "/etc/wwps/aegis/upgrade-transaction/record.json";
const ACK_FILE: &str = "/etc/wwps/aegis/upgrade-transaction/ack.json";
const READY_FILE: &str = "/etc/wwps/aegis/upgrade-transaction/observer-ready.json";
const RESULT_FILE: &str = "/etc/wwps/aegis/upgrade-transaction/result.json";
const SERVICE: &str = "wwps-aegis.service";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const OBSERVER_READY_ATTEMPTS: usize = 50;
const POST_EXEC_ATTEMPTS: usize = 30;
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UpgradePhase {
    Candidate,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeRecord {
    nonce: String,
    old_version: String,
    new_version: String,
    target: String,
    destination: PathBuf,
    backup: PathBuf,
    had_original: bool,
    phase: UpgradePhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpgradeAck {
    nonce: String,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObserverReady {
    nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpgradeResult {
    nonce: String,
    success: bool,
    version: String,
    error: Option<String>,
}

async fn ensure_state_dir() -> Result<()> {
    fs::create_dir_all(STATE_DIR).await?;
    #[cfg(unix)]
    fs::set_permissions(
        STATE_DIR,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .await?;
    Ok(())
}

async fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("upgrade metadata path has no parent")?;
    fs::create_dir_all(parent).await?;
    let staged = path.with_extension("json.new");
    let data = serde_json::to_vec(value)?;
    fs::write(&staged, data).await?;
    #[cfg(unix)]
    fs::set_permissions(
        &staged,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .await?;
    fs::File::open(&staged).await?.sync_all().await?;
    fs::rename(&staged, path).await?;
    let dir = parent.to_owned();
    tokio::task::spawn_blocking(move || std::fs::File::open(dir)?.sync_all()).await??;
    Ok(())
}

async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).await?).context("parse upgrade transaction metadata")
}

async fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => {
            let parent = path.parent().context("upgrade metadata path has no parent")?.to_owned();
            tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all()).await??;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn remove_state_dir_if_empty() -> Result<()> {
    match fs::remove_dir(STATE_DIR).await {
        Ok(()) => {
            let parent = Path::new(STATE_DIR)
                .parent()
                .context("upgrade state directory has no parent")?
                .to_owned();
            tokio::task::spawn_blocking(move || std::fs::File::open(parent)?.sync_all()).await??;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("remove completed upgrade transaction directory"),
    }
}
```

- [ ] **Step 4: Implement detached observer launch and bounded acknowledgement**

Add below the metadata helpers:

```rust
fn observer_command(executable: &Path, nonce: &str, parent_pid: u32) -> Vec<String> {
    vec![
        format!("--unit=wwps-aegis-upgrade-{nonce}"),
        "--collect".into(),
        "--property=Type=exec".into(),
        "--".into(),
        executable.display().to_string(),
        "--observe-aegis-upgrade".into(),
        nonce.into(),
        parent_pid.to_string(),
    ]
}

async fn wait_for_ack(
    path: &Path,
    nonce: &str,
    version: &str,
    attempts: usize,
    interval: Duration,
) -> Result<()> {
    for attempt in 0..attempts {
        if let Ok(ack) = read_json::<UpgradeAck>(path).await
            && ack.nonce == nonce
            && ack.version == version
        {
            return Ok(());
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(interval).await;
        }
    }
    anyhow::bail!("post-exec acknowledgement timeout for version {version}")
}

async fn wait_for_ready(nonce: &str) -> Result<()> {
    for attempt in 0..OBSERVER_READY_ATTEMPTS {
        if let Ok(ready) = read_json::<ObserverReady>(Path::new(READY_FILE)).await
            && ready.nonce == nonce
        {
            return Ok(());
        }
        if attempt + 1 < OBSERVER_READY_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    anyhow::bail!("detached upgrade observer did not become ready")
}

async fn finish_candidate(
    published: &PublishedBinary,
    ack_path: &Path,
    nonce: &str,
    version: &str,
    attempts: usize,
    interval: Duration,
) -> Result<()> {
    wait_for_ack(ack_path, nonce, version, attempts, interval).await?;
    commit_binary(published).await
}

pub async fn prepare_observer(
    new_version: &str,
    target: &TargetId,
    destination: &Path,
) -> Result<UpgradeRecord> {
    ensure_state_dir().await?;
    if fs::try_exists(RECORD_FILE).await? {
        anyhow::bail!("stale Aegis upgrade transaction requires operator recovery");
    }
    for path in [ACK_FILE, READY_FILE, RESULT_FILE] {
        remove_if_exists(Path::new(path)).await?;
    }
    let nonce = hex::encode(rand::random::<[u8; 16]>());
    let record = UpgradeRecord {
        nonce: nonce.clone(),
        old_version: env!("CARGO_PKG_VERSION").to_string(),
        new_version: new_version.trim_start_matches('v').to_string(),
        target: target.0.clone(),
        destination: destination.to_owned(),
        backup: backup_path(destination)?,
        had_original: fs::try_exists(destination).await?,
        phase: UpgradePhase::Candidate,
    };
    write_json(Path::new(RECORD_FILE), &record).await?;

    let executable = std::env::current_exe()?;
    let args = observer_command(&executable, &nonce, std::process::id());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Err(error) = run_cmd_checked("systemd-run", &refs, COMMAND_TIMEOUT).await {
        cancel_observer(&record).await;
        return Err(error).context("launch detached Aegis upgrade observer");
    }
    if let Err(error) = wait_for_ready(&nonce).await {
        cancel_observer(&record).await;
        return Err(error);
    }
    Ok(record)
}

pub async fn cancel_observer(_record: &UpgradeRecord) {
    for path in [RECORD_FILE, ACK_FILE, READY_FILE, RESULT_FILE] {
        let _ = remove_if_exists(Path::new(path)).await;
    }
    let _ = remove_state_dir_if_empty().await;
}
```

- [ ] **Step 5: Implement old-image observation, rollback, and combined result**

Add:

```rust
#[cfg(target_os = "linux")]
fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 performs existence/permission checking and sends no signal.
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(not(target_os = "linux"))]
fn process_exists(_pid: u32) -> bool {
    false
}

async fn wait_for_parent_exit(parent_pid: u32) -> Result<()> {
    for attempt in 0..150 {
        if !process_exists(parent_pid) {
            return Ok(());
        }
        if !fs::try_exists(RECORD_FILE).await? {
            anyhow::bail!("upgrade transaction cancelled");
        }
        if attempt + 1 < 150 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    anyhow::bail!("replacing Aegis process did not exit")
}

pub async fn observe(nonce: String, parent_pid: u32) -> Result<()> {
    let mut record: UpgradeRecord = read_json(Path::new(RECORD_FILE)).await?;
    if record.nonce != nonce || record.phase != UpgradePhase::Candidate {
        anyhow::bail!("upgrade observer nonce or phase mismatch");
    }
    write_json(Path::new(READY_FILE), &ObserverReady { nonce: nonce.clone() }).await?;
    wait_for_parent_exit(parent_pid).await?;

    let published = PublishedBinary {
        destination: record.destination.clone(),
        backup: record.backup.clone(),
        had_original: record.had_original,
    };
    let candidate = finish_candidate(
        &published,
        Path::new(ACK_FILE),
        &nonce,
        &record.new_version,
        POST_EXEC_ATTEMPTS,
        POLL_INTERVAL,
    )
    .await;
    let result = match candidate {
        Ok(()) => UpgradeResult {
            nonce: nonce.clone(),
            success: true,
            version: record.new_version.clone(),
            error: None,
        },
        Err(operation) => {
            remove_if_exists(Path::new(ACK_FILE)).await?;
            let rollback = async {
                rollback_binary(&published).await?;
                record.phase = UpgradePhase::Rollback;
                write_json(Path::new(RECORD_FILE), &record).await?;
                run_cmd_checked("systemctl", &["restart", SERVICE], COMMAND_TIMEOUT).await?;
                wait_for_ack(
                    Path::new(ACK_FILE),
                    &nonce,
                    &record.old_version,
                    POST_EXEC_ATTEMPTS,
                    POLL_INTERVAL,
                )
                .await
            }
            .await;
            let error = match rollback {
                Ok(()) => format!("operation failure: {operation}"),
                Err(rollback) => OperationRollbackError::new(operation, rollback).to_string(),
            };
            UpgradeResult {
                nonce: nonce.clone(),
                success: false,
                version: record.old_version.clone(),
                error: Some(error),
            }
        }
    };
    write_json(Path::new(RESULT_FILE), &result).await?;
    remove_if_exists(Path::new(READY_FILE)).await?;
    Ok(())
}
```

- [ ] **Step 6: Implement restarted-process acknowledgement and result reporting**

Add to `upgrade_observer.rs`:

```rust
async fn wait_for_result(nonce: &str) -> Result<UpgradeResult> {
    for attempt in 0..45 {
        if let Ok(result) = read_json::<UpgradeResult>(Path::new(RESULT_FILE)).await
            && result.nonce == nonce
        {
            return Ok(result);
        }
        if attempt + 1 < 45 {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
    anyhow::bail!("upgrade observer result timeout")
}

pub async fn post_exec_checkpoint(adapter: &dyn BotAdapter) -> Result<()> {
    let record = match read_json::<UpgradeRecord>(Path::new(RECORD_FILE)).await {
        Ok(record) => record,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let running_version = env!("CARGO_PKG_VERSION");
    let expected_version = match record.phase {
        UpgradePhase::Candidate => &record.new_version,
        UpgradePhase::Rollback => &record.old_version,
    };
    if running_version != expected_version {
        return Ok(());
    }

    write_json(
        Path::new(ACK_FILE),
        &UpgradeAck {
            nonce: record.nonce.clone(),
            version: running_version.to_string(),
        },
    )
    .await?;
    let result = wait_for_result(&record.nonce).await?;
    let text = if result.success {
        rust_i18n::t!("system.upgrade_success", "0" => result.version.as_str()).to_string()
    } else {
        format!(
            "Aegis upgrade failed and rollback was attempted: {}",
            result.error.as_deref().unwrap_or("unknown failure")
        )
    };
    adapter
        .send_message(
            &TargetId(record.target),
            MessageContent {
                text,
                markup: None,
            },
        )
        .await?;

    for path in [RESULT_FILE, ACK_FILE, RECORD_FILE, READY_FILE] {
        remove_if_exists(Path::new(path)).await?;
    }
    remove_state_dir_if_empty().await
}
```

The acknowledgement is deliberately after adapter construction. It proves the new executable passed integrity verification, configuration decryption/validation, and adapter construction. It occurs before `main::runtime::run`, so it does not claim gateway liveness or that the replacing process observed its own restart.

- [ ] **Step 7: Wire the hidden observer CLI before normal startup**

Extend `CliMode` in `src/main/cli.rs`:

```rust
    ObserveAegisUpgrade {
        nonce: String,
        parent_pid: u32,
    },
```

Add this `try_cli_mode` arm before `_ => None`:

```rust
        "--observe-aegis-upgrade" if args.len() == 4 => {
            match args[3].parse::<u32>() {
                Ok(parent_pid) => Some(CliMode::ObserveAegisUpgrade {
                    nonce: args[2].clone(),
                    parent_pid,
                }),
                Err(_) => Some(CliMode::Stdout(
                    "invalid Aegis upgrade observer parent pid".to_string(),
                )),
            }
        }
```

Add this `execute_cli_mode` arm:

```rust
        CliMode::ObserveAegisUpgrade { nonce, parent_pid } => {
            aegis::core::system::upgrade_observer::observe(nonce, parent_pid).await
        }
```

Add tests to `main/cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hidden_upgrade_observer_mode() {
        let args = vec![
            "aegis".to_string(),
            "--observe-aegis-upgrade".to_string(),
            "abc".to_string(),
            "42".to_string(),
        ];
        assert!(matches!(
            try_cli_mode(&args),
            Some(CliMode::ObserveAegisUpgrade { nonce, parent_pid })
                if nonce == "abc" && parent_pid == 42
        ));
    }
}
```

- [ ] **Step 8: Replace self-replacement with staged publish plus observer handoff**

In `upgrade.rs`, remove `tokio::task`, `self_replace`, `write_upgrade_flag`, and the success send from `finalize_install`. Add imports/static:

```rust
use crate::core::system::upgrade_observer::{cancel_observer, prepare_observer};
use crate::core::system::upgrade_transaction::{
    SingleFlight, publish_binary, stage_binary, stage_path,
};

static AEGIS_UPGRADE: SingleFlight = SingleFlight::new("aegis");
```

Acquire the guard as the first statement of `UpgradeManager::run`:

```rust
        let _flight = AEGIS_UPGRADE.try_enter()?;
```

Replace `finalize_install` with:

```rust
    async fn finalize_install(
        &self,
        artifact: &ReleaseArtifact,
        update_path: &Path,
        adapter: &dyn BotAdapter,
        target: &TargetId,
        progress_msg_id: &AegisMsgId,
    ) -> Result<()> {
        let _ = adapter
            .edit_message(
                target,
                progress_msg_id,
                MessageContent {
                    text: t!("upgrade.bot_replacing").to_string(),
                    markup: None,
                },
            )
            .await;

        let destination = std::env::current_exe().context("无法获取当前可执行文件路径")?;
        let staged = stage_binary(update_path, &destination).await?;
        if let Err(error) = fs::remove_file(update_path).await {
            let _ = fs::remove_file(&staged).await;
            return Err(error).context("清理下载更新文件失败");
        }
        let record = match prepare_observer(&artifact.tag_name, target, &destination).await {
            Ok(record) => record,
            Err(error) => {
                let _ = fs::remove_file(&staged).await;
                return Err(error);
            }
        };
        if let Err(error) = publish_binary(&staged, &destination).await {
            cancel_observer(&record).await;
            let _ = fs::remove_file(stage_path(&destination)?).await;
            return Err(error);
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
        std::process::exit(0);
    }
```

Do not call `write_upgrade_flag`, do not send `upgrade.bot_updated`, and do not return success before `exit(0)`. Existing `run` error handling remains unchanged for every pre-publish failure.

- [ ] **Step 9: Gate normal runtime on the restarted process checkpoint**

In `src/main.rs`, remove `use aegis::core::system::upgrade::UPGRADE_FLAG_FILE;` and delete the unused `notify_upgrade_success` function. Immediately after adapter construction (after line 93 in the current file), add:

```rust
    aegis::core::system::upgrade_observer::post_exec_checkpoint(adapter.as_ref()).await?;
```

This line must remain after the optional Matrix/Discord setup calls return and adapter construction completes, and before `AppState::new` and `main::runtime::run`. The protocol treats this only as an initialization checkpoint, not as proof that any gateway remains healthy.

Add this test to `upgrade.rs` tests:

```rust
    #[test]
    fn aegis_upgrade_is_single_flight() {
        let first = AEGIS_UPGRADE.try_enter().unwrap();
        let error = AEGIS_UPGRADE.try_enter().unwrap_err();
        assert!(error.to_string().contains("aegis upgrade already in progress"));
        drop(first);
        AEGIS_UPGRADE.try_enter().unwrap();
    }
```

- [ ] **Step 10: Run protocol and unchanged release-verification tests to verify GREEN**

```bash
cargo test --lib core::system::upgrade_observer::tests -- --test-threads=1
cargo test --lib core::system::upgrade::tests -- --test-threads=1
cargo test --bin aegis main::cli::tests -- --test-threads=1
cargo test --lib core::network::release_api::tests
```

Expected: all commands PASS. The existing Aegis fixed repository/asset, SHA256, Minisign, exact trusted-comment, and failed-signature cleanup tests remain unchanged and green. No test observes a success message from the replacing process.

- [ ] **Step 11: Commit**

```bash
git add src/core/system/mod.rs src/core/system/upgrade_observer.rs src/core/system/upgrade.rs src/main/cli.rs src/main.rs
git commit -m "fix: supervise Aegis self-upgrade rollback"
```

---



### Task 5: Linux Filesystem Integration And Full Phase Verification

**Files:**
- Create: `rust/aegis/tests/test_upgrade_transaction_linux.rs`

**Interfaces:**
- Consumes: Task 1 public filesystem transaction API.
- Proves: on Linux, the rollback backup is a same-filesystem hard link and atomic restore returns the original inode/content.

- [ ] **Step 1: Add the Linux integration test**

```rust
#![cfg(target_os = "linux")]

use aegis::core::system::upgrade_transaction::{
    publish_binary, rollback_binary, stage_binary,
};
use std::os::unix::fs::MetadataExt;

#[tokio::test]
async fn same_directory_backup_and_atomic_restore_preserve_old_inode() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("component");
    let candidate = dir.path().join("candidate");
    tokio::fs::write(&destination, b"old-binary").await.unwrap();
    tokio::fs::write(&candidate, b"new-binary").await.unwrap();
    let old_inode = std::fs::metadata(&destination).unwrap().ino();

    let staged = stage_binary(&candidate, &destination).await.unwrap();
    let published = publish_binary(&staged, &destination).await.unwrap();
    assert_eq!(published.backup.parent(), destination.parent());
    assert_eq!(std::fs::metadata(&published.backup).unwrap().ino(), old_inode);
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"new-binary");

    rollback_binary(&published).await.unwrap();
    assert_eq!(std::fs::metadata(&destination).unwrap().ino(), old_inode);
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"old-binary");
    assert!(!published.backup.exists());
}
```

- [ ] **Step 2: Run the Linux integration test**

Run: `cargo test --test test_upgrade_transaction_linux -- --test-threads=1`

Expected on Linux: PASS, 1 test. On non-Linux: 0 tests, PASS.

- [ ] **Step 3: Run every Phase 4 focused suite together**

```bash
cargo test --lib core::system::upgrade_transaction::tests -- --test-threads=1
cargo test --lib core::singbox::installer::tests -- --test-threads=1
cargo test --lib core::system::core_upgrade::tests -- --test-threads=1
cargo test --lib core::system::upgrade_observer::tests -- --test-threads=1
cargo test --lib core::system::upgrade::tests -- --test-threads=1
cargo test --test test_upgrade_transaction_linux -- --test-threads=1
```

Expected: all PASS. Confirm the output includes single-flight rejection, publication preservation, restart rollback, health rollback, combined rollback error, first-install cleanup, detached observer, post-exec timeout, and same-directory inode tests.

- [ ] **Step 4: Re-run the unchanged verification boundary**

```bash
cargo test --lib core::network::release_api::tests
cargo test --lib core::crypto::minisign::tests
```

Expected: PASS. Diff inspection shows no altered repository owner, API path, asset name, URL policy, digest parser, Minisign public key, trusted-comment parser, size bound, or archive validation rule.

- [ ] **Step 5: Run mandatory Rust gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features -- --test-threads=1
```

Expected: all commands exit 0 with no warnings and no failed tests.

- [ ] **Step 6: Inspect the final diff for protocol invariants**

Run:

```bash
git diff --check
git diff -- src/core/system/upgrade_transaction.rs src/core/singbox/installer.rs src/core/system/core_upgrade.rs src/core/system/upgrade_observer.rs src/core/system/upgrade.rs src/main/cli.rs src/main.rs tests/test_upgrade_transaction_linux.rs
```

Expected: `git diff --check` exits 0. The diff contains one static flight per component; `.stage` and `.rollback` siblings; `hard_link` backup; `rename` publish/restore; bounded health loops; combined operation/rollback errors; cleanup on every return path; `systemd-run` observer readiness before Aegis publish; no pre-restart Aegis success message; and unchanged release verification.

- [ ] **Step 7: Commit**

```bash
git add tests/test_upgrade_transaction_linux.rs
git commit -m "test: verify Linux upgrade rollback filesystem protocol"
```

---
## Required Review Gates

- Specification compliance: map every approved-design sentence to Tasks 1-5; Critical or Important gaps block completion.
- Code quality: verify no lock is held across unrelated bot/network work, every child command uses existing bounded command helpers, metadata permissions are `0700`/`0600`, and errors retain both source chains.
- Aegis protocol review: verify the observer runs in a separate transient systemd unit, is ready before publish, and only the restarted process reports a result after a matching version/nonce acknowledgement.
- Cleanup review: success leaves only the active binary plus wwps-core's pre-existing archival backup; rollback leaves the restored binary; unresolved host-level interruption leaves the fixed `.rollback` file and blocks a new operation for deterministic operator recovery.
