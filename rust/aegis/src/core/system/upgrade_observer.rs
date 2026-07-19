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

#[cfg(test)]
use crate::core::system::upgrade_transaction::{publish_binary, stage_binary};

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
    let parent = path
        .parent()
        .context("upgrade metadata path has no parent")?;
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
            let parent = path
                .parent()
                .context("upgrade metadata path has no parent")?
                .to_owned();
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
    write_json(
        Path::new(READY_FILE),
        &ObserverReady {
            nonce: nonce.clone(),
        },
    )
    .await?;
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
            let _ = remove_if_exists(Path::new(ACK_FILE)).await;
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
            MessageContent { text, markup: None },
        )
        .await?;

    for path in [RESULT_FILE, ACK_FILE, RECORD_FILE, READY_FILE] {
        remove_if_exists(Path::new(path)).await?;
    }
    remove_state_dir_if_empty().await
}

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
        assert!(
            error
                .to_string()
                .contains("post-exec acknowledgement timeout")
        );
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
            finish_candidate(&published, &missing_ack, "abc", "3.5.0", 2, Duration::ZERO,)
                .await
                .is_err()
        );
        rollback_binary(&published).await.unwrap();
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"old");
    }
}
