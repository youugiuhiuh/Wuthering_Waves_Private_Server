use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::linux::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};

#[allow(clippy::undocumented_unsafe_blocks)]
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

#[allow(clippy::undocumented_unsafe_blocks)]
fn validate_dir_metadata(path: &Path, meta: &Metadata) -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    if !meta.file_type().is_dir() || meta.st_uid() != uid || meta.st_mode() & 0o777 != 0o700 {
        anyhow::bail!(
            "private directory {}: owner/type/mode validation failed",
            path.display()
        );
    }
    Ok(())
}

pub fn atomic_write_sensitive(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("sensitive path has no parent")?;
    let dir = open_private_dir(parent)?;
    let name = path.file_name().context("sensitive path has no filename")?;
    validate_at(&dir, name)?;
    let temp = unique_temp_name(name);
    let result = (|| -> Result<()> {
        let mut file = create_new_at(&dir, &temp, 0o600)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        validate_at(&dir, name)?;
        rename_at(&dir, &temp, &dir, name)?;
        sync_directory(&dir)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = unlink_at(&dir, &temp);
    }
    result
}

fn unique_temp_name(name: &OsStr) -> OsString {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let mut s = name.to_os_string();
    s.push(format!(".{}.tmp", COUNTER.fetch_add(1, Ordering::Relaxed)));
    s
}

#[allow(clippy::undocumented_unsafe_blocks)]
fn validate_at(dir: &File, name: &OsStr) -> Result<()> {
    let cname = CString::new(name.as_bytes())
        .map_err(|_| anyhow::anyhow!("filename contains null byte"))?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            cname.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if ret == -1 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(err.into());
    }
    let uid = unsafe { libc::geteuid() };
    if st.st_uid != uid {
        anyhow::bail!("file not owned by current user");
    }
    if (st.st_mode & libc::S_IFMT) == libc::S_IFLNK {
        anyhow::bail!("destination is a symlink");
    }
    Ok(())
}

#[allow(clippy::undocumented_unsafe_blocks)]
fn create_new_at(dir: &File, name: &OsStr, mode: libc::mode_t) -> Result<File> {
    let cname = CString::new(name.as_bytes())
        .map_err(|_| anyhow::anyhow!("filename contains null byte"))?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            cname.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd == -1 {
        return Err(io::Error::last_os_error().into());
    }
    unsafe { Ok(File::from_raw_fd(fd)) }
}

#[allow(clippy::undocumented_unsafe_blocks)]
fn rename_at(old_dir: &File, old_name: &OsStr, new_dir: &File, new_name: &OsStr) -> Result<()> {
    let cold = CString::new(old_name.as_bytes())
        .map_err(|_| anyhow::anyhow!("filename contains null byte"))?;
    let cnew = CString::new(new_name.as_bytes())
        .map_err(|_| anyhow::anyhow!("filename contains null byte"))?;
    let ret = unsafe {
        libc::renameat(
            old_dir.as_raw_fd(),
            cold.as_ptr(),
            new_dir.as_raw_fd(),
            cnew.as_ptr(),
        )
    };
    if ret == -1 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[allow(clippy::undocumented_unsafe_blocks)]
fn unlink_at(dir: &File, name: &OsStr) -> Result<()> {
    let cname = CString::new(name.as_bytes())
        .map_err(|_| anyhow::anyhow!("filename contains null byte"))?;
    let ret = unsafe { libc::unlinkat(dir.as_raw_fd(), cname.as_ptr(), 0) };
    if ret == -1 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

fn sync_directory(dir: &File) -> Result<()> {
    dir.sync_all().context("sync directory failed")?;
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

    fn unique_path() -> PathBuf {
        let dir = unique_dir();
        dir.join("secret.bin")
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
        DirBuilder::new()
            .recursive(false)
            .mode(0o700)
            .create(&root)
            .unwrap();
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

    #[test]
    fn atomic_write_creates_file_0600() {
        let path = unique_path();
        atomic_write_sensitive(&path, b"secret-data").unwrap();
        let meta = fs::symlink_metadata(&path).unwrap();
        assert_eq!(meta.st_mode() & 0o777, 0o600);
        assert_eq!(fs::read(&path).unwrap(), b"secret-data");
    }

    #[test]
    fn atomic_write_rejects_symlink_dest() {
        let dir = unique_dir();
        let _fd = open_private_dir(&dir).unwrap();
        let link = dir.join("target");
        symlink("/dev/null", &link).unwrap();
        let err = atomic_write_sensitive(&link, b"data").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("symlink"),
            "expected symlink rejection, got: {msg}"
        );
    }

    #[test]
    fn atomic_write_replaces_content() {
        let path = unique_path();
        atomic_write_sensitive(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        atomic_write_sensitive(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }
}
