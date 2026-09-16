# Discord 平台移除实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从 aegis 运行时（Rust）与安装器（Go）全链路移除 Discord 平台，并对一切残留的 Discord 痕迹（`--discord` 参数、`config.enc` 字段、keyval/JSON 配置键、现有 systemd 单元）采取硬失败，不静默切换到其它平台。

**Architecture:** 删除 `gateways/discord/` 与 `main/discord.rs` 两个模块及其在 `Platform` 枚举 / `PlatformCapabilities` / `run()` 接线 / `AppState` 身份 / 加密配置中的全部字段；`resolve_platform_selection` 在解析任何 flag 前对 `--discord` 返回错误。安装器侧删除选择器第 3 项、配置键、payload 字段与 systemd 描述，并在升级路径的共享入口 `recoveryPlatformForService` 拦截带 `--discord` 的既有单元。

**Tech Stack:** Rust 2024 / tokio / clap-less 手写 CLI 解析 / `serenity`+`poise`（本次移除）/ Go 1.26 安装器（bubbletea）/ systemd。

**Spec:** `docs/superpowers/specs/2026-09-16-remove-discord-design.md`

## Global Constraints

- 依赖增删**只能**通过 `cargo remove` 完成，禁止手改 `Cargo.toml` 依赖段（dependency-management 规则）。
- `--discord` 必须**硬失败**（非零退出 + 明确提示），不得静默忽略、不得落到自动探测。
- 安装器所有 Discord 输入面（keyval 键、JSON 键、既有 systemd 单元、平台编号 `"3"`）一律硬失败。
- `platformSetupForChoice` **保留编号空洞**：`case "3"` 返回错误，`4`/`5` 编号不得改动。
- `EncryptedConfig` 保持 `#[serde(default)]`，**禁止**引入 `deny_unknown_fields`（老 `config.enc` 必须能加载）。
- 每个 Rust 任务结束执行：`cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc`。
- 每个 Go 任务结束执行：`gofmt -l . && go vet ./... && go test ./... && staticcheck ./...`。
- 工作树：`.worktrees/remove-discord`（分支 `feat/remove-discord`）。
- 历史文档**不改写**：`docs/2026-09-16-simplex-platform.md` 与 `docs/superpowers/{specs,plans}/2026-09-16-simplex-platform*.md` 保持原样。
- i18n 三语键集必须一致：`go/installer/i18n/zh.json` 是 `TestAllKeysExist` 的基准，`en`/`ja` 必须覆盖 zh 的每个键。

---

## 文件结构

| 文件 | 职责 | 本次动作 |
|---|---|---|
| `rust/aegis/src/gateways/discord/mod.rs`、`adapter.rs` | Discord 适配器 | 删除 |
| `rust/aegis/src/main/discord.rs` | Discord 连接层与事件循环 | 删除 |
| `rust/aegis/src/gateways/mod.rs`、`src/main/mod.rs` | 模块声明 | 删除 `pub mod discord;` |
| `rust/aegis/src/common/trait.rs` | `Platform` 枚举与能力位 | 删 `Platform::Discord`、`PlatformCapabilities::DISCORD` |
| `rust/aegis/src/main.rs` | CLI 平台选择 + 运行时接线 | `--discord` 硬失败 + 接线删除 |
| `rust/aegis/src/main/runtime.rs` | 网关启动 | 删 Discord 网关块 |
| `rust/aegis/src/app/state.rs` | 管理员身份 | 删 `discord_admin_id` |
| `rust/aegis/src/main/config.rs` | 解密与加载 | 删 discord 字段 + 兼容回归测试 |
| `rust/aegis/src/bootstrap.rs` | 加密配置与 setup | 删 discord 字段 |
| `rust/aegis/src/main/cli.rs` | `--setup` 调用 `run_setup` | 实参同步 |
| `rust/aegis/Cargo.toml` | 依赖 | `cargo remove serenity poise` |
| `go/installer/main.go` | 安装器全部逻辑 | 删 Discord 面 + 硬失败 |
| `go/installer/main_test.go` | 安装器测试 | 同步 + 新增硬失败用例 |
| `go/installer/i18n/{zh,en,ja}.json` | 安装器文案 | 删 17 键 + 加 `install.discord_removed` |
| `README.md` | 平台清单 | 删 Discord 行 |

---

### Task 1: Rust — `--discord` 硬失败与 Discord 平台整删

**Files:**
- Delete: `rust/aegis/src/gateways/discord/mod.rs`、`rust/aegis/src/gateways/discord/adapter.rs`、`rust/aegis/src/main/discord.rs`
- Modify: `rust/aegis/src/gateways/mod.rs`、`rust/aegis/src/main/mod.rs`、`rust/aegis/src/common/trait.rs`、`rust/aegis/src/main.rs`、`rust/aegis/src/main/runtime.rs`、`rust/aegis/src/app/state.rs`、`rust/aegis/src/gateways/matrix/adapter.rs`、`rust/aegis/src/shared/{dispatch,state_ops,destruct,commands}.rs`
- Test: `rust/aegis/src/main.rs`（`platform_selection_tests` 模块）

**Interfaces:**
- Consumes: 无
- Produces:
  - `fn resolve_platform_selection(args: &[String], has_matrix: bool, has_simplex: bool) -> Result<PlatformSelection, String>`（行为变更：含 `--discord` 时返回 Err）
  - `struct PlatformSelection { telegram: bool, matrix: bool, simplex: bool }`（删 `discord` 字段）
  - `enum Platform { Telegram, Matrix, Simplex }`
  - `AppState::new(admin_id, simplex_admin_id, totp_manager, self_destruct_executor, self_destruct_key_hash, session_timeout_secs, adapter)`（7 参）
  - `runtime::run(state, matrix_handle, enable_telegram, enable_matrix, simplex_handle, token, admin_id)`（7 参）

- [ ] **Step 1: 写失败测试（替换整个 `platform_selection_tests` 模块）**

把 `rust/aegis/src/main.rs` 中从 `#[cfg(test)]\nmod platform_selection_tests {` 到该模块结束的 `}` 整段，替换为：

```rust
#[cfg(test)]
mod platform_selection_tests {
    use super::{PlatformSelection, resolve_platform_selection};

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn sel(telegram: bool, matrix: bool, simplex: bool) -> PlatformSelection {
        PlatformSelection {
            telegram,
            matrix,
            simplex,
        }
    }

    #[test]
    fn no_flags_no_config_is_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, false),
            Ok(sel(true, false, false))
        );
    }

    #[test]
    fn no_flags_matrix_config_is_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), true, false),
            Ok(sel(false, true, false))
        );
    }

    #[test]
    fn no_flags_simplex_config_is_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&[]), false, true),
            Ok(sel(false, false, true))
        );
    }

    #[test]
    fn no_flags_both_configs_is_error() {
        assert!(resolve_platform_selection(&v(&[]), true, true).is_err());
    }

    #[test]
    fn matrix_flag_wins_over_simplex_config() {
        assert_eq!(
            resolve_platform_selection(&v(&["--matrix"]), false, true),
            Ok(sel(false, true, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--matrix"]), true, true),
            Ok(sel(false, true, false))
        );
    }

    #[test]
    fn simplex_flag_suppresses_matrix_and_telegram() {
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, true),
            Ok(sel(false, false, true))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--simplex"]), true, false),
            Ok(sel(false, false, true))
        );
    }

    #[test]
    fn discord_flag_is_rejected() {
        let err = resolve_platform_selection(&v(&["--discord"]), false, false)
            .expect_err("--discord 必须被拒绝");
        assert!(
            err.contains("已移除"),
            "错误信息应说明平台已移除，实际: {err}"
        );

        let err = resolve_platform_selection(&v(&["--all", "--discord"]), true, true)
            .expect_err("--discord 与其它 flag 组合也必须被拒绝");
        assert!(
            err.contains("已移除"),
            "错误信息应说明平台已移除，实际: {err}"
        );
    }

    #[test]
    fn all_flag_is_telegram_plus_matrix() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), true, true),
            Ok(sel(true, true, false))
        );
        assert_eq!(
            resolve_platform_selection(&v(&["--all"]), false, false),
            Ok(sel(true, true, false))
        );
    }

    #[test]
    fn tg_only_flag_is_telegram_only() {
        assert_eq!(
            resolve_platform_selection(&v(&["--tg-only"]), true, true),
            Ok(sel(true, false, false))
        );
    }

    #[test]
    fn all_with_simplex_prefers_simplex() {
        assert_eq!(
            resolve_platform_selection(&v(&["--all", "--simplex"]), false, false),
            Ok(sel(false, false, true))
        );
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd rust/aegis && cargo nextest run --cargo-profile fast-test platform_selection`
Expected: 编译失败（`PlatformSelection` 尚无 `discord` 字段可省 / `sel` 参数不匹配）+ `discord_flag_is_rejected` 逻辑上未实现。此步仅确认测试确实针对未实现行为。

> 注：本步骤会因测试代码引用尚不存在的 3 参 `sel` 而编译失败，这是预期的 RED。若为了先拿到可运行的 RED，可临时保留 `sel` 的 4 参签名并只加 `discord_flag_is_rejected`；两种方式都接受，**不要**跳过此步。

- [ ] **Step 3: 在 `resolve_platform_selection` 顶部实现硬失败**

在 `rust/aegis/src/main.rs` 中，把：

```rust
fn resolve_platform_selection(
    args: &[String],
    has_matrix: bool,
    has_simplex: bool,
) -> Result<PlatformSelection, String> {
    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_discord = args.iter().any(|a| a == "--discord");
    let use_simplex = args.iter().any(|a| a == "--simplex");
```

替换为：

```rust
fn resolve_platform_selection(
    args: &[String],
    has_matrix: bool,
    has_simplex: bool,
) -> Result<PlatformSelection, String> {
    // Discord 平台已移除：显式拒绝，而不是静默落到「无 flag → 自动探测」换平台启动。
    if args.iter().any(|a| a == "--discord") {
        return Err(
            "Discord 平台已移除，本版本不再支持 --discord。请改用 --matrix / --simplex / \
             --tg-only，或从 systemd 单元中移除 --discord。"
                .to_string(),
        );
    }

    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_simplex = args.iter().any(|a| a == "--simplex");
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd rust/aegis && cargo nextest run --cargo-profile fast-test platform_selection`
Expected: PASS（`discord_flag_is_rejected`、`all_with_simplex_prefers_simplex` 等全绿）

- [ ] **Step 5: 删除 Discord 模块与模块声明**

```bash
cd rust/aegis
git rm src/gateways/discord/mod.rs src/gateways/discord/adapter.rs src/main/discord.rs
```

删除 `rust/aegis/src/gateways/mod.rs` 中的 `pub mod discord;` 一行。
删除 `rust/aegis/src/main/mod.rs` 中的 `pub mod discord;` 一行。

- [ ] **Step 6: 删除 `Platform::Discord` 与 `PlatformCapabilities::DISCORD`**

`rust/aegis/src/common/trait.rs`：

```rust
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
    Simplex,
}
```

改为：

```rust
pub enum Platform {
    Telegram,
    Matrix,
    Simplex,
}
```

删除以下整块（`PlatformCapabilities::DISCORD` 常量）：

```rust
    pub const DISCORD: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: false,
        can_send_file: true,
        can_send_image: true,
        can_send_voice: false,
        can_send_typing: true,
        can_send_reaction: true,
        can_thread: true,
        has_e2ee: false,
    };

```

`rust/aegis/src/gateways/matrix/adapter.rs` 中：

```rust
        assert_ne!(Platform::Matrix, Platform::Discord);
```

改为：

```rust
        assert_ne!(Platform::Matrix, Platform::Simplex);
```

- [ ] **Step 7: 删除 `main.rs` 的 Discord 接线**

`PlatformSelection` 结构体删 `discord: bool,` 字段：改为

```rust
struct PlatformSelection {
    telegram: bool,
    matrix: bool,
    simplex: bool,
}
```

删除决策表注释中的 `--discord` 相关行与 `--discord + --simplex` 行，并把 `--all` 行改为：

```rust
/// | `--all` | 任意 | 任意 | telegram + matrix（永不包含 simplex） |
```

（同时在决策表上方补一行：`/// 含 `--discord` 的参数组合直接报错（平台已移除）。`）

删除 `discord_raw` 构建块（整段，含尾部空行）：

```rust
    let discord_raw = if selection.discord {
        Some(
            main::discord::connect_discord(
                &security,
                &app_config.decrypted.encrypted_config,
                &config_dir(),
            )
            .await?,
        )
    } else {
        None
    };

```

adapter 选择改为：

```rust
    let adapter = if let Some(ref handle) = simplex_handle {
        handle.adapter.clone()
    } else {
        main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            selection.telegram,
            selection.matrix,
            &matrix_handle,
        )
        .await?
    };
```

`AppState::new` 实参删除第 2 个：

```rust
    let state = Arc::new(AppState::new(
        app_config.decrypted.admin_id,
        app_config.decrypted.simplex_admin_id,
```

`runtime::run` 实参删除 `discord_raw,` 一行（`selection.matrix,` 之后、`simplex_handle,` 之前）。

`resolve_platform_selection` 函数体删除 `use_discord` 相关分支：删除

```rust
    if use_discord && use_simplex {
        return Err(
            "--discord 与 --simplex 都是独立平台，不能同时启用。请只保留其一。".to_string(),
        );
    }

    if use_discord {
        return Ok(PlatformSelection {
            telegram: false,
            matrix: false,
            discord: true,
            simplex: false,
        });
    }
```

并把其余 5 处 `Ok(PlatformSelection { ... })` 字面量里的 `discord: false,` 行逐条删除（`use_simplex` / `use_matrix` / `use_all` / `tg_only` 分支，以及末尾 `match (has_matrix, has_simplex)` 的 4 个分支）。

- [ ] **Step 8: 删除 `runtime.rs` 的 Discord 网关块**

`rust/aegis/src/main/runtime.rs`：

1. `pub async fn run(` 形参删除 `discord_raw: Option<super::discord::DiscordRawHandle>,` 一行。
2. 删除整段 `// ── Discord 网关 ──` 块（从该注释到 `    }` 结束，即 `let discord_enabled = discord_raw.is_some();` 起到 `if let Some(raw) = discord_raw { ... }` 结束）。
3. 文件末尾 SimpleX 保活条件：

```rust
    if simplex_enabled && !enable_telegram && !enable_matrix && !discord_enabled {
```

改为：

```rust
    if simplex_enabled && !enable_telegram && !enable_matrix {
```

- [ ] **Step 9: 删除 `AppState` 的 `discord_admin_id`**

`rust/aegis/src/app/state.rs`：

1. 结构体删除 `discord_admin_id: Option<i64>,` 一行。
2. `pub fn new(` 删除第 2 个形参 `discord_admin_id: Option<i64>,`，并删除 `Self { ... }` 初始化中的 `discord_admin_id,` 一行。
3. `is_admin_user` 改为：

```rust
    pub fn is_admin_user(&self, user_id: i64) -> bool {
        user_id == self.admin_id.unwrap_or(0) || self.simplex_admin_id == Some(user_id)
    }
```

4. 删除整个测试 `discord_admin_id_is_recognized_as_admin`（`#[tokio::test]` 到其结束 `}`）。
5. 文件内所有 `AppState::new(` 调用点删除第 2 个实参（`Some(999)` / `None` 之类）。

- [ ] **Step 10: 修掉其余 `AppState::new` 调用点**

以下文件中的每个 `AppState::new(...)` 调用删除第 2 个实参：

```bash
cd rust/aegis && grep -rln 'AppState::new(' src/
# src/shared/dispatch.rs  src/shared/state_ops.rs  src/shared/destruct.rs
# src/shared/commands.rs  src/main.rs  src/app/state.rs
```

- [ ] **Step 11: 编译并跑完整质量门**

Run:

```bash
cd rust/aegis && cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

Expected: 全绿，零 Clippy warning。

- [ ] **Step 12: 确认无残留引用**

Run:

```bash
cd rust/aegis && grep -rn 'Discord\|discord' src/ --include='*.rs' | grep -v resources/sni
```
Expected: 无输出。

- [ ] **Step 13: 提交**

```bash
git add -A rust/aegis
git commit -m "refactor(discord): 移除 Discord 平台运行时与网关"
```

---

### Task 2: Rust — 加密配置字段删除与老配置兼容回归

**Files:**
- Modify: `rust/aegis/src/bootstrap.rs`、`rust/aegis/src/main/config.rs`、`rust/aegis/src/main/cli.rs`、`rust/aegis/src/main/matrix.rs`、`rust/aegis/src/main/simplex.rs`
- Test: `rust/aegis/src/bootstrap.rs`（`config_tests`）、`rust/aegis/src/main/config.rs`（`tests`）

**Interfaces:**
- Consumes: Task 1 的结果（`main/discord.rs` 已删，因此这些字段不再有消费者）
- Produces:
  - `struct EncryptedConfig`（无 `discord_token` / `discord_admin_id`）
  - `struct SetupInput`（同上）
  - `async fn run_setup(token, admin_id, totp_secret, matrix, matrix_recovery_key, simplex_port, simplex_admin_id) -> Result<()>`（7 参）
  - `struct DecryptedConfig`（无 `discord_token` / `discord_admin_id`）

- [ ] **Step 1: 写失败测试（老 `config.enc` 兼容回归）**

在 `rust/aegis/src/main/config.rs` 的 `mod tests` 内、`load_and_validate_builds_working_totp_manager` 之后追加：

```rust
    #[serial]
    #[test]
    fn load_and_validate_ignores_legacy_discord_fields() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("etc/wwps/aegis");
        fs::create_dir_all(&config_dir).unwrap();
        // SAFETY: test environment, single-threaded
        unsafe {
            std::env::set_var("AEGIS_CONFIG_DIR", config_dir.to_str().unwrap());
        }

        let totp_secret = TotpManager::generate_new_secret();
        let security = SecurityManager::new(&config_dir.join(KEY_FILE)).unwrap();

        // 手工构造含已移除 discord 字段的 config.enc：EncryptedConfig 已无这两个字段，
        // 因此必须写原始 JSON，才能覆盖「老部署升级后仍能启动」这一承诺。
        let legacy = serde_json::json!({
            "token": security.encrypt(b"123456:ABCdefGHIjklMNOpqrsTUVwxyz").unwrap(),
            "admin_id": security.encrypt(b"42").unwrap(),
            "totp_secret": security.encrypt(totp_secret.as_bytes()).unwrap(),
            "lang": "zh",
            "discord_token": security.encrypt(b"MTIzLmFiYw").unwrap(),
            "discord_admin_id": security.encrypt(b"123456789").unwrap(),
        });
        fs::write(
            config_dir.join(CONFIG_FILE),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();

        let (app_config, _security) =
            load_and_validate().expect("含 legacy discord 字段的 config.enc 必须仍能加载");
        let manager = app_config.totp_manager.expect("totp_manager 应已构建");
        let code = manager.generate_current();
        assert!(manager.verify(&code));
    }
```

- [ ] **Step 2: 运行测试确认结果**

Run: `cd rust/aegis && cargo nextest run --cargo-profile fast-test load_and_validate_ignores_legacy_discord_fields`
Expected: **PASS**（此刻字段仍在，未知字段忽略由 serde 默认行为保证）。这条测试的作用是**锁定承诺**：Step 3–6 删除字段后它必须继续 PASS；若将来有人给 `EncryptedConfig` 加 `deny_unknown_fields`，它会立刻变红。

- [ ] **Step 3: 删除 `EncryptedConfig` 与 `SetupInput` 的 discord 字段**

`rust/aegis/src/bootstrap.rs`：

1. `pub struct EncryptedConfig` 中删除：

```rust
    #[serde(default)]
    pub discord_token: Option<Vec<u8>>,
    #[serde(default)]
    pub discord_admin_id: Option<Vec<u8>>,
```

2. `pub struct SetupInput` 中删除：

```rust
    #[serde(default)]
    discord_token: Option<String>,
    #[serde(default)]
    discord_admin_id: Option<String>,
```

3. `impl Drop for EncryptedConfig` 中删除：

```rust
        if let Some(v) = &mut self.discord_token {
            v.zeroize();
        }
        if let Some(v) = &mut self.discord_admin_id {
            v.zeroize();
        }
```

- [ ] **Step 4: 删除 `run_setup` 的 discord 参数**

`rust/aegis/src/bootstrap.rs` 中 `pub async fn run_setup(` 形参删除：

```rust
    discord_token: Option<&str>,
    discord_admin_id: Option<&str>,
```

并把函数体内 `let encrypted_config = EncryptedConfig { ... }` 的 `discord_token,` / `discord_admin_id,` 两行删除；同时删除函数体内为这两个字段生成密文的 `let discord_token = ...` / `let discord_admin_id = ...` 语句（若存在）。

- [ ] **Step 5: 同步 `run_setup` 两个调用点**

`rust/aegis/src/bootstrap.rs` 的 `run_setup(` 调用（在 `run_setup_from_stdin` 内）删除实参：

```rust
    let discord_token = input.discord_token.as_deref();
    let discord_admin_id = input.discord_admin_id.as_deref();
```

以及调用处传入的 `discord_token,` / `discord_admin_id,` 两行。

`rust/aegis/src/main/cli.rs` 的 `run_setup(` 调用由 9 个参数改为 7 个（删除第 5、6 个 `None`）：

```rust
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
```

- [ ] **Step 6: 删除 `DecryptedConfig` 的 discord 字段与解密逻辑**

`rust/aegis/src/main/config.rs`：

1. `pub struct DecryptedConfig` 删除：

```rust
    #[expect(dead_code)]
    pub discord_token: Option<String>,
    #[expect(dead_code)]
    pub discord_admin_id: Option<i64>,
```

2. `load_and_validate` 删除两段解密（`let discord_token = match &encrypted_config.discord_token { ... };` 与 `let discord_admin_id = match &encrypted_config.discord_admin_id { ... };` 整块）。
3. 返回处 `decrypted: DecryptedConfig { ... }` 删除 `discord_token,` / `discord_admin_id,` 两行。
4. `mod tests` 内所有 `EncryptedConfig { ... }` 字面量删除 `discord_token: None,` / `discord_admin_id: None,` 两行（含 Step 1 新增测试里未涉及的那些）。

- [ ] **Step 7: 同步测试字面量**

`rust/aegis/src/bootstrap.rs` 的 `mod config_tests`：

1. 删除整个 `discord_config_fields_round_trip` 测试。
2. `simplex_config_fields_round_trip` 字面量删除 `discord_token: None,` / `discord_admin_id: None,` 两行。

`rust/aegis/src/main/matrix.rs`（3 处）与 `rust/aegis/src/main/simplex.rs`（1 处）的 `EncryptedConfig { ... }` 字面量删除 `discord_token: None,` / `discord_admin_id: None,` 两行。

- [ ] **Step 8: 跑完整质量门**

Run:

```bash
cd rust/aegis && cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

Expected: 全绿，且 `load_and_validate_ignores_legacy_discord_fields` 仍 PASS。

- [ ] **Step 9: 确认无残留引用**

Run:

```bash
cd rust/aegis && grep -rn 'Discord\|discord' src/ --include='*.rs' | grep -v resources/sni
```
Expected: 无输出。

- [ ] **Step 10: 提交**

```bash
git add -A rust/aegis
git commit -m "refactor(discord): 删除加密配置中的 Discord 字段并锁定老配置兼容"
```

---

### Task 3: Rust — 依赖清理

**Files:**
- Modify: `rust/aegis/Cargo.toml`（仅通过 `cargo remove`）、`rust/aegis/Cargo.lock`

**Interfaces:**
- Consumes: Task 1（`serenity` 的唯一使用者已删除）
- Produces: 无（依赖图收缩）

- [ ] **Step 1: 确认依赖确已无使用者**

Run:

```bash
cd rust/aegis && grep -rn 'serenity\|poise' src/ tests/ examples/ --include='*.rs'
```
Expected: 无输出（`poise` 在 `src/resources/sni/*.pb` 里的同名词不算代码引用）。

- [ ] **Step 2: 移除依赖**

Run:

```bash
cd rust/aegis && cargo remove serenity && cargo remove poise
```
Expected: 两条命令各自打印 `Removing serenity from dependencies` / `Removing poise from dependencies`。

- [ ] **Step 3: 更新 Cargo.toml 注释**

把 `rust/aegis/Cargo.toml` 中 `fast-test` profile 上方注释里的：

```toml
# 不挂 cranelift：aws-lc-sys（rustls 的 C 依赖，被 matrix-sdk/serenity 强制启用）
```

改为：

```toml
# 不挂 cranelift：aws-lc-sys（rustls 的 C 依赖，被 matrix-sdk 强制启用）
```

- [ ] **Step 4: 验证依赖图与质量门**

Run:

```bash
cd rust/aegis && cargo tree | grep -E 'serenity|poise' ; \
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```
Expected: `cargo tree | grep` 无输出；质量门全绿。

- [ ] **Step 5: 提交**

```bash
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock
git commit -m "build(aegis): 移除 serenity 与 poise 依赖"
```

---

### Task 4: Go — 安装器 Discord 删除面与硬失败

**Files:**
- Modify: `go/installer/main.go`、`go/installer/main_test.go`、`go/installer/i18n/{zh,en,ja}.json`
- Test: `go/installer/main_test.go`

**Interfaces:**
- Consumes: 无
- Produces（后续任务/调用点依赖的签名）:
  - `func platformSelector.platformSelection() (telegram, matrix, simplex, valid bool)`
  - `func parsePlatformChoice(choice string) (tg, matrix, simplex bool, err error)`
  - `func selectDeploymentPlatforms() (tg, matrix, simplex bool, err error)`
  - `func platformSetupForChoice(choice string) (tg, matrix, simplex bool, err error)`
  - `func servicePlatformForSetup(tg, matrix, simplex bool) string`
  - `func buildSetupPayload(token, adminID, totpSecret []byte, matrixHS, matrixUser, matrixRoom string, matrixPass, matrixStorePassphrase []byte, matrixRecoveryKey, simplexPort, simplexAdminID string) []byte`
  - `func recoveryPlatformForService(service []byte, choice string) (string, bool, error)`（服务单元含 `--discord` 时返回错误）
  - i18n 新键 `install.discord_removed`

- [ ] **Step 1: 新增失败测试**

在 `go/installer/main_test.go` 追加：

```go
func TestDiscordPlatformIsRejected(t *testing.T) {
	t.Run("keyval discord_token", func(t *testing.T) {
		_, err := parseKeyVal([]byte("discord_token=abc\ndiscord_admin_id=123\n"))
		if err == nil {
			t.Fatal("parseKeyVal 含 discord_token 时必须报错")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("choice 3 is reserved", func(t *testing.T) {
		_, _, _, err := platformSetupForChoice("3")
		if err == nil {
			t.Fatal("编号 3 必须报错（保留空洞）")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("existing unit with --discord", func(t *testing.T) {
		_, _, err := recoveryPlatformForService([]byte("ExecStart=/a --discord\n"), "")
		if err == nil {
			t.Fatal("既有单元含 --discord 时必须报错，不得静默换成其它平台")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("parsePlatformChoice rejects discord", func(t *testing.T) {
		if _, _, _, err := parsePlatformChoice("discord"); err == nil {
			t.Fatal("parsePlatformChoice(\"discord\") 必须报错")
		}
		if _, _, _, err := parsePlatformChoice("telegram + discord"); err == nil {
			t.Fatal("parsePlatformChoice(\"telegram + discord\") 必须报错")
		}
	})
}
```

若 `main_test.go` 尚未导入 `strings`，在 import 块中补上。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd go/installer && go test ./... -run TestDiscordPlatformIsRejected`
Expected: FAIL —— `parseKeyVal` 目前只打黄色警告；`platformSetupForChoice("3")` 返回 `discord=true`；`recoveryPlatformForService` 返回 `"discord"`；`parsePlatformChoice("discord")` 返回成功。

- [ ] **Step 3: 添加 i18n 键 `install.discord_removed`（三语）**

在 `go/installer/i18n/zh.json`、`en.json`、`ja.json` 各自加入一个键（`zh` 是基准，必须与 `en`/`ja` 同步）：

```json
"install.discord_removed": "Discord 平台已移除，本版本不再支持。请改用 Matrix（--matrix）、SimpleX（--simplex）或 Telegram，并移除 systemd 单元中的 --discord。"
```

```json
"install.discord_removed": "The Discord platform has been removed. Use Matrix (--matrix), SimpleX (--simplex) or Telegram instead, and remove --discord from the systemd unit."
```

```json
"install.discord_removed": "Discord プラットフォームは削除されました。Matrix（--matrix）、SimpleX（--simplex）、Telegram のいずれかを使用し、systemd ユニットから --discord を削除してください。"
```

- [ ] **Step 4: 删除 `platformSelector` 的 Discord 项**

`go/installer/main.go`：

1. 结构体改为：

```go
type platformSelector struct {
	cursor                             int
	telegram, matrix, simplex, confirmed bool
}
```

2. 键位处理改为：

```go
	case "up":
		m.cursor = (m.cursor + 2) % 3
	case "down":
		m.cursor = (m.cursor + 1) % 3
	case "space":
		switch m.cursor {
		case 0:
			m.telegram = !m.telegram
			if m.telegram {
				m.simplex = false
			}
		case 1:
			m.matrix = !m.matrix
			if m.matrix {
				m.simplex = false
			}
		case 2:
			m.simplex = !m.simplex
			if m.simplex {
				m.telegram = false
				m.matrix = false
			}
		}
	case "enter":
		if _, _, _, valid := m.platformSelection(); valid {
			m.confirmed = true
			return m, tea.Quit
		}
```

（原 `case "space"` 中的 `m.discord` 分支与 `case "enter"` 的 5 值解构同时删除/改为 4 值。）

3. 选择判定改为：

```go
func (m platformSelector) platformSelection() (bool, bool, bool, bool) {
	// SimpleX 为独立平台：不得与 Telegram / Matrix 组合。
	valid := (m.telegram || m.matrix || m.simplex) &&
		!(m.simplex && (m.telegram || m.matrix))
	return m.telegram, m.matrix, m.simplex, valid
}
```

4. `View()` 的 labels 与 choices 改为：

```go
	labels := []string{
		i18n.T("firsttime.platform_selector_telegram"),
		i18n.T("firsttime.platform_selector_matrix"),
		i18n.T("firsttime.platform_selector_simplex"),
	}
	selected := make([]string, 0, 2)
	choices := []bool{m.telegram, m.matrix, m.simplex}
```

- [ ] **Step 5: 删除 `parsePlatformChoice` 的 Discord 分支**

```go
func parsePlatformChoice(choice string) (bool, bool, bool, error) {
	switch strings.ToLower(strings.ReplaceAll(strings.TrimSpace(choice), " ", "")) {
	case "telegram":
		return true, false, false, nil
	case "matrix":
		return false, true, false, nil
	case "simplex":
		return false, false, true, nil
	case "telegram+matrix":
		return true, true, false, nil
	default:
		return false, false, false, fmt.Errorf("invalid platform")
	}
}
```

`selectDeploymentPlatforms` 全函数改为：

```go
func selectDeploymentPlatforms() (bool, bool, bool, error) {
	if !usesInteractivePlatformSelector(term.IsTerminal(int(os.Stdin.Fd())), term.IsTerminal(int(os.Stdout.Fd()))) {
		fmt.Print(i18n.T("firsttime.platform_text_prompt"))
		choice, err := readLine()
		if err != nil {
			return false, false, false, err
		}
		return parsePlatformChoice(choice)
	}

	model, err := tea.NewProgram(newPlatformSelector()).Run()
	if err != nil {
		return false, false, false, err
	}
	selector := model.(platformSelector)
	tg, matrix, simplex, valid := selector.platformSelection()
	if !selector.confirmed || !valid {
		return false, false, false, fmt.Errorf("platform selection cancelled")
	}
	return tg, matrix, simplex, nil
}
```

- [ ] **Step 6: 删除交互式 Discord 输入段**

删除 `main.go` 中整块：

```go
	// ── Discord section ──
	var discordToken, discordAdminID string
	if enableDiscord {
		...
	}
```

（含两次 `readSecureInputStr` 与全部 `discord_*` i18n 打印）。同时删除同函数中 `enableDiscord` 的解构（第 1625 行 `enableTG, enableMatrix, enableDiscord, enableSimplex, err := selectDeploymentPlatforms()` 改为 `enableTG, enableMatrix, enableSimplex, err := selectDeploymentPlatforms()`），并同步该函数末尾 `servicePlatformForSetup(enableTG, enableMatrix, enableDiscord, enableSimplex)` 的实参，以及 `buildSetupPayload(...)` 调用中 `discordToken, discordAdminID,` 两个实参与其上方的 `var discordToken, discordAdminID string` 声明。

- [ ] **Step 7: 删除 `buildSetupPayload` 的 Discord 输出**

签名改为：

```go
func buildSetupPayload(token, adminID, totpSecret []byte, matrixHS, matrixUser, matrixRoom string, matrixPass, matrixStorePassphrase []byte, matrixRecoveryKey, simplexPort, simplexAdminID string) []byte {
```

删除函数体内两段：

```go
	if discordToken != "" {
		...
		buf = append(buf, []byte(`"discord_token":`)...)
		buf = appendJSONEscaped(buf, []byte(discordToken))
	}
	if discordAdminID != "" {
		...
		buf = append(buf, []byte(`"discord_admin_id":`)...)
		buf = appendJSONEscaped(buf, []byte(discordAdminID))
	}
```

同步全部 3 个调用点（交互式路径、`installFromKeyVal`、以及 `main.go:1819` 附近那处）。

- [ ] **Step 8: `parseKeyVal` 硬失败并删除字段**

`setupConfig` 结构体删除 `DiscordToken` / `DiscordAdminID` 字段；`parseKeyVal` 的 `switch` 中把：

```go
		case "discord_token":
			cfg.DiscordToken = val
		case "discord_admin_id":
			cfg.DiscordAdminID = val
```

改为：

```go
		case "discord_token", "discord_admin_id":
			return nil, fmt.Errorf("%s", i18n.T("install.discord_removed"))
```

必填字段校验改为：

```go
	if cfg.Token == "" && cfg.MatrixHS == "" && cfg.SimplexPort == "" {
		return nil, fmt.Errorf("缺少必填字段: 至少需要配置 Telegram (token/admin_id)、Matrix (matrix_homeserver) 或 SimpleX (simplex_port/simplex_admin_id) 之一")
	}
```

- [ ] **Step 9: 删除三处 `platform = "discord"` 判定**

`installFromKeyVal`（约 :1563）：

```go
	platform := "tg"
	if cfg.Token == "" {
		if cfg.SimplexPort != "" {
			platform = "simplex"
		} else if cfg.MatrixHS != "" {
			platform = "matrix"
		}
	}
```

`installFromStdin`（约 :1432）——把 discord 分支替换为硬失败：

```go
	platform := "tg"
	simplexPort, _ := inputData["simplex_port"].(string)
	if _, ok := inputData["discord_token"]; ok {
		printRed(i18n.T("install.discord_removed"))
		os.Exit(1)
	}
	if _, ok := inputData["simplex_port"].(string); ok {
		platform = "simplex"
	} else if _, ok := inputData["matrix_homeserver"].(string); ok {
		if token, ok := inputData["token"].(string); ok && token != "" {
			platform = "tg-matrix"
		} else {
			platform = "matrix"
		}
	}
```

- [ ] **Step 10: `platformSetupForChoice` 保留编号空洞**

```go
func platformSetupForChoice(choice string) (tg, matrix, simplex bool, err error) {
	switch choice {
	case "1":
		return true, false, false, nil
	case "2":
		return false, true, false, nil
	case "3":
		// 编号 3 原为 Discord：保留空洞，避免旧脚本静默落到别的平台。
		return false, false, false, fmt.Errorf("%s", i18n.T("install.discord_removed"))
	case "4":
		return true, true, false, nil
	case "5":
		return false, false, true, nil
	default:
		return false, false, false, fmt.Errorf("invalid platform")
	}
}
```

`servicePlatformForSetup` 改为：

```go
func servicePlatformForSetup(tg, matrix, simplex bool) string {
	switch {
	case simplex:
		return "simplex"
	case tg && matrix:
		return "tg-matrix"
	case tg:
		return "tg"
	case matrix:
		return "matrix"
	default:
		return ""
	}
}
```

- [ ] **Step 11: 升级路径共享入口拦截既有 `--discord` 单元**

`recoveryPlatformForService` 改为：

```go
func recoveryPlatformForService(service []byte, choice string) (string, bool, error) {
	if len(service) > 0 {
		// 既有单元带 --discord：硬失败，否则会退化成 tg 默认单元静默换平台。
		if bytes.Contains(service, []byte("--discord")) {
			return "", false, fmt.Errorf("%s", i18n.T("install.discord_removed"))
		}
		return platformFromService(service), false, nil
	}
	tg, matrix, simplex, err := platformSetupForChoice(choice)
	if err != nil {
		return "", false, err
	}
	return servicePlatformForSetup(tg, matrix, simplex), true, nil
}
```

`platformFromService` 删除 `case bytes.Contains(service, []byte("--discord")): return "discord"` 两行。

- [ ] **Step 12: 让 `installAegis` 不再吞掉恢复路径的错误**

`installAegis` 中：

```go
		} else {
			platform, _, _ = recoveryPlatformForService(service, "")
		}
```

改为：

```go
		} else {
			var err error
			platform, _, err = recoveryPlatformForService(service, "")
			if err != nil {
				printRed(err.Error())
				return
			}
		}
```

同时把相邻的：

```go
			if err != nil {
				printRed(i18n.T("firsttime.platform_invalid"))
				return
			}
```

改为：

```go
			if err != nil {
				printRed(err.Error())
				return
			}
```

- [ ] **Step 13: 删除服务单元与 flag 映射中的 Discord**

`platformFlagFor` 删除：

```go
	case "discord":
		return "--discord"
```

`writeSystemdService` 删除：

```go
	case "discord":
		descName = "WWPS Discord Bot"
```

- [ ] **Step 14: 同步 `main_test.go` 既有用例**

1. `TestPlatformFromService`：删除 `"ExecStart=/etc/wwps/aegis/aegis --discord": "discord",` 一行。
2. `TestPlatformSetupForChoice`：表驱动用例由 4 值改为 3 值；`"3": {discord: true}` 删除；断言行删除 discord 维；`platformSetupForChoice("0")` 的期望错误保留。
3. `TestServicePlatformForSetup`：结构体去 discord 字段、用例 `{discord: true, want: "discord"}` 与 `{matrix: true, discord: true, want: "discord"}` 删除、`servicePlatformForSetup` 调用改 3 参。
4. `TestParsePlatformChoice`（含 :816 的 `"telegram + discord"` 断言）：`telegram`/`matrix`/`simplex`/`telegram + matrix` 用例改为 3 值解构；`discord` 与 `telegram + discord` 断言改为期望错误。
5. 交互选择器用例（:731、:741、:746 附近）：删除 `m.discord` 断言，`TestPlatformSelectorMakesTelegramAndDiscordExclusive` 改为断言「选中 simplex 会清掉 Telegram 与 Matrix」并改名 `TestPlatformSelectorSimplexIsStandalone`。
6. `parseKeyVal` 的 `with discord fields` 子测试（:576）删除（硬失败已由 `TestDiscordPlatformIsRejected` 覆盖）；`with discord fields` 的 payload 子测试（:372）删除；`without discord fields` 子测试保留并去掉 discord 断言。

- [ ] **Step 15: 跑 Go 质量门**

Run:

```bash
cd go/installer && gofmt -l . && go vet ./... && go test ./... && staticcheck ./...
```
Expected: `gofmt -l` 无输出；其余全绿（含 Step 1 新增的 `TestDiscordPlatformIsRejected`）。

- [ ] **Step 16: 提交**

```bash
git add go/installer/main.go go/installer/main_test.go go/installer/i18n
git commit -m "refactor(installer): 移除 Discord 平台并硬失败残留配置"
```

---

### Task 5: Go — 删除废弃的 Discord i18n 键

**Files:**
- Modify: `go/installer/i18n/{zh,en,ja}.json`
- Test: `go/installer/i18n/i18n_test.go`（既有 `TestAllKeysExist` / `TestNoDuplicateKeys`，无需改动）

**Interfaces:**
- Consumes: Task 4（代码已不再引用这些键）
- Produces: 无

- [ ] **Step 1: 确认键已无代码引用**

Run:

```bash
cd go/installer && grep -rn 'discord' --include='*.go' . | grep -v 'discord_removed'
```
Expected: 无输出（`install.discord_removed` 是保留的新键）。

- [ ] **Step 2: 删除 17 个键（三语）**

从 `go/installer/i18n/zh.json`、`en.json`、`ja.json` 各删除：

```
firsttime.platform_selector_discord
firsttime.discord_section
firsttime.discord_desc1
firsttime.discord_desc2
firsttime.discord_prompt_yn
firsttime.discord_token_title
firsttime.discord_token_help_step1
firsttime.discord_token_help_step2
firsttime.discord_token_help_format
firsttime.discord_token_prompt
firsttime.discord_admin_title
firsttime.discord_admin_help_step1
firsttime.discord_admin_help_step2
firsttime.discord_admin_help_format
firsttime.discord_admin_prompt
firsttime.discord_intent_warning
firsttime.discord_guild_warning
```

删除后每个文件应剩 174 - 17 + 1 = **158** 个键。

- [ ] **Step 3: 检查相邻文案是否提及 Discord**

Run:

```bash
cd go/installer && grep -rn -i 'discord' i18n/ | grep -v 'discord_removed'
```
Expected: 无输出。若有输出（例如 `platform_selector_help` 提到 Discord），改写该文案去掉 Discord。

- [ ] **Step 4: 验证 JSON 与键集一致性**

Run:

```bash
cd go/installer && python3 -c "
import json
for f in ['zh','en','ja']:
    d=json.load(open('i18n/%s.json'%f)); print(f, len(d))
" && go test ./i18n/ -v -run 'TestAllKeysExist|TestNoDuplicateKeys'
```
Expected: 三语键数均为 158；两个测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add go/installer/i18n
git commit -m "chore(installer): 删除废弃的 Discord 文案"
```

---

### Task 6: 文档 — README 平台清单

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: 无
- Produces: 无

- [ ] **Step 1: 删除 README 中的 Discord 引用**

`README.md` 中：

```markdown
- bot-driven operations and maintenance workflows over Telegram, Matrix, Discord, or SimpleX
```

改为：

```markdown
- bot-driven operations and maintenance workflows over Telegram, Matrix, or SimpleX
```

平台表删除整行：

```markdown
| Discord | `--discord` | standalone |
```

- [ ] **Step 2: 验证**

Run:

```bash
grep -rn -i 'discord' README.md
```
Expected: 无输出。

- [ ] **Step 3: 提交**

```bash
git add README.md
git commit -m "docs(readme): 平台清单移除 Discord"
```

---

## Self-Review

**1. Spec coverage**

| Spec 章节 | 对应任务 |
|---|---|
| §2.1 `--discord` 硬失败 | Task 1 Step 3 + Task 1 Step 1 测试 |
| §2.2 `config.enc` 直接删字段 | Task 2 Step 3–7 |
| §2.2 老配置兼容回归测试 | Task 2 Step 1–2、Step 8 |
| §2.3 installer keyval 硬失败 | Task 4 Step 1、Step 8 |
| §2.4 installer 编号保留空洞 | Task 4 Step 1、Step 10 |
| §3.1 整文件删除 | Task 1 Step 5 |
| §3.2 逐点修改表 | Task 1 Step 6–10、Task 2 Step 3–7 |
| §3.3 依赖清理 | Task 3 |
| §4 installer 改动表 | Task 4 Step 4–13 |
| §5 i18n 17 键删除 | Task 5 |
| §6 README | Task 6 |
| §7 测试策略 | 各任务 Step 1–2 与质量门 Step |
| §8 验收标准 | Task 6 Step 2 + 下方「最终验收」 |
| §9 风险（deny_unknown_fields 回归） | Task 2 Step 1 测试 |

**2. Placeholder scan:** 无 TBD/TODO；所有删除点均给出被删代码原文，所有新增代码均为完整可编译片段。

**3. Type consistency:** `PlatformSelection`（3 字段：telegram/matrix/simplex）在 Task 1 的测试与实现中一致；`AppState::new` 7 参在 Task 1 Step 9–10 与调用点同步；Go 侧 `platformSelection()`/`parsePlatformChoice`/`selectDeploymentPlatforms`/`platformSetupForChoice` 一律 3 平台 + `err`，`servicePlatformForSetup` 3 参，在 Task 4 的各 Step 与 Step 14 测试同步中一致。

---

## 最终验收（所有任务完成后）

- [ ] `grep -rn 'discord\|Discord' rust/aegis/src go/installer --include='*.rs' --include='*.go' --include='*.json'` 零命中（`resources/sni/*.pb` 域名词表除外；`install.discord_removed` 是唯一允许的命中）
- [ ] `cd rust/aegis && cargo tree | grep -E 'serenity|poise'` 无输出
- [ ] Rust 与 Go 质量门各自全绿
- [ ] 手工验证：`./aegis --discord` 非零退出并打印「Discord 平台已移除」
- [ ] 手工验证：`echo 'discord_token=x' | ./installer` 报错退出
- [ ] `README.md` 无 Discord
- [ ] 历史文档（`docs/2026-09-16-simplex-platform.md`、`docs/superpowers/{specs,plans}/2026-09-16-simplex-platform*.md`）未被修改
