# Configuration Transactions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serialize encrypted-config mutations under a single in-process mutex, so concurrent read-modify-write cycles from `bootstrap.rs`, `dispatch.rs`, `state_ops.rs`, and `matrix.rs` cannot overwrite each other's changes.

**Architecture:** Add a static `std::sync::Mutex<()>` guarding one `update_config` sync function. The three existing mutation functions (`save_lang_to_config`, `save_self_destruct_key_hash_to_config`, `clear_matrix_recovery_key`) delegate to `update_config`. Callers move in-memory state updates *after* the successful disk write.

**Tech Stack:** Rust, `std::sync::Mutex`, `serde`, `atomic_write_sensitive`

**Files touched:**
- Modify: `src/bootstrap.rs`
- Modify: `src/shared/dispatch.rs`
- Modify: `src/shared/state_ops.rs`
- Test target: `src/bootstrap.rs` (tests module)

## Global Constraints

- Only successful persistence may publish in-memory state (order: disk first, then in-memory).
- Parse, decrypt, and validation failures abort; no default config substituted.
- Locks are not held across unrelated network or bot operations.
- Security material is absent from errors and logs.
- Changes stay inside the named boundary. No generic framework, compatibility bypass, unrelated refactor, or speculative dependency.

---

### Task 1: Add `update_config` + mutex

**Files:**
- Modify: `src/bootstrap.rs`

**Interfaces:**
- Produces: `fn update_config<F: FnOnce(&mut EncryptedConfig) -> Result<()>>(f: F) -> Result<()>`

- [ ] **Step 1: Write the failing test — concurrent mutations preserve both (thread)**

```rust
#[test]
fn concurrent_mutations_preserve_both_updates() {
    let dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
    }
    let config_dir = dir.path();
    let key = [0u8; 32];
    fs::write(config_dir.join(".key"), key).unwrap();
    let init = EncryptedConfig {
        token: b"t".to_vec(),
        admin_id: b"0".to_vec(),
        totp_secret: b"s".to_vec(),
        self_destruct_key_hash: None,
        matrix_homeserver: None,
        matrix_username: None,
        matrix_password: None,
        matrix_room_id: None,
        matrix_store_passphrase: None,
        lang: None,
        discord_token: None,
        discord_admin_id: None,
        matrix_recovery_key: None,
    };
    fs::write(
        config_dir.join("config.enc"),
        serde_json::to_vec(&init).unwrap(),
    )
    .unwrap();

    let t1 = std::thread::spawn(|| {
        save_lang_to_config(i18n::Lang::En).unwrap();
    });
    let t2 = std::thread::spawn(|| {
        save_self_destruct_key_hash_to_config(Some("a".repeat(64))).unwrap();
    });
    t1.join().unwrap();
    t2.join().unwrap();

    let data = fs::read(config_dir.join("config.enc")).unwrap();
    let config: EncryptedConfig = serde_json::from_slice(&data).unwrap();
    assert_eq!(config.lang, Some("en".to_string()));
    assert_eq!(
        config.self_destruct_key_hash,
        Some("a".repeat(64))
    );
}
```

- [ ] **Step 2: Run test to verify it fails** — without mutex, the two threads race and one overwrites the other's change

Run: `cargo test --lib bootstrap::tests::concurrent_mutations_preserve_both_updates -- --test-threads=1 2>&1`
Expected: FAIL (not deterministic, but the race occasionally loses one update)

- [ ] **Step 3: Write minimal implementation**

Add after the `use` block (around line 16):

```rust
use std::sync::Mutex;
use std::sync::LazyLock;
```

Add before `save_lang_to_config` (around line 338):

```rust
static CONFIG_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Apply a mutation to the encrypted config file under a lock.
/// The mutation function receives `&mut EncryptedConfig` and should modify it.
/// On success the complete value is validated and atomically written.
fn update_config<F>(f: F) -> Result<()>
where
    F: FnOnce(&mut EncryptedConfig) -> Result<()>,
{
    let _lock = CONFIG_MUTEX.lock().unwrap();
    let config_dir = config_dir();
    let path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&path).context("读取 config.enc 失败")?;
    let mut config: EncryptedConfig =
        serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    f(&mut config)?;
    atomic_write_sensitive(
        &path,
        &serde_json::to_vec(&config).context("序列化 config.enc 失败")?,
    )?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib bootstrap::tests::concurrent_mutations_preserve_both_updates -- --test-threads=1 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/bootstrap.rs
git commit -m "feat: add update_config with mutex guard"
```

---

### Task 2: Replace existing mutation functions

**Files:**
- Modify: `src/bootstrap.rs`

- [ ] **Step 1: Rewrite `save_lang_to_config`, `save_self_destruct_key_hash_to_config`, `clear_matrix_recovery_key`**

Replace:

```rust
#[allow(dead_code)]
pub fn save_lang_to_config(lang: i18n::Lang) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.lang = Some(lang.as_str().to_string());
    atomic_write_sensitive(&path, &serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}
```

With:

```rust
#[allow(dead_code)]
pub fn save_lang_to_config(lang: i18n::Lang) -> Result<()> {
    let _security = SecurityManager::new(&config_dir().join(KEY_FILE))?;
    update_config(|config| {
        config.lang = Some(lang.as_str().to_string());
        Ok(())
    })
}
```

Replace:

```rust
#[allow(dead_code)]
pub fn save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.self_destruct_key_hash = hash;
    atomic_write_sensitive(&path, &serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}
```

With:

```rust
#[allow(dead_code)]
pub fn save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()> {
    let _security = SecurityManager::new(&config_dir().join(KEY_FILE))?;
    update_config(|config| {
        config.self_destruct_key_hash = hash;
        Ok(())
    })
}
```

Replace:

```rust
pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()> {
    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig = serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    enc.matrix_recovery_key = None;
    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;
    atomic_write_sensitive(&config_path, &new_data)?;
    println!("✅ 恢复密钥已从配置中清除（用完即焚）");
    Ok(())
}
```

With:

```rust
pub fn clear_matrix_recovery_key() -> Result<()> {
    update_config(|config| {
        config.matrix_recovery_key = None;
        Ok(())
    })?;
    println!("✅ 恢复密钥已从配置中清除（用完即焚）");
    Ok(())
}
```

Note: `clear_matrix_recovery_key` no longer takes a `config_dir` argument since `update_config` uses `config_dir()` internally.

- [ ] **Step 2: Fix caller in `main/matrix.rs`**

In `src/main/matrix.rs`, change:

```rust
crate::bootstrap::clear_matrix_recovery_key(config_dir)?;
```

To:

```rust
crate::bootstrap::clear_matrix_recovery_key()?;
```

- [ ] **Step 3: Run existing tests**

Run: `cargo test --lib bootstrap::tests 2>&1`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/bootstrap.rs src/main/matrix.rs
git commit -m "refactor: delegate config mutations to update_config"
```

---

### Task 3: Fix caller ordering — in-memory after disk

**Files:**
- Modify: `src/shared/dispatch.rs`
- Modify: `src/shared/state_ops.rs`

- [ ] **Step 1: Fix `dispatch.rs`** — move `state.set_self_destruct_key_hash` after successful disk write

Replace (lines 167-171):

```rust
        let hash = hex::encode(sha2::Sha256::digest(&content));
        state.set_self_destruct_key_hash(Some(hash.clone())).await;
        if let Err(e) = crate::bootstrap::save_self_destruct_key_hash_to_config(Some(hash.clone()))
        {
            log::error!("保存安全文件雜湊失敗: {}", e);
        }
```

With:

```rust
        let hash = hex::encode(sha2::Sha256::digest(&content));
        if let Err(e) = crate::bootstrap::save_self_destruct_key_hash_to_config(Some(hash.clone()))
        {
            log::error!("保存安全文件雜湊失敗: {}", e);
        } else {
            state.set_self_destruct_key_hash(Some(hash.clone())).await;
        }
```

- [ ] **Step 2: Fix `state_ops.rs`** — move `state.set_lang` after successful disk write

Replace (lines 53-57):

```rust
    i18n::set_lang(lang);
    state.set_lang(lang).await;
    if let Err(e) = crate::bootstrap::save_lang_to_config(lang) {
        log::error!("保存语言配置失败: {}", e);
    }
```

With:

```rust
    i18n::set_lang(lang);
    if let Err(e) = crate::bootstrap::save_lang_to_config(lang) {
        log::error!("保存语言配置失败: {}", e);
    } else {
        state.set_lang(lang).await;
    }
```

- [ ] **Step 3: Add test — corrupt input fails closed**

```rust
#[test]
fn update_config_rejects_corrupt_input() {
    let dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
    }
    let key = [0u8; 32];
    fs::write(dir.path().join(".key"), key).unwrap();
    let path = dir.path().join("config.enc");
    fs::write(&path, b"not valid json").unwrap();
    let result = save_lang_to_config(i18n::Lang::En);
    assert!(result.is_err());
    // File content unchanged
    assert_eq!(fs::read(&path).unwrap(), b"not valid json");
}
```

- [ ] **Step 4: Add test — missing config file fails closed**

```rust
#[test]
fn update_config_rejects_missing_file() {
    let dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("AEGIS_CONFIG_DIR", dir.path().to_str().unwrap());
    }
    let result = save_lang_to_config(i18n::Lang::En);
    assert!(result.is_err());
}
```

- [ ] **Step 5: Add test — write failure preserves old content**

```rust
#[test]
fn write_failure_preserves_old_content() {
    let dir = TempDir::new().unwrap();
    let config_dir = dir.path().join("etc/wwps/aegis");
    fs::create_dir_all(&config_dir).unwrap();
    unsafe {
        std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
    }
    let key = [0u8; 32];
    fs::write(config_dir.join(".key"), key).unwrap();
    let init = EncryptedConfig {
        token: b"t".to_vec(),
        admin_id: b"0".to_vec(),
        totp_secret: b"s".to_vec(),
        self_destruct_key_hash: None,
        matrix_homeserver: None,
        matrix_username: None,
        matrix_password: None,
        matrix_room_id: None,
        matrix_store_passphrase: None,
        lang: None,
        discord_token: None,
        discord_admin_id: None,
        matrix_recovery_key: None,
    };
    let path = config_dir.join("config.enc");
    let initial_bytes = serde_json::to_vec(&init).unwrap();
    fs::write(&path, &initial_bytes).unwrap();
    // Make directory read-only so atomic_write_sensitive fails
    fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = save_lang_to_config(i18n::Lang::En);
    assert!(result.is_err());
    // Restore perms so we can read
    fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), initial_bytes);
}
```

- [ ] **Step 6: Run all tests**

Run: `cargo test --lib bootstrap::tests -- --test-threads=1 2>&1` (serial test uses `AEGIS_CONFIG_DIR`)
Expected: PASS

- [ ] **Step 7: Verify fmt + clippy**

```bash
cargo fmt --check && cargo clippy -- -D warnings 2>&1
```
Expected: clean

- [ ] **Step 8: Commit**

```bash
git add src/bootstrap.rs src/shared/dispatch.rs src/shared/state_ops.rs
git commit -m "fix: persist config before publishing in-memory state"
```

---

### Task 4: Full verification sweep

- [ ] **Step 1: Run full test suite**

Run: `cargo test -- --test-threads=1 2>&1`
Expected: all pass (except pre-existing skip `test_deploy_candidate_rejects_version_mismatch`)

- [ ] **Step 2: Verify fmt + clippy**

```bash
cargo fmt --check && cargo clippy -- -D warnings 2>&1
```
Expected: clean

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "chore: add config-transaction tests"
```
