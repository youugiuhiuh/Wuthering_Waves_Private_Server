# 用完即焚（totp_secret + matrix 恢复密钥）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 TOTP secret 与 Matrix 恢复密钥在内存中"用完即焚"——消除全进程驻留的死明文字段与裸明文中间拷贝。

**Architecture:** 在 `SecurityManager` 新增 `decrypt_secret` 助手（decrypt → UTF-8 → trim → `SecretString`，全链仅一次明文拷贝且受 zeroize 保护）；`config.rs` 中 totp 改局部 `SecretString` 用完即弃、不再存入 `DecryptedConfig`；`matrix.rs` 恢复密钥路径改用同一助手消除裸中间态。

**Tech Stack:** Rust, zeroize 1.9, secrecy 0.8 (`SecretString = Secret<String>`，drop 清零，`expose_secret() -> &String`)

**Spec:** `docs/superpowers/specs/2026-09-07-secret-burn-design.md`

## Global Constraints

- 不得改动 `TotpManager` 内部 TOTP 密钥、totp 磁盘密文（重启必需）、matrix 恢复密钥磁盘"成功即烧、失败保留"语义（matrix.rs:277 现有逻辑）。
- 全链路禁止新增裸明文拷贝；凡明文必经 `SecretString`/`Zeroizing`。
- 所有改动在 worktree `.worktrees/secret-burn`（分支 `feat/secret-burn`）内完成。
- 完成每个任务跑：`cargo test`、任务收尾跑 rust-lint-format 门禁（fmt/clippy/nextest）。
- 已知基线：`xhttp_domain_provider_routes_to_xray_handler` 整包并行偶发失败、单跑必过——预存 flaky，与本计划无关。

---

### Task 1: `SecurityManager::decrypt_secret` 助手 + 单测

**Files:**
- Modify: `rust/aegis/src/core/security/crypto.rs`（import + 新增方法）
- Test: `rust/aegis/src/core/security/crypto.rs`（`#[cfg(test)] mod tests` 内新增）

**Interfaces:**
- Consumes: 既有 `SecurityManager::decrypt(&self, &[u8]) -> Result<SecretVec<u8>>`、`encrypt(&self, &[u8]) -> Result<Vec<u8>>`
- Produces: `SecurityManager::decrypt_secret(&self, data: &[u8]) -> Result<secrecy::SecretString>` — Task 2/3 消费

- [ ] **Step 1: 修改 import（让编译失败 → RED 的第一半）**

`rust/aegis/src/core/security/crypto.rs` 顶部（现为 `use secrecy::SecretVec;`）：

```rust
use secrecy::{SecretString, SecretVec};
```

- [ ] **Step 2: 写失败测试**

在 `crypto.rs` 的 `#[cfg(test)] mod tests`（现已有 `use secrecy::ExposeSecret;`、`tempfile::TempDir`）末尾追加：

```rust
    #[test]
    fn test_decrypt_secret_roundtrip() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        let encrypted = sm.encrypt(b"JBSWY3DPEHPK3PXP").unwrap();
        let secret = sm.decrypt_secret(&encrypted).unwrap();

        assert_eq!(secret.expose_secret(), "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn test_decrypt_secret_trims_whitespace() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        let encrypted = sm.encrypt(b"  JBSWY3DPEHPK3PXP\n").unwrap();
        let secret = sm.decrypt_secret(&encrypted).unwrap();

        assert_eq!(secret.expose_secret(), "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn test_decrypt_secret_rejects_invalid_utf8() {
        let temp = TempDir::new().unwrap();
        let sm = SecurityManager::new(&temp.path().join("key")).unwrap();

        // 合法密文但明文非 UTF-8（0xFF 非法字节）
        let encrypted = sm.encrypt(&[0xff, 0xfe, 0x00, 0x80]).unwrap();
        let result = sm.decrypt_secret(&encrypted);

        assert!(result.is_err());
    }
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test test_decrypt_secret -p aegis -- --nocapture`（在 `rust/aegis` 下）
Expected: FAIL——`no method named decrypt_secret found`

- [ ] **Step 4: 实现 `decrypt_secret`**

在 `crypto.rs` 的 `SecurityManager` impl 内、`decrypt` 方法之后插入：

```rust
    /// 解密为受保护字符串：decrypt → UTF-8 校验 → trim → SecretString。
    /// 全链仅一次明文拷贝（to_vec），且经 Zeroizing 包裹后 trim 进 SecretString，
    /// SecretString drop 时自动清零——不留任何游离裸拷贝。
    pub fn decrypt_secret(&self, data: &[u8]) -> Result<SecretString> {
        let vec = self.decrypt(data)?;
        let s = String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("decrypted data contains invalid UTF-8: {}", e))?;
        let s = Zeroizing::new(s);
        Ok(SecretString::from(s.trim().to_string()))
    }
```

注意：`zeroize::Zeroizing`、`secrecy::ExposeSecret`（trait 方法需要）——`Zeroizing` 已在文件顶部 import；若 `ExposeSecret` 未在文件主体 import，在文件顶部 `use secrecy::{ExposeSecret, SecretString, SecretVec};` 补上（trait 须在作用域内才能调 `expose_secret()`）。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test test_decrypt_secret -p aegis`
Expected: PASS（3 个新测试）

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/core/security/crypto.rs
git commit -m "feat(security): add SecurityManager::decrypt_secret one-hop secret decryption"
```

---

### Task 2: `config.rs` — totp_secret 用完即焚，移除全进程死字段

**Files:**
- Modify: `rust/aegis/src/main/config.rs`
- Test: `rust/aegis/src/main/config.rs`（`#[cfg(test)] mod tests` 内新增回归测试）

**Interfaces:**
- Consumes: Task 1 的 `decrypt_secret`；既有 `TotpManager::new(&SecretString)`、`ConfigValidator::validate_decrypted_config`
- Produces: `DecryptedConfig` **删除** `totp_secret` 字段；`load_and_validate` 行为不变（返回 `AppConfig { totp_manager, .. }`）

- [ ] **Step 1: 写回归/特征测试（先绿后改——重构类改动，行为不变量先钉死）**

`src/main/config.rs` 底部 `#[cfg(test)] mod tests`（已有 `use super::*; use std::fs; use tempfile::TempDir;`，测试均 `#[serial]` + 设 `AEGIS_CONFIG_DIR` env）追加。**同步构造** .key + config.enc（不等价于 `run_setup` 的 async 路径，直接用 `SecurityManager` + `EncryptedConfig` 字面量——避开 serial_test×tokio 兼容问题）：

```rust
    #[serial]
    #[test]
    fn load_and_validate_builds_working_totp_manager() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        // 同步构造 setup 等价产物：.key 由 SecurityManager 生成，config.enc 手工加密
        // token 需通过格式校验: <数字bot_id>:<token>
        let totp_secret = "JBSWY3DPEHPK3PXP";
        let security = SecurityManager::new(&config_dir.join(KEY_FILE)).unwrap();
        let encrypted = EncryptedConfig {
            token: Some(security.encrypt(b"123456:ABCdefGHIjklMNOpqrsTUVwxyz").unwrap()),
            admin_id: Some(security.encrypt(b"42").unwrap()),
            totp_secret: Some(security.encrypt(totp_secret.as_bytes()).unwrap()),
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: Some("zh".to_string()),
            matrix_recovery_key: None,
        };
        fs::write(
            config_dir.join(CONFIG_FILE),
            serde_json::to_vec(&encrypted).unwrap(),
        )
        .unwrap();

        let (app_config, _security) = load_and_validate().unwrap();
        let manager = app_config.totp_manager.expect("totp_manager 应已构建");

        // TotpManager 功能完好：自身生成的当前码能通过 verify（证明密钥正确装载且可用）
        let code = manager.generate_current().unwrap();
        assert!(manager.verify(&code));
    }
```

注：`SecurityManager`、`KEY_FILE`、`CONFIG_FILE`、`EncryptedConfig` 均已在本文件顶部 import（`use aegis::core::security::SecurityManager;` / `use crate::bootstrap::{... KEY_FILE, CONFIG_FILE, EncryptedConfig ...}`），`super::*` 带进测试模块。

- [ ] **Step 2: 跑测试确认当前代码 PASS（特征钉死）**

Run: `cargo test load_and_validate_builds_working_totp_manager -p aegis`
Expected: PASS（重构前行为即如此——本测试用于重构后防回归，不驱动 RED）

- [ ] **Step 3: 重构 — 删除结构体死字段**

`DecryptedConfig`（`src/main/config.rs:14-26`）：

```rust
pub struct DecryptedConfig {
    pub token: Option<String>,
    pub admin_id: Option<i64>,
    #[expect(dead_code)]
    pub totp_secret: Option<String>,   // ← 删除本字段及其上 #[expect(dead_code)]
    #[expect(dead_code)]
    pub discord_token: Option<String>,
    #[expect(dead_code)]
    pub discord_admin_id: Option<i64>,
    pub encrypted_config: EncryptedConfig,
}
```

改为（仅删 totp 两行，discord 字段保留——不在本次范围）：

```rust
pub struct DecryptedConfig {
    pub token: Option<String>,
    pub admin_id: Option<i64>,
    #[expect(dead_code)]
    pub discord_token: Option<String>,
    #[expect(dead_code)]
    pub discord_admin_id: Option<i64>,
    pub encrypted_config: EncryptedConfig,
}
```

- [ ] **Step 4: 重构 — 解密改局部 SecretString（用完即焚）**

把解密块（现为）：

```rust
    let totp_secret = match &encrypted_config.totp_secret {
        Some(v) => {
            let vec = security.decrypt(v).context("解密 totp_secret 失败")?;
            Some(
                String::from_utf8(vec.expose_secret().to_vec())
                    .context("totp_secret 包含无效的 UTF-8 字符")?
                    .trim()
                    .to_string(),
            )
        }
        None => None,
    };
```

替换为：

```rust
    // totp_secret：用完即焚——仅用于构建 TotpManager，不存入任何驻留字段。
    // SecretString 局部变量出作用域即清零。
    let totp_secret: Option<secrecy::SecretString> = match &encrypted_config.totp_secret {
        Some(v) => Some(security.decrypt_secret(v).context("解密 totp_secret 失败")?),
        None => None,
    };
```

- [ ] **Step 5: 重构 — 校验调用适配（Option<String> → Option<SecretString>）**

`validate_decrypted_config` 调用点（`totp_secret.as_deref()` 不再可用）：

```rust
        totp_secret.as_deref(),
```
改为：
```rust
        totp_secret.as_ref().map(|s| s.expose_secret().as_str()),
```

- [ ] **Step 6: 重构 — TotpManager 构建去 clone + 显式 drop**

现为：

```rust
    let totp_manager = match totp_secret {
        Some(ref secret) => Some(
            TotpManager::new(&secrecy::SecretString::from(secret.clone()))
                .map_err(|e| anyhow::anyhow!("初始化 TOTP 验证器失败: {}", e))?,
        ),
        None => None,
    };
```

改为：

```rust
    let totp_manager = match totp_secret.as_ref() {
        Some(secret) => Some(
            TotpManager::new(secret)
                .map_err(|e| anyhow::anyhow!("初始化 TOTP 验证器失败: {}", e))?,
        ),
        None => None,
    };
    // 用完即焚：TotpManager 已持有受保护副本，明文局部立即清零释放，不再驻留整个进程
    drop(totp_secret);
```

- [ ] **Step 7: 重构 — 结构体字面量删字段**

`AppConfig` 构建处（现含 `totp_secret,` 行）：

```rust
            decrypted: DecryptedConfig {
                token,
                admin_id,
                totp_secret,
                discord_token,
                discord_admin_id,
                encrypted_config,
            },
```

改为（删 `totp_secret,` 行）：

```rust
            decrypted: DecryptedConfig {
                token,
                admin_id,
                discord_token,
                discord_admin_id,
                encrypted_config,
            },
```

- [ ] **Step 8: 编译 + 全测试**

Run: `cargo test -p aegis`
Expected: 编译通过；`load_and_validate_builds_working_totp_manager` 及既有全部测试 PASS（flaky 单测例外：如遇 `xhttp_domain_provider_routes_to_xray_handler` 失败，单独重跑确认通过即可）

- [ ] **Step 9: Commit**

```bash
git add rust/aegis/src/main/config.rs
git commit -m "refactor(security): burn totp_secret after TotpManager construction, drop dead DecryptedConfig field"
```

---

### Task 3: `matrix.rs` — 恢复密钥解密路径收口（消除裸中间拷贝）

**Files:**
- Modify: `rust/aegis/src/main/matrix.rs`（`try_recover_with_key`）

**Interfaces:**
- Consumes: Task 1 的 `decrypt_secret`；既有 `client.encryption().recovery()` API
- Produces: `try_recover_with_key(...) -> bool` 行为不变（配置缺失 false / 解密失败 false / 空密钥 false / 成功 true）

- [ ] **Step 1: 改写 `try_recover_with_key`**

现为（`src/main/matrix.rs:84-115` 附近，`let Some(rk_encrypted)` 到 match result 结束）：

```rust
    let Some(rk_encrypted) = encrypted_config.matrix_recovery_key.as_ref() else {
        return false; // 未配置恢复密钥——属正常路径，无需日志
    };
    let Ok(rk_decrypted) = security.decrypt(rk_encrypted) else {
        println!("⚠ 恢复密钥解密失败（config.enc 中 matrix_recovery_key 损坏）");
        return false;
    };
    let Some(rk_str) = String::from_utf8(rk_decrypted.expose_secret().to_vec())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        println!("⚠ 恢复密钥为空或包含无效 UTF-8");
        return false;
    };

    let rk = SecretString::from(rk_str);
    let recovery = client.encryption().recovery();
    let result = match recovery.recover(rk.expose_secret()).await {
```

替换为：

```rust
    let Some(rk_encrypted) = encrypted_config.matrix_recovery_key.as_ref() else {
        return false; // 未配置恢复密钥——属正常路径，无需日志
    };
    // 用完即焚：decrypt_secret 一步到位进 SecretString（trim 后），
    // 不再产生 to_vec/String 裸中间拷贝；SecretString 用完 drop 即清零。
    let Ok(rk) = security.decrypt_secret(rk_encrypted) else {
        println!("⚠ 恢复密钥解密失败（config.enc 中 matrix_recovery_key 损坏）");
        return false;
    };
    if rk.expose_secret().is_empty() {
        println!("⚠ 恢复密钥为空或包含无效 UTF-8");
        return false;
    }

    let recovery = client.encryption().recovery();
    let result = match recovery.recover(rk.expose_secret()).await {
```

其余部分（`Ok(_) => Ok(())` / BackupExistsOnServer / Err(e) / match result）**不动**。函数内 `rk` 保持为本地 `SecretString`，随函数返回 drop 清零。

行为差异说明（有意为之，仅影响告警文案）：原先"密文合法但明文含非法 UTF-8"会落入"为空或包含无效 UTF-8"提示；现在落入"解密失败"提示（`decrypt_secret` 的 UTF-8 错误归入 Err）。均为 boot 期一次性 println，无逻辑影响。

- [ ] **Step 2: 编译 + 相关测试**

Run: `cargo test -p aegis`
Expected: 编译通过（`ExposeSecret` 已在 matrix.rs:14 import，`SecretString` 如在函数内不再直接使用则可能产生 unused import——如报错，把 `use secrecy::ExposeSecret;` 保留（`rk.expose_secret()` 仍用 trait），仅当编译器提示时才清理）；既有 matrix 相关测试（`after_recovery_action` 纯函数测试等）PASS

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/main/matrix.rs
git commit -m "refactor(security): burn matrix recovery-key intermediates via decrypt_secret"
```

---

### Task 4: rust-lint-format 门禁 + 全量验证

**Files:** 无（验证任务）

- [ ] **Step 1: fmt + clippy**

Run（`rust/aegis` 下）:
```bash
cargo fmt --all -- --check   # 若报 diff: cargo fmt --all 后复查
cargo clippy --all-targets -- -D warnings
```
Expected: 均通过。如 clippy 报既有代码问题（非本次引入），记录不修并说明。

- [ ] **Step 2: nextest 全量**

Run: `cargo nextest run -p aegis`
Expected: PASS（flaky 单测例外处理同 Task 2 Step 8）

- [ ] **Step 3: 全仓编译冒烟**

Run: `cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/secret-burn && cargo build --manifest-path rust/aegis/Cargo.toml`
Expected: 成功

- [ ] **Step 4: 自查改动面**

Run: `git diff main --stat`（相对 main 应为：crypto.rs、config.rs、matrix.rs + 计划/设计文档）
Expected: 无计划外文件改动

- [ ] **Step 5: Commit（如 fmt/clippy 有修正）**

```bash
git add -A rust/aegis/src
git commit -m "style: satisfy fmt/clippy after secret-burn changes"
```
（无改动则跳过）
