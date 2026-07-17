# Aegis Secure Sensitive Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure audit-listed secrets are born with restrictive permissions and replaced atomically without following symlinks.

**Architecture:** A narrow Unix `secure_fs` module validates owner/type/mode and performs same-directory atomic replacement through an `O_DIRECTORY | O_NOFOLLOW` directory descriptor. Only encryption keys, Matrix credentials/store material, WARP account files, and Reality seed files use it.

**Tech Stack:** Rust 2024 stdlib Unix filesystem APIs, existing `libc`, Tokio `spawn_blocking` at async call sites.

## Global Constraints

- Private directories are mode `0700`; sensitive files are mode `0600` at creation.
- Reject symlinks, non-regular objects, and files not owned by effective UID.
- Temporary files and replacement operations are anchored to one validated directory file descriptor; path re-resolution is forbidden after validation.
- Sync file before rename and parent directory after rename.
- Failure preserves the previous destination and removes only the temporary file.
- Scope is limited to audit-listed sensitive files; no general persistence refactor.

---

### Task 1: Private Directory Validation

**Files:**
- Create: `rust/aegis/src/core/security/secure_fs.rs`
- Modify: `rust/aegis/src/core/security/mod.rs`

**Interfaces:**
- Produces: `open_private_dir(path: &Path) -> Result<File>`
- Internal primitives: `validate_at(dir: &File, name: &OsStr)`, `create_new_at`, `rename_at`, `unlink_at`, and `sync_directory`, implemented with existing `libc` `*at`/`fsync` calls and owned descriptors.

- [ ] **Step 1: Add Unix tests for birth mode, symlink rejection, regular-file rejection, and wrong-owner validation seam**

```rust
#[test]
fn private_directory_is_created_0700() {
    let root = test_dir();
    open_private_dir(&root).unwrap();
    assert_eq!(fs::symlink_metadata(&root).unwrap().mode() & 0o777, 0o700);
}

#[test]
fn private_directory_rejects_symlink() {
    let root = test_dir();
    let real = root.with_extension("real");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &root).unwrap();
    assert!(open_private_dir(&root).is_err());
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test core::security::secure_fs:: --all-features`
Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement directory creation and validation**

```rust
pub fn open_private_dir(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) => validate_dir_metadata(path, &meta)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().recursive(false).mode(0o700).create(path)?;
            validate_dir_metadata(path, &fs::symlink_metadata(path)?)?;
        }
        Err(e) => return Err(e.into()),
    }
    let dir = OpenOptions::new().read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    validate_dir_metadata(path, &dir.metadata()?)?;
    Ok(dir)
}

fn validate_dir_metadata(path: &Path, meta: &Metadata) -> Result<()> {
    if !meta.file_type().is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
        anyhow::bail!("private directory failed owner/type/mode validation: {}", path.display());
    }
    Ok(())
}
```

- [ ] **Step 4: Run focused tests and commit**

```bash
git add rust/aegis/src/core/security/secure_fs.rs rust/aegis/src/core/security/mod.rs
git commit -m "security: validate private filesystem locations"
```

### Task 2: Atomic Sensitive-File Replacement

**Files:**
- Modify: `rust/aegis/src/core/security/secure_fs.rs`

**Interfaces:**
- Produces: `atomic_write_sensitive(path: &Path, bytes: &[u8]) -> Result<()>`, internally anchored to the parent directory `File`.
- Internal seam: `atomic_write_sensitive_with(path, bytes, hooks)` for fault tests only.

- [ ] **Step 1: Add failing tests for `0600` birth mode, symlink destination, old-content preservation, and temporary cleanup**

```rust
#[test]
fn failed_replace_preserves_old_contents() {
    let path = private_test_path();
    atomic_write_sensitive(&path, b"old").unwrap();
    let err = atomic_write_sensitive_with(&path, b"new", TestHooks::fail_before_rename()).unwrap_err();
    assert!(err.to_string().contains("rename"));
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert!(temp_files_for(&path).is_empty());
}
```

- [ ] **Step 2: Verify RED with `cargo test core::security::secure_fs:: --all-features`**
- [ ] **Step 3: Implement the complete replacement protocol; wrap `openat`, `fstatat(AT_SYMLINK_NOFOLLOW)`, `renameat`, `unlinkat`, and `fsync` with checked return values and documented unsafe preconditions**

```rust
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
    if result.is_err() { let _ = unlink_at(&dir, &temp); }
    result
}
```

- [ ] **Step 4: Add a concurrent symlink-swap stress test; assert no outside file changes**
- [ ] **Step 5: Run focused tests and commit**

```bash
git add rust/aegis/src/core/security/secure_fs.rs
git commit -m "security: atomically replace sensitive files"
```

### Task 3: Migrate Audit-Listed Writers

**Files:**
- Modify: `rust/aegis/src/core/security/crypto.rs`
- Modify: `rust/aegis/src/main/matrix.rs`
- Modify: `rust/aegis/src/core/xray/installer.rs` (`warp::ACCOUNT_FILE` persistence)
- Inspect only: `rust/aegis/src/core/network/warp_api.rs` (produces account secrets but does not persist them)
- Modify: `rust/aegis/src/bootstrap.rs` (`PQ_SEED_PATH` and `PQ_PUB_PATH`)
- Modify: `rust/aegis/src/core/xray/reality.rs`

**Interfaces:**
- Consumes: `open_private_dir`, `atomic_write_sensitive`.
- Produces: no direct `fs::write` followed by `chmod` for audit-listed secrets.

- [ ] **Step 1: Add focused caller tests that set `umask(0)` and still observe `0600`/`0700`**
- [ ] **Step 2: Verify at least one caller test fails under current create-then-chmod behavior**
- [ ] **Step 3: Replace synchronous key creation**

```rust
if !key_path.exists() {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    atomic_write_sensitive(key_path, &key)?;
    key.zeroize();
}
```

- [ ] **Step 4: Wrap synchronous secure writes from async code**

```rust
let path = path.to_owned();
let bytes = bytes.to_vec();
tokio::task::spawn_blocking(move || atomic_write_sensitive(&path, &bytes)).await??;
```

- [ ] **Step 5: Confirm all audit-listed paths use the helper and no unrelated config writer changed**
- [ ] **Step 6: Run full gates**

Run: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`
Expected: PASS.

- [ ] **Step 7: Update only the sensitive-file audit finding and commit**

```bash
git add rust/aegis/src/core/security rust/aegis/src/main/matrix.rs rust/aegis/src/core/xray/installer.rs rust/aegis/src/core/xray/reality.rs rust/aegis/src/bootstrap.rs docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "security: harden sensitive file persistence"
```
