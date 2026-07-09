# Matrix Recovery Key + E2EE Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add recovery key import to `connect_matrix()` for existing cross-signing accounts, keep bootstrap for brand-new accounts, with atomic-clean "用完即焚".

**Architecture:** `bootstrap_cross_signing_if_needed` stays (it already forks: `get_user_identity(user_id).is_none()` → bootstrap; `Some()` → no-op). After bootstrap + `wait_for_e2ee`, check `cross_signing_status().is_complete()`. If true (brand-new or restore), skip. If false (existing identity, new device), call `Recovery::recover()` from `encrypted_config.matrix_recovery_key`, atomically clear the field. `SecretString` type prevents Debug/log leaks.

**Tech Stack:** Rust (matrix-sdk 0.18), Go (installer), i18n

## Global Constraints

- `matrix_recovery_key` added to `EncryptedConfig`/`SetupInput`/`DecryptedConfig` following exact `matrix_*` pattern (Option<Vec<u8>>, zeroize, serde(default))
- Recovery key is `secrecy::SecretString` in memory (not Debug, auto-zeroize)
- `cross_signing_status().await` → `Option<CrossSigningStatus>`; use `.is_complete()` helper
- `Recovery::recover(key)` → if `RecoveryError::BackupExistsOnServer`, fallback to `recover_and_fix_backup(key)`
- Atomic write: write `.tmp`, `fsync`, `rename` (same filesystem atomic)
- Go installer: add under Matrix section (not standalone), only `--setup-stdin` JSON (no CLI position arg)
- `cargo fmt && cargo clippy -- -D warnings && cargo test` must pass before each commit
- `go fmt ./... && go test ./...` must pass for installer changes

### Task 2: config.rs — Decrypt matrix_recovery_key in load_and_validate()

**Files:**
- Modify: `src/main/config.rs` — DecryptedConfig struct + decrypt block
- No test needed beyond compile (same pattern as discord_token/Matrix)

**Interfaces:**
- Consumes: `EncryptedConfig.matrix_recovery_key` from Task 1
- Produces: `DecryptedConfig.matrix_recovery_key: Option<String>` for use by connect_matrix in Task 3

- [ ] **Step 1: Add `matrix_recovery_key` to `DecryptedConfig`**

```rust
pub struct DecryptedConfig {
    ...
    pub matrix_recovery_key: Option<String>,
}
```

- [ ] **Step 2: Add decrypt block in `load_and_validate()` after the discord_admin_id block (between line 93 and the validator block)**

```rust
    let matrix_recovery_key = match &encrypted_config.matrix_recovery_key {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 matrix_recovery_key 失败")?;
            Some(
                String::from_utf8(vec.expose_secret().to_vec())
                    .map_err(|e| anyhow::anyhow!("matrix_recovery_key 包含无效的 UTF-8: {}", e))?
                    .trim()
                    .to_string(),
            )
        }
        None => None,
    };
```

- [ ] **Step 3: Add to DecryptedConfig construction in the return**

```rust
        DecryptedConfig {
            ...
            matrix_recovery_key,
        }
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check
```

- [ ] **Step 5: Commit**

```bash
git add src/main/config.rs src/bootstrap.rs
git commit -m "feat(aegis): decrypt matrix_recovery_key in config.rs"
```

---

### Task 3: matrix.rs — Keep bootstrap, add recovery logic + cross_signing_status check

**Files:**
- Modify: `src/main/matrix.rs` — connect_matrix()

**Interfaces:**
- Consumes: `encrypted_config.matrix_recovery_key`, `clear_matrix_recovery_key()` from Task 1, `security` (SecurityManager)
- Produces: MatrixHandle (unchanged signature)

**Flow change in connect_matrix():**
- L83-97 bootstrap: **KEEP** (no deletion)
- L99-103 wait_for_e2ee: **KEEP**
- **INSERT** cross_signing_status check + recover logic after wait_for_e2ee, before sync_once

- [ ] **Step 1: Add imports at top of matrix.rs**

```rust
use secrecy::SecretString;
```

- [ ] **Step 2: Add recovery key decrypt + recovery logic after wait_for_e2ee (between L103 and L105)**

Insert after `wait_for_e2ee_initialization_tasks()`:

```rust
    // P2: Check if local device has cross-signing private keys.
    // bootstrap_cross_signing_if_needed already ran (P2 above). If the account
    // had no remote identity, bootstrap created one and stored keys locally:
    // cross_signing_status.is_complete() == true. If the account already had
    // a remote identity, bootstrap was a no-op and local keys may be absent.
    let status = client.encryption().cross_signing_status().await;
    if status.map_or(false, |s| s.is_complete()) {
        println!("✅ 交叉签名状态完整");
    } else {
        println!("⚠ 交叉签名状态不完整，尝试恢复密钥导入");
        let rk_encrypted = encrypted_config
            .matrix_recovery_key
            .as_ref()
            .context("远端已有交叉签名身份，本设备缺少私钥。请在配置中提供 matrix_recovery_key (Element 的恢复密钥)")?;
        let rk_decrypted = security.decrypt(rk_encrypted)
            .context("解密 matrix_recovery_key 失败")?;
        let rk_str = String::from_utf8(rk_decrypted.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("matrix_recovery_key 包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string();
        let rk = SecretString::from(rk_str);

        let recovery = client.encryption().recovery();
        match recovery.recover(rk.expose_secret()).await {
            Ok(_) => {}
            Err(matrix_sdk::encryption::recovery::RecoveryError::BackupExistsOnServer) => {
                recovery.recover_and_fix_backup(rk.expose_secret()).await?;
                println!("✅ 恢复密钥 + 修复 backup 成功");
            }
            Err(e) => anyhow::bail!("恢复密钥导入失败: {e}"),
        }

        // Verify
        let status = client
            .encryption()
            .cross_signing_status()
            .await
            .context("recover 后 cross_signing_status 返回 None")?;
        ensure!(
            status.is_complete(),
            "恢复密钥导入后交叉签名状态仍不完整: master={}, self={}, user={}",
            status.has_master,
            status.has_self_signing,
            status.has_user_signing,
        );
        println!("✅ 恢复密钥导入成功，设备已加入信任链");

        // 用完即焚 — atomic clear
        crate::bootstrap::clear_matrix_recovery_key(config_dir)?;
        // SecretString zeroize happens on drop at end of else block
    }
```

Add `pub use anyhow::ensure;` or change to `anyhow::ensure!` (the crate already uses `anyhow`).

Actually, check: `anyhow::ensure!` is available. Use `anyhow::ensure!` directly.

Also need `use matrix_sdk::encryption::recovery::RecoveryError;` or use the full path.

- [ ] **Step 3: Verify compilation**

```bash
cargo check
```

- [ ] **Step 4: Run tests to verify no regressions**

```bash
cargo test
```
Expected: 584 tests, 0 failed

- [ ] **Step 5: Commit**

```bash
git add src/main/matrix.rs
git commit -m "feat(aegis): add recovery key import to connect_matrix()"
```

---

### Task 4: Update Rust test callers — EncryptedConfig constructors

**Files:**
- Modify: `src/main/matrix.rs` (test section) — `empty_config()` + 6 test constructors
- Modify: `src/bootstrap.rs` (test section) — `save_self_destruct_hash_compiles`

These are mechanical: every `EncryptedConfig` literal needs `matrix_recovery_key: None`.

- [ ] **Step 1: Update `empty_config()` in matrix.rs tests (add `matrix_recovery_key: None` before `lang`)**

```rust
    fn empty_config() -> EncryptedConfig {
        EncryptedConfig {
            ...
            matrix_recovery_key: None,
            lang: None,
        }
    }
```

- [ ] **Step 2: Update 6 test functions in matrix.rs that construct EncryptedConfig**

Search for `EncryptedConfig {` — there are 6 in matrix.rs tests. Each needs `matrix_recovery_key: None` added. They are:

1. `returns_true_when_all_matrix_fields_present` (L155)
2. `returns_false_when_matrix_fields_missing` (L174)
3. `returns_true_when_flag_overrides_empty_fields` (L180)
4. `returns_true_when_all_flag_overrides_empty_fields` (L186)
5. `returns_false_when_some_fields_missing` (L192)
6. `ignores_non_matrix_flags` (L209 - uses `empty_config()` already)

- [ ] **Step 3: Run tests**

```bash
cargo test
```

Expected: 584 tests, 0 failed

- [ ] **Step 4: Commit**

```bash
git add src/main/matrix.rs
git commit -m "test(aegis): update test EncryptedConfig constructors with matrix_recovery_key"
```

---

### Task 5: Go installer — main.go + i18n + test

**Files:**
- Modify: `go/installer/main.go` — buildSetupPayload + setupConfig + parseKeyVal + firstTimeSetup
- Modify: `go/installer/i18n/zh.json` — add keys
- Modify: `go/installer/i18n/en.json` — add keys
- Modify: `go/installer/i18n/ja.json` — add keys
- Modify: `go/installer/main_test.go` — update callers + new test

**Interfaces:**
- Consumes: Discord pattern from previous PR (buildSetupPayload signature extension, setupConfig field, parseKeyVal case)
- Produces: `matrix_recovery_key` in JSON payload for `--setup-stdin`; interactive prompt in Matrix section

- [ ] **Step 1: Add to `setupConfig` struct**

```go
    MatrixRecoveryKey string
```

- [ ] **Step 2: Add to `parseKeyVal` switch**

```go
    case "matrix_recovery_key":
        cfg.MatrixRecoveryKey = val
```

- [ ] **Step 3: Add to `buildSetupPayload` signature and body**

After Discord fields block, add Matrix recovery key:
```go
    if cfg.MatrixRecoveryKey != "" {
        payload = append(payload, ',')
        payload = append(payload, `"matrix_recovery_key":`...)
        payload = appendJSONEscaped(payload, []byte(cfg.MatrixRecoveryKey))
    }
```

- [ ] **Step 4: Add interactive prompt in `firstTimeSetup` Matrix section**

Inside the `if setupMatrix == "y"` block, after the room prompt and closing brace:
```go
    printYellow(i18n.T("firsttime.matrix_recovery_title"))
    printYellow(i18n.T("firsttime.matrix_recovery_help"))
    printYellow(i18n.T("firsttime.matrix_recovery_format"))
    fmt.Print(i18n.T("firsttime.matrix_recovery_prompt"))
    matrixRecoveryKey, _ = readLine()  // or readSecureInputStr — recovery key is sensitive
```

Use `readSecureInputStr` since recovery key is high-value.

- [ ] **Step 5: Add variable + wire into `buildSetupPayload` call**

In firstTimeSetup:
```go
    var matrixRecoveryKey string
    // inside setupMatrix == "y" block:
    // ... after matrixRoom reading ...
    printYellow(...)
    printYellow(...)
    printYellow(...)
    matrixRecoveryKey = readSecureInputStr(i18n.T("firsttime.matrix_recovery_prompt"))

    // And in the buildSetupPayload call:
    setupPayload := buildSetupPayload(
        ...,
        discordToken, discordAdminID, matrixRecoveryKey,
    )
```

- [ ] **Step 6: Update buildSetupPayload call in `installFromKeyVal` (add empty string param)**

```go
    payload := buildSetupPayload(
        ...,
        cfg.DiscordToken, cfg.DiscordAdminID, cfg.MatrixRecoveryKey,
    )
```

- [ ] **Step 7: Add i18n keys in zh.json**

```json
  "firsttime.matrix_recovery_title": "\n🔑 恢复密钥（可选）",
  "firsttime.matrix_recovery_help": "   如果你在 Element 的 E2EE 设置中设置了「恢复密钥」，",
  "firsttime.matrix_recovery_help2": "   可以在这里输入，让 bot 设备加入信任链。",
  "firsttime.matrix_recovery_format": "   格式: 人类可读的恢复密码或 base58 密钥",
  "firsttime.matrix_recovery_prompt": "请输入 Matrix 恢复密钥（留空跳过）: "
```

- [ ] **Step 8: Add i18n keys in en.json** (translate accordingly)

```json
  "firsttime.matrix_recovery_title": "\n🔑 Recovery Key (Optional)",
  "firsttime.matrix_recovery_help": "   If you configured a recovery key in Element's E2EE settings,",
  "firsttime.matrix_recovery_help2": "   enter it here to let the bot device join the trust chain.",
  "firsttime.matrix_recovery_format": "   Format: human-readable passphrase or base58 key",
  "firsttime.matrix_recovery_prompt": "Enter Matrix recovery key (leave empty to skip): "
```

- [ ] **Step 9: Add i18n keys in ja.json**

```json
  "firsttime.matrix_recovery_title": "\n🔑 復号キー（オプション）",
  "firsttime.matrix_recovery_help": "   Element の E2EE 設定で復号キーを設定している場合、",
  "firsttime.matrix_recovery_help2": "   ここに入力すると bot を信頼チェーンに追加できます。",
  "firsttime.matrix_recovery_format": "   形式: パスフレーズまたは base58 キー",
  "firsttime.matrix_recovery_prompt": "Matrix 復号キーを入力（空欄でスキップ）: "
```

- [ ] **Step 10: Update tests in main_test.go**

Update all `buildSetupPayload` callers with the new `""` empty string param:
- `TestBuildSetupPayload/without_matrix` (line ~87)
- `TestBuildSetupPayload/with_matrix` (line ~104)  
- `TestBuildSetupPayload/partial_matrix_fields` (line ~127)
- `TestParseKeyVal/non-ASCII password values` (line ~268)
- Add a new subtest verifying matrix_recovery_key appears in JSON

- [ ] **Step 11: Run Go tests**

```bash
go fmt ./... && go test ./...
```

Expected: all pass

- [ ] **Step 12: Commit**

```bash
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/zh.json go/installer/i18n/en.json go/installer/i18n/ja.json
git commit -m "feat(installer): add matrix recovery key support"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run full Rust lint + test**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

- [ ] **Step 2: Run full Go lint + test**

```bash
go fmt ./... && go test ./...
```

- [ ] **Step 3: Verify all commits on branch**

```bash
git log --oneline
```

Expected: 5-6 commits on `matrix-recovery-key` branch, all tests green

---

**Files:**
- Modify: `src/bootstrap.rs` — EncryptedConfig struct, SetupInput, Drop impl, run_setup, run_setup_from_stdin
- No test needed beyond compile (field addition is structural; integration tests verify serialization)

**Interfaces:**
- Consumes: `EncryptedConfig` existing pattern with `matrix_*` fields
- Produces: `matrix_recovery_key: Option<Vec<u8>>` on `EncryptedConfig`, `Option<String>` on `SetupInput`, `pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()>`

- [ ] **Step 1: Add `matrix_recovery_key` to `EncryptedConfig` struct (after `lang`)**

```rust
    #[serde(default)]
    pub matrix_recovery_key: Option<Vec<u8>>,
```

- [ ] **Step 2: Add `matrix_recovery_key` to `SetupInput` struct**

```rust
    #[serde(default)]
    matrix_recovery_key: Option<String>,
```

- [ ] **Step 3: Add zeroize in `Drop` impl**

```rust
    if let Some(v) = &mut self.matrix_recovery_key {
        v.zeroize();
    }
```

- [ ] **Step 4: Add to `run_setup()` function: decrypt from SetupInput → encrypt → set in EncryptedConfig**

In `run_setup()`: add recovery key field to the EncryptedConfig construction block. After the `run_setup_from_stdin()` pattern, the recovery key comes from `SetupInput.matrix_recovery_key`. Since `run_setup()` takes `matrix: Option<MatrixSetupConfig>`, add recovery_key param alongside the matrix block:

```rust
pub async fn run_setup(
    ...
    matrix_recovery_key: Option<&str>,
) -> Result<()> {
    ...
    let matrix_recovery_key = matrix_recovery_key
        .map(|k| security.encrypt(k.as_bytes()))
        .transpose()?;
    ...
    let encrypted_config = EncryptedConfig {
        ...
        matrix_recovery_key,
    };
```

- [ ] **Step 5: Wire in `run_setup_from_stdin()`**

```rust
    let matrix_recovery_key = input.matrix_recovery_key.as_deref();
    run_setup(..., matrix_recovery_key).await
```

- [ ] **Step 6: Add `clear_matrix_recovery_key()` atomic helper at end of bootstrap.rs (before tests)**

```rust
pub fn clear_matrix_recovery_key(config_dir: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::Write;

    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig =
        serde_json::from_slice(&data).context("解析 config.enc 失败")?;
    enc.matrix_recovery_key = None;
    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;

    let tmp_path = config_path.with_extension("enc.tmp");
    {
        let mut f = File::create(&tmp_path).context("创建临时文件失败")?;
        f.write_all(&new_data).context("写入临时文件失败")?;
        f.sync_all().context("fsync 临时文件失败")?;
    }
    fs::rename(&tmp_path, &config_path).context("rename config.enc 失败")?;
    println!("✅ 恢复密钥已从配置中清除（用完即焚）");
    Ok(())
}
```

- [ ] **Step 7: Verify compilation**

```bash
cargo check
```

Expected: compiles clean

- [ ] **Step 8: Commit**

```bash
git add src/bootstrap.rs
git commit -m "feat(aegis): add matrix_recovery_key to EncryptedConfig + atomic clear helper"
```

---
