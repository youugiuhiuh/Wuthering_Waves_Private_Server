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
        Self {
            operation,
            rollback,
        }
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
    let parent = destination
        .parent()
        .context("binary destination has no parent")?;
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
    let parent = path
        .parent()
        .context("binary path has no parent")?
        .to_owned();
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
            Ok(()) => Err(anyhow::anyhow!(
                "upgrade failed and prior runtime was restored: {operation}"
            )),
            Err(rollback) => Err(OperationRollbackError::new(operation, rollback).into()),
        };
    }
    commit_binary(published).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    static TEST_FLIGHT: SingleFlight = SingleFlight::new("test-component");

    #[test]
    fn single_flight_rejects_overlap_and_releases_on_drop() {
        let first = TEST_FLIGHT.try_enter().unwrap();
        let error = TEST_FLIGHT.try_enter().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("test-component upgrade already in progress")
        );
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
}
