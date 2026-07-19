#![cfg(target_os = "linux")]

use aegis::core::system::upgrade_transaction::{publish_binary, rollback_binary, stage_binary};
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
    assert_eq!(
        std::fs::metadata(&published.backup).unwrap().ino(),
        old_inode
    );
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"new-binary");

    rollback_binary(&published).await.unwrap();
    assert_eq!(std::fs::metadata(&destination).unwrap().ino(), old_inode);
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"old-binary");
    assert!(!published.backup.exists());
}
