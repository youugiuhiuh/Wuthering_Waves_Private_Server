use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io;
use std::os::linux::fs::MetadataExt;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result};

pub fn open_private_dir(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) => validate_dir_metadata(path, &meta)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            DirBuilder::new()
                .recursive(false)
                .mode(0o700)
                .create(path)
                .with_context(|| format!("create private directory {}", path.display()))?;
            validate_dir_metadata(path, &fs::symlink_metadata(path)?)?;
        }
        Err(e) => return Err(e.into()),
    }
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .with_context(|| format!("open directory fd {}", path.display()))?;
    validate_dir_metadata(path, &dir.metadata()?)?;
    Ok(dir)
}

fn validate_dir_metadata(path: &Path, meta: &Metadata) -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    if !meta.file_type().is_dir()
        || meta.st_uid() != uid
        || meta.st_mode() & 0o777 != 0o700
    {
        anyhow::bail!(
            "private directory {}: owner/type/mode validation failed",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::linux::fs::MetadataExt;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn unique_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aegis_sfs_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        p
    }

    #[test]
    fn private_directory_is_created_0700() {
        let root = unique_dir();
        open_private_dir(&root).unwrap();
        let meta = fs::symlink_metadata(&root).unwrap();
        assert_eq!(meta.st_mode() & 0o777, 0o700);
    }

    #[test]
    fn private_directory_rejects_symlink() {
        let root = unique_dir();
        let real = root.with_extension("real");
        fs::create_dir(&real).unwrap();
        symlink(&real, &root).unwrap();
        let err = open_private_dir(&root).unwrap_err();
        assert!(err.to_string().contains("validation failed"));
    }

    #[test]
    fn open_private_dir_accepts_existing_0700() {
        let root = unique_dir();
        DirBuilder::new().recursive(false).mode(0o700).create(&root).unwrap();
        let _file = open_private_dir(&root).unwrap();
        let meta = fs::symlink_metadata(&root).unwrap();
        assert_eq!(meta.st_mode() & 0o777, 0o700);
    }

    #[test]
    fn existing_file_rejected() {
        let root = unique_dir();
        fs::write(&root, b"not a directory").unwrap();
        let err = open_private_dir(&root).unwrap_err();
        assert!(err.to_string().contains("validation failed"));
    }
}
