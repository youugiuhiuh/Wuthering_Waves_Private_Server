# SimpleX Onboarding 修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 SimpleX 部署的 onboarding 端到端可完成 —— 管理员能拿到 bot 地址、连上 bot、取得自己的 contactId，并在不破坏其他配置字段的前提下写入配置。

**Architecture:** 三处定点改动。①aegis 连上 WebSocket 后取 `bot.address()`，日志 + 0600 原子落盘到 `/etc/wwps/aegis/simplex_address`。②aegis 新增 `--set-simplex-admin <contactId>`，以磁盘上的现有 `config.enc` 为底做定点字段替换（复用 `clear_matrix_recovery_key` 的原子写模式），完全不动 `run_setup` 的既有语义。③installer 在重启 aegis 后有界轮询该地址文件并打印，超时不算失败。i18n 文案从"去启动 simplex-chat"改为"首次安装留空"。

**Tech Stack:** Rust 2024 edition（aegis：tokio / anyhow / serde_json / aes-gcm `SecurityManager`）、Go 1.26（installer：标准库 + i18n embed）、`simploxide-client 0.14.0`、cargo-nextest。

**Spec:** `docs/superpowers/specs/2026-09-18-simplex-onboarding-design.md`

## Global Constraints

- **工作目录**：所有命令在 worktree `/home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/simplex-onboarding` 内执行。
- **Rust 构建缓存**：worktree 是全新的，直接 `cargo` 会重建 `matrix-sdk` 全套。**每次 cargo 命令前先 `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`**（已验证可把 10+ 分钟降到 ~1.5 分钟）。
- **Rust 质量门（每个 Rust 任务提交前必须全绿）**：
  ```
  cargo fmt && \
  cargo clippy --all-targets --all-features -- -D warnings && \
  cargo nextest run --cargo-profile fast-test && \
  cargo test --doc
  ```
  `cargo-nextest` 已安装。`cargo test --doc` 不能省 —— nextest 不跑文档测试。
- **Go 质量门**：`go fmt ./... && go vet ./... && go test ./...`（在 `go/installer` 下）。
- **回归门**：既有 Rust 测试 894 passed / 1 skipped 必须保持；Go 2 个包必须保持 ok。
- **禁止修改** `run_setup` / `run_setup_from_stdin` / `--setup` / `--setup-stdin` / `--setup-keyval` 的任何既有行为。
- **禁止修改** `platformForNonInteractive` / `parseKeyVal` 的校验规则。
- **禁止照抄** `bootstrap.rs::clear_matrix_recovery_key` 的权限写法 —— 它用 `File::create` 落成 0644。新增的地址写入必须自保证 0600。
- **权限要求**：`simplex_address` 文件必须是 0600；`config.enc` 保持 0600。
- **Rust 版本锁不动**：`simploxide-client` 保持 `=0.14.0`，`simplex-chat` 保持 `v7.0.0`。
- **命名**：注释与日志用中文（与仓库既有风格一致）；提交信息用 `type(scope): 描述` 中文格式。
- **单文件改动上限**：任何单个 patch 不超过 200 行。

---

## File Structure

| 文件 | 责任 | 本计划中的改动 |
| --- | --- | --- |
| `rust/aegis/src/core/paths.rs` | 全项目路径常量，按子系统分 `pub mod` | 在 `pub mod bot` 内加 `SIMPLEX_ADDRESS_FILE` |
| `rust/aegis/src/main/simplex.rs` | SimpleX 连接层（bin）：连接、事件流句柄 | 加 `write_address_file` / `record_address`，接线到 `connect_simplex` |
| `rust/aegis/src/bootstrap.rs` | 加密配置的读写与初始化（lib + bin 双编译） | 加 `set_simplex_admin_id` |
| `rust/aegis/src/main/cli.rs` | 早期 CLI 模式分发（bin） | 加 `CliMode::SetSimplexAdmin` + `parse_contact_id` |
| `go/installer/main.go` | 安装器主流程 | 加 3 个函数 + 2 处接线 |
| `go/installer/main_test.go` | 安装器 Go 测试 | 加 5 个测试 |
| `go/installer/i18n/zh.json` | 中文文案（基准语言） | +4 key，改 4 key |
| `go/installer/i18n/en.json` | 英文文案 | 同步 |
| `go/installer/i18n/ja.json` | 日文文案 | 同步 |

**不需要新文件。** `write_address_file` / `record_address` 留在 `main/simplex.rs`（与它们的唯一调用方同文件，且能直接访问私有 fn 做单测）；Go 三个函数留在 `main.go`（与既有 `validateSimplexPort` / `simplexPortFromUnit` 等 simplex 辅助函数同文件，保持文件职责不变）。

---

### Task 1: aegis — 地址常量与 0600 原子写

**Files:**
- Modify: `rust/aegis/src/core/paths.rs`（`pub mod bot` 块内，第 45-49 行附近）
- Modify: `rust/aegis/src/main/simplex.rs`（新增私有 fn + 测试）

**Interfaces:**
- Consumes: 无
- Produces:
  - `aegis::core::paths::bot::SIMPLEX_ADDRESS_FILE: &str`
  - `write_address_file(path: &std::path::Path, address: &str) -> std::io::Result<()>`（`main/simplex.rs` 内私有）

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/main/simplex.rs` 的 `#[cfg(test)] mod tests {`（文件末尾既有 `use super::*;`）内追加：

```rust
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_address_file_writes_address_with_newline_and_0600() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd\n"
        );
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn write_address_file_truncates_previous_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, &"a".repeat(500)).unwrap();
        write_address_file(&path, "short").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short\n");
    }

    /// 只靠建文件时的 mode() 不够：它对已存在的 tmp 不生效，而 truncate(true) 会复用该文件。
    #[test]
    fn write_address_file_overrides_permissions_of_stale_tmp() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        let stale_tmp = dir.path().join("simplex_address.tmp");
        std::fs::write(&stale_tmp, b"leftover").unwrap();
        std::fs::set_permissions(&stale_tmp, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_address_file(&path, "simplex:/x").unwrap();

        assert_eq!(mode_of(&path), 0o600, "陈旧 tmp 的 0644 不得被 rename 出去");
        assert!(!stale_tmp.exists(), "tmp 应已被 rename 消费掉");
    }

    #[test]
    fn write_address_file_reports_error_when_parent_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("no-such-dir").join("simplex_address");
        assert!(write_address_file(&path, "simplex:/x").is_err());
    }
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd rust/aegis && export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test write_address_file
```
Expected: 编译失败 —— `cannot find function write_address_file in this scope`。

- [ ] **Step 3: 加常量**

`rust/aegis/src/core/paths.rs`，在 `pub mod bot {` 块内、`BBR3_PENDING_FLAG_FILE` 之后：

```rust
    /// SimpleX bot 的 long-term 连接地址。aegis 连接后写入，installer 读取并打印。
    /// 必须与 go/installer/main.go 的 simplexAddressFile 保持一致。
    pub const SIMPLEX_ADDRESS_FILE: &str = "/etc/wwps/aegis/simplex_address";
```

- [ ] **Step 4: 写最小实现**

`rust/aegis/src/main/simplex.rs`，放在 `has_simplex_config` 之前：

```rust
/// 以 0600 原子写入 bot 地址文件：写 tmp → fsync → rename。
///
/// 权限显式设置两次 —— 建 tmp 时的 `.mode()` 对**已存在**的 tmp 不生效，而
/// `truncate(true)` 会复用它；上次崩溃残留的 tmp 若是 0644，rename 出去就是 0644。
fn write_address_file(path: &Path, address: &str) -> std::io::Result<()> {
    use std::fs::Permissions;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        writeln!(file, "{address}")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}
```

`main/simplex.rs` 顶部已有 `use std::path::Path;`（`connect_simplex` 的参数用到），无需新增 import。

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo nextest run --cargo-profile fast-test write_address_file
```
Expected: 4 passed。

- [ ] **Step 6: 质量门 + 提交**

```bash
cd rust/aegis && export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && \
  cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/core/paths.rs rust/aegis/src/main/simplex.rs
git commit -m "feat(simplex): 新增 bot 地址文件的 0600 原子写"
```

---

### Task 2: aegis — 接线：连接后记录 bot 地址

**Files:**
- Modify: `rust/aegis/src/main/simplex.rs`（`connect_simplex`，第 30-58 行附近）

**Interfaces:**
- Consumes: Task 1 的 `write_address_file`、`SIMPLEX_ADDRESS_FILE`
- Produces: `record_address(address: &str, path: &Path)`（`main/simplex.rs` 内私有）

- [ ] **Step 1: 写失败测试**

在 `main/simplex.rs` 的测试模块内追加：

```rust
    #[test]
    fn record_address_writes_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        record_address("simplex:/contact#/?v=2-7", &path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "simplex:/contact#/?v=2-7\n"
        );
    }

    /// 地址落盘失败不得影响 bot 启动 —— record_address 必须吞掉错误只 warn。
    #[test]
    fn record_address_swallows_io_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("no-such-dir").join("simplex_address");
        record_address("simplex:/contact#/?v=2-7", &path);
        assert!(!path.exists());
    }
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo nextest run --cargo-profile fast-test record_address
```
Expected: 编译失败 —— `cannot find function record_address`。

- [ ] **Step 3: 写最小实现**

在 `write_address_file` 之后：

```rust
/// 记录 bot 地址：打日志 + best-effort 落盘。
///
/// 任何失败只 warn，不返回错误 —— 地址拿不到不应让 bot 起不来。
fn record_address(address: &str, path: &Path) {
    log::info!("SimpleX bot 地址: {address}");
    if let Err(e) = write_address_file(path, address) {
        log::warn!("写入 SimpleX 地址文件失败（不影响 bot 运行）: {e}");
    }
}
```

- [ ] **Step 4: 接线到 `connect_simplex`**

把 `connect_simplex` 中这一段的：

```rust
    let (bot, events) = simploxide_client::ws::BotBuilder::new("Aegis", port)
        .auto_accept_with(rust_i18n::t!("simplex.welcome").to_string())
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("连接 SimpleX WebSocket 失败: {e}"))?;

    let adapter: Arc<dyn BotAdapter> = Arc::new(SimplexAdapter::new(bot.clone()));
```

改为：

```rust
    let (bot, events) = simploxide_client::ws::BotBuilder::new("Aegis", port)
        .auto_accept_with(rust_i18n::t!("simplex.welcome").to_string())
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("连接 SimpleX WebSocket 失败: {e}"))?;

    // connect() 内部已走完 setup_auto_accept，因此此刻地址必然已存在。
    // 读不到只 warn：地址缺失不应阻止 bot 启动（管理员仍可从日志排查）。
    match bot.address().await {
        Ok(address) => {
            record_address(&address, Path::new(aegis::core::paths::bot::SIMPLEX_ADDRESS_FILE))
        }
        Err(e) => log::warn!("读取 SimpleX bot 地址失败（不影响 bot 运行）: {e}"),
    }

    let adapter: Arc<dyn BotAdapter> = Arc::new(SimplexAdapter::new(bot.clone()));
```

`connect_simplex` 的参数 `_config_dir: &Path` 保持不动。

- [ ] **Step 5: 跑测试 + 质量门**

```bash
cargo nextest run --cargo-profile fast-test
```
Expected: 894 + 新增全 passed，1 skipped。

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test --doc
```

- [ ] **Step 6: 提交**

```bash
git add rust/aegis/src/main/simplex.rs
git commit -m "feat(simplex): 连接后输出并落盘 bot 地址"
```

---

### Task 3: aegis — 定点写入 simplex_admin_id

**Files:**
- Modify: `rust/aegis/src/bootstrap.rs`（新增 fn，放在 `clear_matrix_recovery_key` 之后、`install_crypto_provider` 之前）
- Modify: `rust/aegis/src/bootstrap.rs`（在既有 `mod config_tests` 内加测试）

**Interfaces:**
- Consumes: `EncryptedConfig`、`KEY_FILE`、`CONFIG_FILE`、`SecurityManager::new`（会自动生成缺失的 key 文件）
- Produces: `set_simplex_admin_id(config_dir: &Path, admin_id: i64) -> Result<()>`

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/bootstrap.rs` 的 `mod config_tests` 内追加（该模块已有 `use super::*;`）：

```rust
    /// 造一份填满所有字段的配置，用于验证定点替换不会碰到别的字段。
    fn seed_full_config(dir: &Path) -> EncryptedConfig {
        let seeded = EncryptedConfig {
            token: Some(b"123456:AA".to_vec()),
            admin_id: Some(b"777".to_vec()),
            totp_secret: Some(b"JBSWY3DPEHPK3PXP".to_vec()),
            self_destruct_key_hash: Some("a".repeat(64)),
            matrix_homeserver: Some(b"https://m.example".to_vec()),
            matrix_username: Some(b"@a:m.example".to_vec()),
            matrix_password: Some(b"pw".to_vec()),
            matrix_room_id: Some(b"!r:m.example".to_vec()),
            matrix_store_passphrase: Some(b"sp".to_vec()),
            lang: Some("zh".to_string()),
            matrix_recovery_key: Some(b"rk".to_vec()),
            simplex_port: Some(b"5225".to_vec()),
            simplex_admin_id: None,
        };
        fs::write(
            dir.join(CONFIG_FILE),
            serde_json::to_vec(&seeded).unwrap(),
        )
        .unwrap();
        seeded
    }

    fn read_config(dir: &Path) -> EncryptedConfig {
        serde_json::from_slice(&fs::read(dir.join(CONFIG_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn set_simplex_admin_id_preserves_every_other_field() {
        let dir = tempfile::TempDir::new().unwrap();
        let before = seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        let after = read_config(dir.path());
        assert_eq!(after.token, before.token);
        assert_eq!(after.admin_id, before.admin_id);
        assert_eq!(after.totp_secret, before.totp_secret, "TOTP 不得被轮换");
        assert_eq!(after.self_destruct_key_hash, before.self_destruct_key_hash);
        assert_eq!(after.matrix_homeserver, before.matrix_homeserver);
        assert_eq!(after.matrix_username, before.matrix_username);
        assert_eq!(after.matrix_password, before.matrix_password);
        assert_eq!(after.matrix_room_id, before.matrix_room_id);
        assert_eq!(after.matrix_store_passphrase, before.matrix_store_passphrase);
        assert_eq!(after.lang, before.lang);
        assert_eq!(after.matrix_recovery_key, before.matrix_recovery_key);
        assert_eq!(after.simplex_port, before.simplex_port, "simplex_port 不得被清空");
        assert!(after.simplex_admin_id.is_some(), "新值必须被写入");
    }

    #[test]
    fn set_simplex_admin_id_stores_decryptable_value() {
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        let security = SecurityManager::new(&dir.path().join(KEY_FILE)).unwrap();
        let raw = read_config(dir.path()).simplex_admin_id.unwrap();
        let plain = security.decrypt(&raw).unwrap();
        assert_eq!(String::from_utf8(plain.expose_secret().to_vec()).unwrap(), "42");
    }

    #[test]
    fn set_simplex_admin_id_rejects_non_positive() {
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());
        assert!(set_simplex_admin_id(dir.path(), 0).is_err());
        assert!(set_simplex_admin_id(dir.path(), -1).is_err());
    }

    #[test]
    fn set_simplex_admin_id_errors_when_config_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(set_simplex_admin_id(dir.path(), 42).is_err());
    }

    #[test]
    fn set_simplex_admin_id_leaves_no_tmp_and_keeps_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        seed_full_config(dir.path());

        set_simplex_admin_id(dir.path(), 42).unwrap();

        assert!(!dir.path().join("config.enc.tmp").exists());
        let mode = std::fs::metadata(dir.path().join(CONFIG_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
```

`set_simplex_admin_id_stores_decryptable_value` 用到 `expose_secret()` —— 需在 `mod config_tests` 内加 `use secrecy::ExposeSecret;`。

- [ ] **Step 2: 跑测试确认失败**

```bash
cd rust/aegis && export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test set_simplex_admin_id
```
Expected: 编译失败 —— `cannot find function set_simplex_admin_id`。

- [ ] **Step 3: 写最小实现**

在 `rust/aegis/src/bootstrap.rs` 的 `clear_matrix_recovery_key` 之后：

```rust
/// 就地更新配置中的 `simplex_admin_id`，保留其余字段（含各自的密文）不变。
///
/// 与 `run_setup` 的区别：`run_setup` 从参数从零构造 `EncryptedConfig`，未传字段一律写
/// `None`；本函数以磁盘上的现有配置为底做定点替换，因此不会清空 `simplex_port` /
/// `totp_secret` / `matrix_*`，也不会轮换 TOTP。
///
/// 原子写（tmp + fsync + rename），权限显式钉 0600 —— 不能沿用
/// `clear_matrix_recovery_key` 里 `File::create` 落成 0644 的写法。
pub fn set_simplex_admin_id(config_dir: &Path, admin_id: i64) -> Result<()> {
    use std::io::Write;

    if admin_id <= 0 {
        anyhow::bail!("contactId 必须是正整数，收到 {admin_id}");
    }

    let config_path = config_dir.join(CONFIG_FILE);
    let data = fs::read(&config_path).context("读取 config.enc 失败")?;
    let mut enc: EncryptedConfig =
        serde_json::from_slice(&data).context("解析 config.enc 失败")?;

    let security = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    enc.simplex_admin_id = Some(security.encrypt(admin_id.to_string().as_bytes())?);

    let new_data = serde_json::to_vec(&enc).context("序列化 config.enc 失败")?;
    let tmp_path = config_path.with_extension("enc.tmp");
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .context("创建临时文件失败")?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("设置临时文件权限失败")?;
        f.write_all(&new_data).context("写入临时文件失败")?;
        f.sync_all().context("fsync 临时文件失败")?;
    }
    fs::rename(&tmp_path, &config_path).context("rename config.enc 失败")?;

    println!("✅ SimpleX 管理员 contactId 已写入配置: {admin_id}");
    Ok(())
}
```

`File` 不得 import —— 实现里用的是 `std::fs::OpenOptions` 全路径，多一行 `use std::fs::File;` 在 `-D warnings` 下会因 `unused_imports` 直接挂掉质量门。

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo nextest run --cargo-profile fast-test set_simplex_admin_id
```
Expected: 5 passed × 2（lib 与 bin 各跑一次，因 `bootstrap.rs` 同时编进两个 target）。

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && \
  cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/bootstrap.rs
git commit -m "feat(simplex): 新增定点写入 simplex_admin_id 的配置函数"
```

---

### Task 4: aegis — `--set-simplex-admin` CLI 入口

**Files:**
- Modify: `rust/aegis/src/main/cli.rs`

**Interfaces:**
- Consumes: Task 3 的 `set_simplex_admin_id`、`config_dir`
- Produces: `CliMode::SetSimplexAdmin(Option<String>)`、`parse_contact_id(&str) -> anyhow::Result<i64>`

**关键约束**：`src/main.rs:36` 是 `if let Some(mode) = main::cli::try_cli_mode(&args) { return … }`。返回 `None` 会**继续正常启动 bot**。因此畸形的 `--set-simplex-admin` 必须返回 `Some(CliMode::SetSimplexAdmin(..))`，把校验推迟到 `execute_cli_mode` 让它以 `Err` 终止。

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/main/cli.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_plain_contact_id() {
        assert_eq!(parse_contact_id("42").unwrap(), 42);
    }

    #[test]
    fn parses_contact_id_with_surrounding_whitespace() {
        assert_eq!(parse_contact_id("  42\t").unwrap(), 42);
    }

    #[test]
    fn rejects_non_numeric_contact_id() {
        assert!(parse_contact_id("abc").is_err());
        assert!(parse_contact_id("").is_err());
        assert!(parse_contact_id("42.0").is_err());
    }

    #[test]
    fn recognises_set_simplex_admin_with_value() {
        let mode = try_cli_mode(&args(&["aegis", "--set-simplex-admin", "42"]));
        assert!(matches!(mode, Some(CliMode::SetSimplexAdmin(Some(v))) if v == "42"));
    }

    /// 缺参数时仍必须返回 Some —— 返回 None 会让 main 继续正常启动 bot。
    #[test]
    fn recognises_set_simplex_admin_without_value() {
        let mode = try_cli_mode(&args(&["aegis", "--set-simplex-admin"]));
        assert!(matches!(mode, Some(CliMode::SetSimplexAdmin(None))));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd rust/aegis && export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo nextest run --cargo-profile fast-test main::cli
```
Expected: 编译失败 —— `cannot find function parse_contact_id` / `no variant named SetSimplexAdmin`。

- [ ] **Step 3: 写最小实现**

`rust/aegis/src/main/cli.rs` 全文替换为：

```rust
use aegis::core::totp::TotpManager;
use anyhow::{Context, Result};

use crate::bootstrap::{config_dir, run_setup, run_setup_from_stdin, set_simplex_admin_id};

pub enum CliMode {
    Stdout(String),
    Setup {
        token: Option<String>,
        admin_id: Option<String>,
        totp_secret: Option<String>,
    },
    SetupStdin,
    /// 补填 SimpleX 管理员 contactId。保留原始字符串，正整数校验在 bootstrap 层
    /// （那里才知道 id 的语义），与 `CliMode::Setup` 保留原始 token 的风格一致。
    SetSimplexAdmin(Option<String>),
}

/// 解析 contactId 参数。仅做整数解析，非正数由 [`set_simplex_admin_id`] 拒绝。
pub fn parse_contact_id(raw: &str) -> Result<i64> {
    raw.trim()
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("contactId 必须是整数，收到 {raw:?}: {e}"))
}

pub fn try_cli_mode(args: &[String]) -> Option<CliMode> {
    if args.len() <= 1 {
        return None;
    }
    match args[1].as_str() {
        "--generate-totp-secret" => Some(CliMode::Stdout(TotpManager::generate_new_secret())),
        "-v" | "--version" => Some(CliMode::Stdout(format!(
            "aegis {}",
            env!("CARGO_PKG_VERSION")
        ))),
        "--setup" => Some(CliMode::Setup {
            token: args.get(2).cloned(),
            admin_id: args.get(3).cloned(),
            totp_secret: args.get(4).cloned(),
        }),
        "--setup-stdin" => Some(CliMode::SetupStdin),
        // 注意：缺参数时也必须返回 Some —— 返回 None 会让 main 继续正常启动 bot。
        "--set-simplex-admin" => Some(CliMode::SetSimplexAdmin(args.get(2).cloned())),
        _ => None,
    }
}

pub async fn execute_cli_mode(mode: CliMode) -> Result<()> {
    match mode {
        CliMode::Stdout(msg) => {
            println!("{msg}");
            Ok(())
        }
        CliMode::Setup {
            token,
            admin_id,
            totp_secret,
        } => {
            run_setup(
                token.as_deref(),
                admin_id.as_deref(),
                totp_secret.as_deref(),
                None,
                None,
                None,
                None,
            )
            .await
        }
        CliMode::SetupStdin => run_setup_from_stdin().await,
        CliMode::SetSimplexAdmin(raw) => {
            let raw = raw.context("用法: aegis --set-simplex-admin <contactId>")?;
            let admin_id = parse_contact_id(&raw)?;
            set_simplex_admin_id(&config_dir(), admin_id)
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo nextest run --cargo-profile fast-test main::cli
```
Expected: 5 passed。

- [ ] **Step 5: 手动验证 end-to-end（不碰真实 `/etc/wwps`）**

```bash
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
tmp=$(mktemp -d)
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --setup-stdin <<< '{"simplex_port":"5225"}'
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --set-simplex-admin 42
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --set-simplex-admin abc; echo "exit=$?（期望非 0）"
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --set-simplex-admin; echo "exit=$?（期望非 0）"
rm -rf "$tmp"
```
Expected: 第 2 条打印 `✅ SimpleX 管理员 contactId 已写入配置: 42`；第 3、4 条打印错误且退出码非 0（**不能静默启动 bot**）。

- [ ] **Step 6: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && \
  cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/main/cli.rs
git commit -m "feat(simplex): 新增 --set-simplex-admin CLI 入口"
```

---

### Task 5: installer — 读取、轮询并打印 bot 地址

**Files:**
- Modify: `go/installer/main.go`（3 个新函数 + 2 处接线）
- Modify: `go/installer/main_test.go`（新增测试）

**Interfaces:**
- Consumes: Task 1/2 的 `/etc/wwps/aegis/simplex_address`（0600，内容 `{addr}\n`）
- Produces:
  - `simplexAddressFile string`
  - `readSimplexAddress(path string) (string, bool)`
  - `pollSimplexAddress(path string, attempts int, interval time.Duration) (string, bool)`
  - `printSimplexOnboarding(platform string)`

- [ ] **Step 1: 写失败测试**

追加到 `go/installer/main_test.go` 末尾：

```go
func TestReadSimplexAddress(t *testing.T) {
	dir := t.TempDir()

	writeFile := func(name, body string) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}

	tests := []struct {
		name string
		path string
		want string
		ok   bool
	}{
		{"normal", writeFile("a", "simplex:/contact#/?v=2-7\n"), "simplex:/contact#/?v=2-7", true},
		{"no trailing newline", writeFile("b", "simplex:/x"), "simplex:/x", true},
		{"surrounding whitespace", writeFile("c", "  simplex:/y  \n"), "simplex:/y", true},
		{"empty", writeFile("d", ""), "", false},
		{"whitespace only", writeFile("e", "  \n\t "), "", false},
		{"missing", filepath.Join(dir, "nope"), "", false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, ok := readSimplexAddress(tt.path)
			if got != tt.want || ok != tt.ok {
				t.Fatalf("readSimplexAddress(%q) = (%q, %v), want (%q, %v)", tt.path, got, ok, tt.want, tt.ok)
			}
		})
	}
}

func TestPollSimplexAddressHitsImmediatelyWhenPresent(t *testing.T) {
	p := filepath.Join(t.TempDir(), "simplex_address")
	if err := os.WriteFile(p, []byte("simplex:/z\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	got, ok := pollSimplexAddress(p, 1, time.Millisecond)
	if !ok || got != "simplex:/z" {
		t.Fatalf("pollSimplexAddress = (%q, %v), want (simplex:/z, true)", got, ok)
	}
}

func TestPollSimplexAddressGivesUpWithoutPanic(t *testing.T) {
	got, ok := pollSimplexAddress(filepath.Join(t.TempDir(), "nope"), 1, time.Millisecond)
	if ok || got != "" {
		t.Fatalf("pollSimplexAddress = (%q, %v), want (\"\", false)", got, ok)
	}
}

// attempts<=0 必须立刻放弃，不得进入无条件循环。
func TestPollSimplexAddressZeroAttempts(t *testing.T) {
	p := filepath.Join(t.TempDir(), "simplex_address")
	if err := os.WriteFile(p, []byte("simplex:/z\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if got, ok := pollSimplexAddress(p, 0, time.Millisecond); ok || got != "" {
		t.Fatalf("pollSimplexAddress(attempts=0) = (%q, %v), want (\"\", false)", got, ok)
	}
}

// 文件延迟出现时轮询必须等到它 —— 这是真实场景（aegis 刚 fork，地址稍后才落盘）。
func TestPollSimplexAddressWaitsForLateFile(t *testing.T) {
	p := filepath.Join(t.TempDir(), "simplex_address")
	go func() {
		time.Sleep(10 * time.Millisecond)
		_ = os.WriteFile(p, []byte("simplex:/late\n"), 0o600)
	}()
	got, ok := pollSimplexAddress(p, 100, 5*time.Millisecond)
	if !ok || got != "simplex:/late" {
		t.Fatalf("pollSimplexAddress = (%q, %v), want (simplex:/late, true)", got, ok)
	}
}
```

`main_test.go` 的 import 需要含 `time` —— 当前是 `bytes/encoding/json/errors/fmt/io/net/http/net/http/httptest/os/os/exec/path/filepath/slices/strings/testing/testing/iotest`，**缺 `time`**，补上。

- [ ] **Step 2: 跑测试确认失败**

```bash
cd go/installer && go test ./... 2>&1 | head -20
```
Expected: 编译失败 —— `undefined: readSimplexAddress`。

- [ ] **Step 3: 写最小实现**

在 `go/installer/main.go` 的 `simplexPortFromUnit` 之后追加：

```go
// simplexAddressFile 是 aegis 落盘的 bot 连接地址。
// 必须与 rust/aegis/src/core/paths.rs::bot::SIMPLEX_ADDRESS_FILE 保持一致。
var simplexAddressFile = filepath.Join(installDir, "simplex_address")

// readSimplexAddress 读取并校验地址文件；不存在 / 空 / 仅空白视为未命中。
func readSimplexAddress(path string) (string, bool) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return "", false
	}
	addr := strings.TrimSpace(string(raw))
	if addr == "" {
		return "", false
	}
	return addr, true
}

// pollSimplexAddress 有界轮询地址文件；attempts <= 0 视为不轮询。
// 只在两次尝试之间 sleep，因此 attempts=1 是零等待的即时探测。
func pollSimplexAddress(path string, attempts int, interval time.Duration) (string, bool) {
	for i := 0; i < attempts; i++ {
		if addr, ok := readSimplexAddress(path); ok {
			return addr, true
		}
		if i < attempts-1 {
			time.Sleep(interval)
		}
	}
	return "", false
}

// printSimplexOnboarding 在部署收尾后打印 bot 地址与后续补填步骤。
// 平台不含 simplex 时为空操作。取不到地址只提示、不失败 —— aegis 的启动不应
// 因为地址文件写入慢而让安装以非 0 退出。
func printSimplexOnboarding(platform string) {
	if platform != "simplex" && platform != "tg-simplex" {
		return
	}
	if addr, ok := pollSimplexAddress(simplexAddressFile, 20, 500*time.Millisecond); ok {
		printGreen(i18n.T("simplex.address_ready", addr))
		printYellow(i18n.T("simplex.address_paste_hint"))
	} else {
		printYellow(i18n.T("simplex.address_pending"))
	}
	printYellow(i18n.T("simplex.admin_fill_hint"))
}
```

- [ ] **Step 4: 两处接线**

`installAegis()` 末尾（注意该分支用的是 `return`）：

```go
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		return
	}

	printGreen(i18n.T("install.success"))
	printSimplexOnboarding(platform)
	printSkyBlue(i18n.T("install.manage_hint"))
```

`finishDeploy()` 末尾（该分支用的是 `os.Exit(1)`）：

```go
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		os.Exit(1)
	}
	printGreen(i18n.T("install.success"))
	printSimplexOnboarding(platform)
	printSkyBlue(i18n.T("install.manage_hint"))
```

两处的 `platform` 变量在各自函数作用域内已存在，无需新增参数。

- [ ] **Step 5: 跑测试确认通过**

```bash
cd go/installer && go test ./... 2>&1 | tail -10
```
Expected: `ok … /installer` 与 `ok … /installer/i18n`。

- [ ] **Step 6: 质量门 + 提交**

```bash
go fmt ./... && go vet ./... && go test ./...
git add go/installer/main.go go/installer/main_test.go
git commit -m "feat(installer): 部署后打印 SimpleX bot 地址"
```

---

### Task 6: installer — i18n 文案（留空 + 地址提示）

**Files:**
- Modify: `go/installer/i18n/zh.json`
- Modify: `go/installer/i18n/en.json`
- Modify: `go/installer/i18n/ja.json`
- Modify: `go/installer/i18n/i18n_test.go`（新增 1 条格式串守卫）

**Interfaces:**
- Consumes: Task 5 里的 4 个新 key 名
- Produces: 无新符号

**注意**：`TestAllKeysExist`（`i18n_test.go:124`）已强制 `zh ⊆ en` 且 `zh ⊆ ja`，新增 key 只加在 `zh.json` 会直接测试失败；`TestNoDuplicateKeys` 已防重复键。这两条无需另写。

- [ ] **Step 1: 写失败测试**

追加到 `go/installer/i18n/i18n_test.go` 末尾：

```go
// 带参数的文案必须保留 %s 占位符，否则 i18n.T 的实参会被 fmt 丢弃。
func TestSimplexAddressKeysKeepFormatVerb(t *testing.T) {
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		if !strings.Contains(locale.data["simplex.address_ready"], "%s") {
			t.Errorf("%s: simplex.address_ready 缺少 %%s 占位符: %q",
				locale.name, locale.data["simplex.address_ready"])
		}
	}
}
```

`i18n_test.go` 的 import 块当前是 `bytes / embed / encoding/json / testing` —— **必须把 `strings` 加进去**，否则编译失败：

```go
import (
	"bytes"
	"embed"
	"encoding/json"
	"strings"
	"testing"
)
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd go/installer/i18n && go test ./... 2>&1 | head -10
```
Expected: 编译失败或 `missing key` —— `simplex.address_ready` 尚未定义。

- [ ] **Step 3: 改 zh.json**

`go/installer/i18n/zh.json`：

新增 4 个 key（放在既有 `simplex.*` 键组内，紧邻 `simplex.service_ok` 之后）：

```json
  "simplex.address_ready": "✅ bot 地址: %s",
  "simplex.address_paste_hint": "   复制上面这一行，粘到 SimpleX 客户端的输入框里连接 bot（或发 /c <地址>）",
  "simplex.address_pending": "⏳ 暂未取到 bot 地址（aegis 可能仍在启动）。稍后执行：journalctl -u wwps-aegis | grep \"bot 地址\"",
  "simplex.admin_fill_hint": "   连上后给 bot 发任意一条消息，再执行：aegis --set-simplex-admin <ID>（ID 见 journalctl -u wwps-aegis | grep 未授权联系人）",
```

改写 4 个既有 key：

```json
  "firsttime.simplex_admin_title": "\n👤 SimpleX 管理员 contactId（首次安装请留空）",
  "firsttime.simplex_admin_help_step1": "  首次安装时 bot 还没启动，拿不到 contactId —— 直接回车留空即可。",
  "firsttime.simplex_admin_help_step2": "  安装完成后安装器会打印 bot 地址；连上并发送一条消息后，用 aegis --set-simplex-admin 补填。",
  "firsttime.simplex_admin_prompt": "请输入 SimpleX 管理员 contactId（留空即可，稍后补填）：",
```

`firsttime.simplex_admin_help_format` 保留原文（格式示例仍成立）。

- [ ] **Step 4: 同步 en.json / ja.json**

`en.json` 对应 4 新增 + 4 改写：

```json
  "simplex.address_ready": "✅ Bot address: %s",
  "simplex.address_paste_hint": "   Copy the line above and paste it into your SimpleX client to connect (or send /c <address>)",
  "simplex.address_pending": "⏳ Bot address not available yet (aegis may still be starting). Later run: journalctl -u wwps-aegis | grep \"bot 地址\"",
  "simplex.admin_fill_hint": "   After connecting, send any message to the bot, then run: aegis --set-simplex-admin <ID> (find the ID via journalctl -u wwps-aegis | grep 未授权联系人)",
  "firsttime.simplex_admin_title": "\n👤 SimpleX admin contactId (leave empty on first install)",
  "firsttime.simplex_admin_help_step1": "  On first install the bot is not running yet, so no contactId exists — just press Enter to leave it empty.",
  "firsttime.simplex_admin_help_step2": "  After install the installer prints the bot address; connect, send one message, then fill it in with aegis --set-simplex-admin.",
  "firsttime.simplex_admin_prompt": "SimpleX admin contactId (leave empty, fill in later):",
```

`ja.json` 对应 4 新增 + 4 改写：

```json
  "simplex.address_ready": "✅ bot アドレス: %s",
  "simplex.address_paste_hint": "   上の行をコピーし、SimpleX クライアントに貼り付けて接続します（または /c <アドレス> を送信）",
  "simplex.address_pending": "⏳ bot アドレスをまだ取得できていません（aegis 起動中の可能性）。後で実行: journalctl -u wwps-aegis | grep \"bot 地址\"",
  "simplex.admin_fill_hint": "   接続後に bot へ任意のメッセージを送り、次を実行: aegis --set-simplex-admin <ID>（ID は journalctl -u wwps-aegis | grep 未授权联系人）",
  "firsttime.simplex_admin_title": "\n👤 SimpleX 管理者 contactId（初回インストールでは空欄）",
  "firsttime.simplex_admin_help_step1": "  初回インストール時点では bot が未起動のため contactId は存在しません —— Enter で空欄のまま進めてください。",
  "firsttime.simplex_admin_help_step2": "  インストール完了後に installer が bot アドレスを表示します。接続して 1 通送信した後、aegis --set-simplex-admin で入力してください。",
  "firsttime.simplex_admin_prompt": "SimpleX 管理者 contactId（空欄可、後で入力）：",
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd go/installer && go test ./... 2>&1 | tail -10
```
Expected: `ok … /installer` 与 `ok … /installer/i18n`。特别确认 `TestAllKeysExist`、`TestNoDuplicateKeys`、`TestSimplexAddressKeysKeepFormatVerb` 三个都 pass。

- [ ] **Step 6: 质量门 + 提交**

```bash
go fmt ./... && go vet ./... && go test ./...
git add go/installer/i18n/
git commit -m "feat(installer): SimpleX 提示语改为首次安装留空并说明补填路径"
```

---

## 收尾：全量验收

- [ ] **Rust 全量质量门**

```bash
cd rust/aegis && export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
cargo fmt --check && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```
Expected: `894 + 11 条新增`（Task 1 四条、Task 2 两条、Task 3 五条、Task 4 五条 = 16 → 实际数以本次输出为准）全 passed，1 skipped，0 failed。

- [ ] **Go 全量质量门**

```bash
cd go/installer && go fmt ./... && go vet ./... && go test ./...
```

- [ ] **端到端手验（模拟纯 simplex 部署的补填闭环）**

```bash
export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target
tmp=$(mktemp -d)
cd rust/aegis
# 1. 模拟安装器写入的「首次配置」：只有端口，没有管理员
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --setup-stdin <<< '{"simplex_port":"5225"}'
# 2. 模拟管理员拿到 contactId 后补填
AEGIS_CONFIG_DIR="$tmp" cargo run --quiet -- --set-simplex-admin 42
# 3. 断言端口还在（这正是旧 keyval 路径会清空的字段）
python3 -c "
import json,sys
d=json.load(open('$tmp/config.enc'))
assert d.get('simplex_port') is not None, 'simplex_port 被清空了 —— 回归！'
assert d.get('simplex_admin_id') is not None, 'simplex_admin_id 未写入'
print('✅ simplex_port 保留 + simplex_admin_id 已写入')
"
rm -rf "$tmp"
```

- [ ] **对照 spec §8 的 8 条验收标准逐条勾选**

- [ ] **requesting-code-review**：对照计划审查全部 diff，Critical 问题阻塞收尾。

- [ ] **finishing-a-development-branch**：验证测试 → 选择 merge / PR / keep / discard → 清理 worktree。
