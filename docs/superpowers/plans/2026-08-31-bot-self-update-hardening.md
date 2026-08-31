# Bot 自更新加固 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `aegis` bot 自更新的 6 个缺陷：minisign 静默降级、60s 超时、固定临时文件竞态、无版本比较、无并发锁、部署不可测/无回滚点。

**Architecture:** 保持现有「chat 回调 → `UpgradeManager::run()`」流程，但把部署段抽成可注入的 `DeployStrategy` trait（生产实现 `ProductionDeploy`：冒烟测试 → `.bak` 备份 → `fs::rename` 原子替换 → systemd/reexec 双重启策略），其余组件（锁、版本比较、临时文件、超时、minisign 强制）以独立小函数 + RAII 守卫实现，全部可单测；`run()` 全链路用 wiremock + `MockBotAdapter` + `MockDeployStrategy` 集成测试。

**Tech Stack:** Rust 2024 edition, tokio, reqwest, anyhow, async-trait, wiremock (dev), serial_test (dev, 已有), mockall (已有)。

**Spec:** `docs/superpowers/specs/2026-08-31-bot-self-update-hardening-design.md`

## Global Constraints

- Rust 2024 edition：`std::env::set_var` 是 `unsafe`（测试中必须包 `unsafe {}`）。
- 所有新增 env 测试用 `serial_test`（已有 dev-dep）避免并行污染。
- 错误处理用 `anyhow` + `.context()`，禁止 `unwrap()`（`err-anyhow-app`, `err-context-chain`, `err-no-unwrap-prod`）。
- async 代码用 `tokio::fs` 而非 `std::fs`（`async-tokio-fs`）；CPU 密集（冒烟测试子进程）用 `tokio::process`。
- 不新增运行时依赖（wiremock/tempfile 仅 dev-deps；临时文件名自生成，不引 `tempfile` crate）。
- 每个任务结束必须跑 `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`（rust-lint-format 强制），通过后才提交。
- 工作目录：仓库根。aegis crate 位于 `rust/aegis/`。

---

### Task 1: 测试依赖 + Release API base 可配置 + client 注入

**Files:**
- Modify: `rust/aegis/Cargo.toml`（dev-deps 加 `wiremock`）
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: `rust/aegis/src/core/system/upgrade.rs`（`#[cfg(test)]` 模块内）

**Interfaces:**
- Consumes: `fetch_json_from_mirrors(&client, bases, api_path, token)`（`release_api.rs` 现有）
- Produces:
  - `fn configured_release_api_bases() -> Vec<String>` — env `AEGIS_RELEASE_API_BASES`（逗号分隔）覆盖默认 `https://api.github.com`
  - `impl UpgradeManager { pub fn new_with_client(client: reqwest::Client) -> Result<Self> }` — 注入 client + bases 的构造器；`new()` 委托
  - `UpgradeManager` 新增字段 `bases: Vec<String>`（`fetch_latest_release` 改用 `&self.bases`）

- [ ] **Step 1: 写失败测试**（`upgrade.rs` 的 `mod tests` 顶部添加）

```rust
#[test]
fn test_configured_release_api_bases_default() {
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
    let bases = configured_release_api_bases();
    assert_eq!(bases, vec!["https://api.github.com".to_string()]);
}

#[test]
fn test_configured_release_api_bases_env_override() {
    unsafe { std::env::set_var("AEGIS_RELEASE_API_BASES", "http://127.0.0.1:8080, http://mirror.example.com") };
    let bases = configured_release_api_bases();
    assert_eq!(
        bases,
        vec![
            "http://127.0.0.1:8080".to_string(),
            "http://mirror.example.com".to_string()
        ]
    );
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis test_configured_release_api_bases 2>&1 | tail -5`
Expected: FAIL — `cannot find function configured_release_api_bases`

- [ ] **Step 3: 最小实现**（`upgrade.rs` 常量区添加；`impl UpgradeManager` 内改 `new()`）

```rust
fn configured_release_api_bases() -> Vec<String> {
    if let Ok(value) = env::var("AEGIS_RELEASE_API_BASES") {
        let bases: Vec<String> = value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !bases.is_empty() {
            return bases;
        }
    }
    vec![RELEASE_API_BASE.to_string()]
}
```

```rust
    pub fn new() -> Result<Self> {
        Self::new_with_client(Self::build_client()?)
    }

    fn build_client() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .build()
            .context("构建 HTTP 客户端失败")
    }

    pub fn new_with_client(client: reqwest::Client) -> Result<Self> {
        let repositories = configured_release_repositories();
        let bases = configured_release_api_bases();
        let asset_name =
            env::var("AEGIS_RELEASE_ASSET").unwrap_or_else(|_| DEFAULT_ASSET_NAME.to_string());
        let token = env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty());
        Ok(Self {
            client,
            repositories,
            bases,
            asset_name,
            token,
        })
    }
```

`fetch_latest_release` 内 `let bases = vec![RELEASE_API_BASE.to_string()];` 改为 `let bases = &self.bases;`（`fetch_latest_release_from_repo(&self, repository, bases, api_path)` 已接受 `bases: &[String]`，直接传）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis test_configured_release_api_bases 2>&1 | tail -5`
Expected: PASS（2 个测试）

- [ ] **Step 5: 质量门 + 提交**

```bash
cd rust/aegis && cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/Cargo.toml rust/aegis/src/core/system/upgrade.rs
git commit -m "feat(upgrade): 支持 AEGIS_RELEASE_API_BASES 与 client 注入"
```

---

### Task 2: minisign 强制（消除静默降级）

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: `rust/aegis/tests/upgrade_integration.rs`（新建）

**Interfaces:**
- Consumes: `fetch_latest_release_from_repo`（Task 1）、`find_minisig_asset`（`release_api.rs`）、`MockBotAdapter`（`aegis::common::MockBotAdapter`，mockall 已生成）
- Produces: 语义变更 — minisig asset 存在但下载失败 → `run()` 返回 Err；asset 缺失 → 正常继续

- [ ] **Step 1: 写失败测试**（新建 `rust/aegis/tests/upgrade_integration.rs`）

```rust
use aegis::common::{BotAdapter, MessageContent, MessageId, MockBotAdapter, TargetId};
use aegis::core::system::upgrade::UpgradeManager;
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_release(server: &MockServer, with_minisig: bool) {
    let asset_json = |name: &str, url: &str| {
        format!(
            r#"{{"name":"{name}","browser_download_url":"{url}","size":5,"digest":"sha256:2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"}}"#
        )
    };
    let binary_url = format!("{}/download/aegis", server.uri());
    let mut assets = vec![asset_json("aegis", &binary_url)];
    if with_minisig {
        let sig_url = format!("{}/download/aegis.minisig", server.uri());
        assets.push(asset_json("aegis.minisig", &sig_url));
        Mock::given(method("GET"))
            .and(path("/download/aegis.minisig"))
            .respond_with(ResponseTemplate::new(500))
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/repos/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"tag_name":"v9.9.9","body":"release","assets":[{}]}}"#,
            assets.join(",")
        )))
        .mount(server)
        .await;
}

async fn run_upgrade(server: &MockServer) -> anyhow::Result<()> {
    let mut adapter = MockBotAdapter::new();
    adapter.expect_platform().returning(|| aegis::common::Platform::Telegram);
    adapter
        .expect_send_message()
        .returning(|_, _| Ok(MessageId("1".to_string())));
    adapter.expect_edit_message().returning(|_, _, _| Ok(()));
    adapter
        .expect_answer_callback()
        .returning(|_, _, _| Ok(()));

    let manager = UpgradeManager::new_with_client(
        reqwest::Client::builder()
            .build()
            .expect("client build"),
    )?;
    manager.run(&adapter, &TargetId("1".to_string())).await
}

#[tokio::test]
#[serial]
async fn minisig_present_but_download_fails_aborts() {
    let server = MockServer::start().await;
    unsafe {
        std::env::set_var(
            "AEGIS_RELEASE_API_BASES",
            server.uri().as_str(),
        );
    }
    mock_release(&server, true).await;
    let err = run_upgrade(&server).await.unwrap_err();
    assert!(
        err.to_string().contains("Minisign"),
        "期望 Minisign 下载失败中止，实际: {err}"
    );
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}

#[tokio::test]
#[serial]
async fn minisig_asset_missing_skips_verification() {
    let server = MockServer::start().await;
    unsafe {
        std::env::set_var(
            "AEGIS_RELEASE_API_BASES",
            server.uri().as_str(),
        );
    }
    mock_release(&server, false).await;
    let result = run_upgrade(&server).await;
    // 无 minisig asset -> 跳过验证；但二进制下载 URL 未 mock 会失败——此处先验证不会因 minisig 报错
    if let Err(e) = result {
        assert!(
            !e.to_string().contains("Minisign"),
            "minisig asset 缺失不应触发 Minisign 错误: {e}"
        );
    }
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cd rust/aegis && cargo test --test upgrade_integration 2>&1 | tail -20`
Expected: FAIL — 当前实现吞掉 minisig 下载错误，`minisig_present_but_download_fails_aborts` 不满足 abort 断言

- [ ] **Step 3: 最小实现**（`fetch_latest_release_from_repo` 内）

```rust
        // 现状
        let minisig = self
            .download_minisig(&release.assets, &asset.name)
            .await
            .ok()
            .flatten();
        // 目标
        let minisig = self.download_minisig(&release.assets, &asset.name).await?;
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test upgrade_integration 2>&1 | tail -20`
Expected: PASS — `minisig_present_but_download_fails_aborts` 现在返回 Minisign 错误

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/tests/upgrade_integration.rs
git commit -m "fix(upgrade): minisign 下载失败不再静默降级，改为中止"
```

---

### Task 3: 下载超时（60s → 600s + 10s connect）

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: 常量 `CONNECT_TIMEOUT_SECS = 10`、`REQUEST_TIMEOUT_SECS = 600`；`build_client` 使用它们

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn test_timeout_constants() {
    assert_eq!(CONNECT_TIMEOUT_SECS, 10);
    assert_eq!(REQUEST_TIMEOUT_SECS, 600);
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis test_timeout_constants 2>&1 | tail -5`
Expected: FAIL — 常量不存在

- [ ] **Step 3: 最小实现**（常量区 + `build_client` 内）

```rust
const CONNECT_TIMEOUT_SECS: u64 = 10;
const REQUEST_TIMEOUT_SECS: u64 = 600;
```

```rust
    fn build_client() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .context("构建 HTTP 客户端失败")
    }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis test_timeout_constants 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/upgrade.rs
git commit -m "fix(upgrade): 下载超时 60s 提升至 600s，connect 独立 10s"
```

---

### Task 4: 唯一临时文件（消除固定文件名竞态）

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: `fn unique_update_path(exe_dir: &Path) -> PathBuf` — `.aegis-update-<pid>-<nanos>.tmp`，同目录保证 rename 原子；`download_with_progress` 改用

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn test_unique_update_path_is_unique_and_same_dir() {
    let dir = Path::new("/etc/wwps/aegis");
    let a = unique_update_path(dir);
    let b = unique_update_path(dir);
    assert_eq!(a.parent(), Some(dir));
    assert_ne!(a, b);
    assert!(a
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with(".aegis-update-"));
    assert_eq!(a.extension().unwrap().to_str().unwrap(), "tmp");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis test_unique_update_path 2>&1 | tail -5`
Expected: FAIL — 函数不存在

- [ ] **Step 3: 最小实现**

```rust
fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn unique_update_path(exe_dir: &Path) -> PathBuf {
    exe_dir.join(format!(
        ".aegis-update-{}-{:x}.tmp",
        std::process::id(),
        now_nanos()
    ))
}
```

`download_with_progress` 内替换：

```rust
        let current_exe = std::env::current_exe().context("无法获取当前可执行文件路径")?;
        let exe_dir = current_exe
            .parent()
            .context("无法获取可执行文件目录")?
            .to_path_buf();
        let update_path = unique_update_path(&exe_dir);
        let mut file = File::create(&update_path)
            .await
            .context("创建临时更新文件失败")?;
```

删除 `let update_path = current_exe.with_extension("update");` 原行。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis test_unique_update_path 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/upgrade.rs
git commit -m "fix(upgrade): 下载临时文件改为唯一命名，消除并发竞态"
```

---

### Task 5: 升级前版本比较

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: `fn is_current_version(tag: &str) -> bool` — 去 `v` 前缀后与 `env!("CARGO_PKG_VERSION")` 比较；`run()` 在 fetch 成功后接入

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn test_is_current_version() {
    assert!(is_current_version("v1.3.2"));
    assert!(is_current_version("1.3.2"));
    assert!(!is_current_version("v9.9.9"));
    assert!(!is_current_version(""));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis test_is_current_version 2>&1 | tail -5`
Expected: FAIL — 函数不存在

- [ ] **Step 3: 最小实现**（常量区附近加函数；`run()` 接入）

```rust
fn is_current_version(tag: &str) -> bool {
    tag.trim().trim_start_matches('v') == env!("CARGO_PKG_VERSION")
}
```

`run()` 中 fetch 成功后、发送 summary 前插入：

```rust
        if is_current_version(&artifact.tag_name) {
            let _ = adapter
                .edit_message(
                    target,
                    &progress_msg_id,
                    MessageContent {
                        text: t!("upgrade.bot_already_latest").to_string(),
                        markup: None,
                    },
                )
                .await;
            return Ok(());
        }
```

在 `src/resources/i18n/zh.yml`、`en.yml`、`ja.yml` 添加键（值：zh `"已是最新版本，无需更新"` / en `"Already up to date"` / ja `"すでに最新版です"`）：

```yaml
upgrade:
  bot_already_latest: "已是最新版本，无需更新"
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis test_is_current_version 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/src/resources/i18n/
git commit -m "feat(upgrade): 升级前版本比较，已最新则跳过"
```

---

### Task 6: 升级并发锁（O_EXCL + stale 检测）

**Files:**
- Modify: `rust/aegis/src/core/paths.rs`（`maintenance` 模块加常量）
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces:
  - `pub const UPGRADE_LOCK_FILE: &str = "/etc/wwps/aegis/upgrade.lock";`（`paths.rs::maintenance`）
  - `fn acquire_upgrade_lock(lock_path: &Path) -> Result<UpgradeLock>` — `create_new` 独占创建并写 pid；已存在时读 pid 检查 `/proc/<pid>`，stale 则删除重试一次，否则报错
  - `pub struct UpgradeLock { path: PathBuf }` — `Drop` 时删除锁文件；`run()` 开头获取并 `let _lock = ...` 持有至结束

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn test_acquire_upgrade_lock_success_and_conflict() {
    let dir = std::env::temp_dir().join(format!("upgrade-lock-test-{}", std::process::id()));
    let lock_path = dir.join("upgrade.lock");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let lock = acquire_upgrade_lock(&lock_path).await.unwrap();
    // 第二次获取应失败（非 stale：pid 是当前进程）
    let err = acquire_upgrade_lock(&lock_path).await.unwrap_err();
    assert!(err.to_string().contains("升级"), "期望并发冲突错误: {err}");
    drop(lock);
    // 释放后应可重新获取
    assert!(acquire_upgrade_lock(&lock_path).await.is_ok());
    tokio::fs::remove_dir_all(&dir).await.unwrap();
}

#[tokio::test]
async fn test_acquire_upgrade_lock_stale_takeover() {
    let dir = std::env::temp_dir().join(format!("upgrade-lock-stale-{}", std::process::id()));
    let lock_path = dir.join("upgrade.lock");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    // 写入一个不存在的 pid
    tokio::fs::write(&lock_path, "99999999").await.unwrap();
    // stale -> 应接管成功
    assert!(acquire_upgrade_lock(&lock_path).await.is_ok());
    tokio::fs::remove_dir_all(&dir).await.unwrap();
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis test_acquire_upgrade_lock 2>&1 | tail -5`
Expected: FAIL — 函数/结构不存在

- [ ] **Step 3: 最小实现**（`upgrade.rs`；`paths.rs` 加常量）

```rust
use tokio::io::AsyncWriteExt;

pub struct UpgradeLock {
    path: PathBuf,
}

impl Drop for UpgradeLock {
    fn drop(&mut self) {
        // Drop 无法 async，此处用阻塞式删除是合理例外（仅删一个小文件）
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn lock_is_stale(lock_path: &Path) -> bool {
    let pid = tokio::fs::read_to_string(lock_path)
        .await
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    match pid {
        Some(pid) => !Path::new(&format!("/proc/{pid}")).exists(),
        None => true,
    }
}

async fn acquire_upgrade_lock(lock_path: &Path) -> Result<UpgradeLock> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).await.context("创建锁目录失败")?;
    }
    let mut attempt = 0;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
            .await
        {
            Ok(mut file) => {
                let _ = file
                    .write_all(format!("{}\n", std::process::id()).as_bytes())
                    .await;
                return Ok(UpgradeLock {
                    path: lock_path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_is_stale(lock_path).await && attempt == 0 {
                    fs::remove_file(lock_path).await.ok();
                    attempt += 1;
                    continue;
                }
                anyhow::bail!("另一个升级正在进行中");
            }
            Err(e) => return Err(e.into()),
        }
    }
}
```

`run()` 开头（发消息前）插入：`let _lock = acquire_upgrade_lock(&self.lock_path).await?;`（`lock_path` 字段在 Task 8 加入，此处先加字段声明：`lock_path: PathBuf` 并在 `new_with_client` 初始化 `PathBuf::from(UPGRADE_LOCK_FILE)`，`use crate::core::paths::maintenance::UPGRADE_LOCK_FILE;`）

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis test_acquire_upgrade_lock 2>&1 | tail -5`
Expected: PASS（2 个测试）

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/paths.rs rust/aegis/src/core/system/upgrade.rs
git commit -m "feat(upgrade): 并发升级锁 O_EXCL + stale pid 接管"
```

---

### Task 7: DeployStrategy trait + ProductionDeploy（冒烟/备份/原子替换/双重启策略）

**Files:**
- Create: `rust/aegis/src/core/system/deploy.rs`
- Modify: `rust/aegis/src/core/system/mod.rs`（`pub mod deploy;`）
- Modify: `rust/aegis/src/core/system/upgrade.rs`（字段 + `with_deploy`）
- Test: `rust/aegis/src/core/system/deploy.rs`（`mod tests`）

**Interfaces:**
- Produces:
  - `#[async_trait] pub trait DeployStrategy: Send + Sync { async fn deploy(&self, update_path: &Path, current_exe: &Path) -> Result<()>; fn needs_exit(&self) -> bool { true } }`
  - `pub enum RestartStrategy { Systemd, Reexec }`
  - `pub struct ProductionDeploy { strategy: RestartStrategy }` + `new()` / `with_strategy()`
  - `fn is_systemd_managed() -> bool`（`INVOCATION_ID` env 或 `/run/systemd/system` 存在）
  - `fn detect_restart_strategy() -> RestartStrategy`（env `AEGIS_UPGRADE_STRATEGY`: `systemd`/`reexec`/默认 `auto`）
  - `async fn smoke_test_binary(path: &Path) -> Result<()>` — 执行 `path --version`，要求 exit 0 且 stdout 含 `aegis`
  - `UpgradeManager::with_deploy(deploy: Arc<dyn DeployStrategy>) -> Self`；`deploy` 字段默认 `Arc::new(ProductionDeploy::new())`

- [ ] **Step 1: 写失败测试**（新建 `deploy.rs`，测试代码一并写入）

```rust
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::fs;

pub enum RestartStrategy {
    Systemd,
    Reexec,
}

#[async_trait]
pub trait DeployStrategy: Send + Sync {
    async fn deploy(&self, update_path: &Path, current_exe: &Path) -> Result<()>;
    fn needs_exit(&self) -> bool {
        true
    }
}

pub fn is_systemd_managed() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
        || std::env::var_os("JOURNAL_STREAM").is_some()
        || Path::new("/run/systemd/system").exists()
}

pub fn detect_restart_strategy() -> RestartStrategy {
    match std::env::var("AEGIS_UPGRADE_STRATEGY").as_deref() {
        Ok("systemd") => return RestartStrategy::Systemd,
        Ok("reexec") => return RestartStrategy::Reexec,
        _ => {}
    }
    if is_systemd_managed() {
        RestartStrategy::Systemd
    } else {
        RestartStrategy::Reexec
    }
}

pub struct ProductionDeploy {
    strategy: RestartStrategy,
}

impl ProductionDeploy {
    pub fn new() -> Self {
        Self {
            strategy: detect_restart_strategy(),
        }
    }

    pub fn with_strategy(strategy: RestartStrategy) -> Self {
        Self { strategy }
    }
}

#[async_trait]
impl DeployStrategy for ProductionDeploy {
    async fn deploy(&self, update_path: &Path, current_exe: &Path) -> Result<()> {
        // 1. 冒烟测试：新二进制可执行且 --version 正常
        smoke_test_binary(update_path).await?;
        // 2. 备份旧二进制
        let backup = current_exe.with_extension("bak");
        fs::copy(current_exe, &backup)
            .await
            .context("备份旧二进制失败")?;
        // 3. 原子替换
        fs::rename(update_path, current_exe)
            .await
            .context("替换二进制失败")?;
        match self.strategy {
            RestartStrategy::Systemd => { /* 调用方 exit(0)，systemd Restart=always 拉起 */ }
            RestartStrategy::Reexec => spawn_replacement(current_exe)?,
        }
        Ok(())
    }
}

#[cfg(unix)]
fn spawn_replacement(current_exe: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::Command::new(current_exe)
        .args(&args)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("启动新版本进程失败")?;
    Ok(())
}

#[cfg(not(unix))]
fn spawn_replacement(_current_exe: &Path) -> Result<()> {
    anyhow::bail!("当前平台不支持 reexec 重启策略")
}

async fn smoke_test_binary(path: &Path) -> Result<()> {
    let output = tokio::process::Command::new(path)
        .arg("--version")
        .output()
        .await
        .context("冒烟测试：无法执行新二进制")?;
    if !output.status.success() {
        anyhow::bail!(
            "冒烟测试失败：退出码 {:?}",
            output.status.code()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("aegis") {
        anyhow::bail!(
            "冒烟测试失败：--version 输出异常: {}",
            stdout.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smoke_test_accepts_valid_binary() {
        let dir = std::env::temp_dir().join(format!("smoke-ok-{}", std::process::id()));
        fs::create_dir_all(&dir).await.unwrap();
        let script = dir.join("fake-aegis");
        fs::write(&script, "#!/bin/sh\necho 'aegis 1.3.2'\n").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).await.unwrap();
        }
        assert!(smoke_test_binary(&script).await.is_ok());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn smoke_test_rejects_nonzero_exit() {
        let dir = std::env::temp_dir().join(format!("smoke-fail-{}", std::process::id()));
        fs::create_dir_all(&dir).await.unwrap();
        let script = dir.join("fake-aegis");
        fs::write(&script, "#!/bin/sh\nexit 1\n").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).await.unwrap();
        }
        let err = smoke_test_binary(&script).await.unwrap_err();
        assert!(err.to_string().contains("冒烟测试失败"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn smoke_test_rejects_missing_aegis_output() {
        let dir = std::env::temp_dir().join(format!("smoke-out-{}", std::process::id()));
        fs::create_dir_all(&dir).await.unwrap();
        let script = dir.join("fake-aegis");
        fs::write(&script, "#!/bin/sh\necho 'not-aegis'\n").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).await.unwrap();
        }
        let err = smoke_test_binary(&script).await.unwrap_err();
        assert!(err.to_string().contains("输出异常"));
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
```

（`use anyhow::Context;` 需加入 import：文件顶部 `use anyhow::{Context, Result, anyhow};`）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p aegis deploy:: 2>&1 | tail -10`
Expected: FAIL — `deploy.rs` 不存在（`mod deploy` 未声明）

- [ ] **Step 3: 实现**（上面 Step 1 的代码即完整实现；另在 `mod.rs` 加 `pub mod deploy;`；`upgrade.rs` 加字段与 `with_deploy`）

```rust
// upgrade.rs
use crate::core::system::deploy::{DeployStrategy, ProductionDeploy};

    // struct 字段新增：
    deploy: Arc<dyn DeployStrategy>,

    // new_with_client 末尾：
    Ok(Self {
        client,
        repositories,
        bases,
        asset_name,
        token,
        deploy: Arc::new(ProductionDeploy::new()),
    })

    pub fn with_deploy(mut self, deploy: Arc<dyn DeployStrategy>) -> Self {
        self.deploy = deploy;
        self
    }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p aegis deploy:: 2>&1 | tail -10`
Expected: PASS（3 个测试）

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/deploy.rs rust/aegis/src/core/system/mod.rs rust/aegis/src/core/system/upgrade.rs
git commit -m "feat(upgrade): DeployStrategy trait + ProductionDeploy 冒烟/备份/原子替换/双重启策略"
```

---

### Task 8: finalize_install 重构 + flag/lock 路径可配置 + run() 全链路集成测试

**Files:**
- Modify: `rust/aegis/src/core/system/upgrade.rs`
- Modify: `rust/aegis/tests/upgrade_integration.rs`
- Test: `rust/aegis/tests/upgrade_integration.rs`

**Interfaces:**
- Consumes: `DeployStrategy`（Task 7）、`UPGRADE_LOCK_FILE`（Task 6）、`MockDeployStrategy`（本任务测试定义）
- Produces: `UpgradeManager` 新增 `flag_path: PathBuf`（默认 `UPGRADE_FLAG_FILE`）与 `lock_path: PathBuf`（默认 `UPGRADE_LOCK_FILE`）+ `with_paths(flag_path, lock_path)`；`finalize_install` 使用 `self.deploy` 并 `needs_exit()` 时 exit(0)；`write_upgrade_flag` 用 `self.flag_path`；`run()` 尾部重写

- [ ] **Step 1: 写失败测试**（`upgrade_integration.rs` 添加 happy path 全链路）

```rust
use aegis::core::system::deploy::DeployStrategy;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct MockDeploy {
    calls: AtomicUsize,
    last_update_path: std::sync::Mutex<Option<PathBuf>>,
}

#[async_trait]
impl DeployStrategy for MockDeploy {
    async fn deploy(&self, update_path: &Path, _current_exe: &Path) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_update_path.lock().unwrap() = Some(update_path.to_path_buf());
        Ok(())
    }
    fn needs_exit(&self) -> bool {
        false
    }
}

#[tokio::test]
#[serial]
async fn happy_path_downloads_verifies_and_deploys() {
    let server = MockServer::start().await;
    unsafe { std::env::set_var("AEGIS_RELEASE_API_BASES", server.uri().as_str()) };

    // 二进制内容 -> sha256: 2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b = sha256("hello")
    let binary = b"hello";
    Mock::given(method("GET"))
        .and(path("/download/aegis"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(binary.to_vec()))
        .mount(&server)
        .await;
    mock_release(&server, false).await;

    let tmp = std::env::temp_dir().join(format!("upgrade-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut adapter = MockBotAdapter::new();
    adapter.expect_platform().returning(|| aegis::common::Platform::Telegram);
    adapter.expect_send_message().returning(|_, _| Ok(MessageId("1".to_string())));
    adapter.expect_edit_message().returning(|_, _, _| Ok(()));
    adapter.expect_answer_callback().returning(|_, _, _| Ok(()));

    let deploy = Arc::new(MockDeploy::default());
    let manager = UpgradeManager::new_with_client(
        reqwest::Client::builder().build().expect("client build"),
    )
    .expect("manager")
    .with_paths(tmp.join("upgrade.flag"), tmp.join("upgrade.lock"))
    .with_deploy(deploy.clone());

    manager.run(&adapter, &TargetId("1".to_string())).await.unwrap();

    assert_eq!(deploy.calls.load(Ordering::SeqCst), 1);
    let deployed = deploy.last_update_path.lock().unwrap().clone().unwrap();
    assert!(deployed.file_name().unwrap().to_str().unwrap().starts_with(".aegis-update-"));
    assert!(tmp.join("upgrade.flag").exists(), "升级成功 flag 应写入");
    // 清理
    std::fs::remove_dir_all(&tmp).unwrap();
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test upgrade_integration happy_path 2>&1 | tail -20`
Expected: FAIL — `with_paths`/`with_deploy`/`DeployStrategy` 导入或 `run` 行为不满足（当前 `finalize_install` 会 `exit(0)`）

- [ ] **Step 3: 实现**（`upgrade.rs`）

```rust
    // 字段新增
    flag_path: PathBuf,
    lock_path: PathBuf,

    // new_with_client 初始化
        flag_path: PathBuf::from(UPGRADE_FLAG_FILE),
        lock_path: PathBuf::from(UPGRADE_LOCK_FILE),

    pub fn with_paths(mut self, flag_path: PathBuf, lock_path: PathBuf) -> Self {
        self.flag_path = flag_path;
        self.lock_path = lock_path;
        self
    }
```

`finalize_install` 尾部替换为：

```rust
        let current_exe = std::env::current_exe().context("无法获取当前可执行文件路径")?;
        self.deploy.deploy(update_path, &current_exe).await?;

        fs::remove_file(update_path).await.ok();

        self.write_upgrade_flag(&artifact.tag_name).await?;

        adapter
            .send_message(
                target,
                MessageContent {
                    text: t!("upgrade.bot_updated", "0" => artifact.tag_name.as_str()).to_string(),
                    markup: None,
                },
            )
            .await?;

        if self.deploy.needs_exit() {
            sleep(Duration::from_secs(2)).await;
            std::process::exit(0);
        }
        Ok(())
```

`run()` 开头插入锁：`let _lock = acquire_upgrade_lock(&self.lock_path).await?;`

`write_upgrade_flag` 改用字段：

```rust
    pub async fn write_upgrade_flag(&self, version: &str) -> Result<()> {
        if let Some(parent) = self.flag_path.parent() {
            fs::create_dir_all(parent)
                .await
                .context("创建升级标记目录失败")?;
        }
        fs::write(&self.flag_path, version)
            .await
            .context("写入升级标记文件失败")
    }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test upgrade_integration 2>&1 | tail -20`
Expected: PASS — 3 个测试（minisig abort / minisig skip / happy path）

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/src/core/system/upgrade.rs rust/aegis/tests/upgrade_integration.rs
git commit -m "refactor(upgrade): finalize 接入 DeployStrategy，flag/lock 路径可配置"
```

---

### Task 9: sha256 不匹配清理 + 版本相同跳过 集成测试（回归加固）

**Files:**
- Modify: `rust/aegis/tests/upgrade_integration.rs`

**Interfaces:**
- Consumes: Task 8 的完整 `run()` 链路
- Produces: 两条回归断言 — sha256 不匹配时临时文件被清理；tag 等于当前版本时跳过下载（deploy 不被调用）

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
#[serial]
async fn sha256_mismatch_aborts_and_cleans_temp() {
    let server = MockServer::start().await;
    unsafe { std::env::set_var("AEGIS_RELEASE_API_BASES", server.uri().as_str()) };
    // 二进制内容与 digest 不匹配（digest 是 sha256("hello")，内容为 "world"）
    let binary = b"world";
    Mock::given(method("GET"))
        .and(path("/download/aegis"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(binary.to_vec()))
        .mount(&server)
        .await;
    mock_release(&server, false).await;

    let tmp = std::env::temp_dir().join(format!("upgrade-sha-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut adapter = MockBotAdapter::new();
    adapter.expect_platform().returning(|| aegis::common::Platform::Telegram);
    adapter.expect_send_message().returning(|_, _| Ok(MessageId("1".to_string())));
    adapter.expect_edit_message().returning(|_, _, _| Ok(()));
    adapter.expect_answer_callback().returning(|_, _, _| Ok(()));

    let manager = UpgradeManager::new_with_client(
        reqwest::Client::builder().build().expect("client build"),
    )
    .expect("manager")
    .with_paths(tmp.join("upgrade.flag"), tmp.join("upgrade.lock"));

    let err = manager.run(&adapter, &TargetId("1".to_string())).await.unwrap_err();
    assert!(err.to_string().contains("SHA256"), "期望 sha256 不匹配: {err}");

    // 临时文件应被清理：扫描 exe 目录下无 .aegis-update-*.tmp 新文件
    let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let leftovers: Vec<_> = std::fs::read_dir(&exe_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(".aegis-update-"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件应被清理: {leftovers:?}");

    std::fs::remove_dir_all(&tmp).unwrap();
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}

#[tokio::test]
#[serial]
async fn already_latest_skips_download_and_deploy() {
    let server = MockServer::start().await;
    unsafe { std::env::set_var("AEGIS_RELEASE_API_BASES", server.uri().as_str()) };
    // tag 与当前版本 env!("CARGO_PKG_VERSION") 一致（测试用 "v1.3.2" 前缀推导）
    let current = env!("CARGO_PKG_VERSION");
    let tag = format!("v{current}");
    let body = format!(
        r#"{{"tag_name":"{tag}","body":"release","assets":[{{"name":"aegis","browser_download_url":"{}/download/aegis","size":5,"digest":"sha256:2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"}}]}}"#,
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/repos/youugiuhiuh/Wuthering_Waves_Private_Server/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let tmp = std::env::temp_dir().join(format!("upgrade-skip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let mut adapter = MockBotAdapter::new();
    adapter.expect_platform().returning(|| aegis::common::Platform::Telegram);
    adapter.expect_send_message().returning(|_, _| Ok(MessageId("1".to_string())));
    adapter.expect_edit_message().returning(|_, _, _| Ok(()));
    adapter.expect_answer_callback().returning(|_, _, _| Ok(()));

    let deploy = Arc::new(MockDeploy::default());
    let manager = UpgradeManager::new_with_client(
        reqwest::Client::builder().build().expect("client build"),
    )
    .expect("manager")
    .with_paths(tmp.join("upgrade.flag"), tmp.join("upgrade.lock"))
    .with_deploy(deploy.clone());

    manager.run(&adapter, &TargetId("1".to_string())).await.unwrap();
    assert_eq!(deploy.calls.load(Ordering::SeqCst), 0, "已最新不应触发部署");
    assert!(!tmp.join("upgrade.flag").exists(), "不应写升级 flag");

    std::fs::remove_dir_all(&tmp).unwrap();
    unsafe { std::env::remove_var("AEGIS_RELEASE_API_BASES") };
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test upgrade_integration 2>&1 | tail -20`
Expected: 视当前实现 FAIL（临时文件路径/清理或跳过逻辑尚未全部满足）

- [ ] **Step 3: 实现**（如测试因实现缺失失败则补实现；通常 Task 4/5/8 已覆盖，此任务主要是补测试固化回归）

核查点：`download_with_progress` 中 sha256 不匹配路径必须 `fs::remove_file(&update_path).await.ok();`（已存在，确认保留）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test upgrade_integration 2>&1 | tail -20`
Expected: PASS（5 个集成测试）

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc
git add rust/aegis/tests/upgrade_integration.rs
git commit -m "test(upgrade): sha256 不匹配清理 + 版本相同跳过 回归测试"
```

---

### Task 10: 全量回归 + 文档收尾

**Files:**
- Modify: `README.md`（如含升级说明，补充 `AEGIS_RELEASE_API_BASES`/`AEGIS_UPGRADE_STRATEGY` 两个 env）
- Test: 全量

- [ ] **Step 1: 全量质量门**

Run: `cd rust/aegis && cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`
Expected: 全部通过，0 warnings

- [ ] **Step 2: README 补充 env 说明**（如 `README.md` 有配置章节；无则跳过）

```markdown
- `AEGIS_RELEASE_API_BASES`: 逗号分隔的 Release API base（默认 `https://api.github.com`），可指向自建镜像
- `AEGIS_UPGRADE_STRATEGY`: 升级重启策略 `systemd` / `reexec` / `auto`（默认 auto 自动检测）
```

- [ ] **Step 3: 提交**

```bash
git add README.md
git commit -m "docs: 补充自更新 env 配置说明"
```

---

## Self-Review

**1. Spec 覆盖检查：**
- minisign 强制 → Task 2 ✓
- 超时 600s/10s → Task 3 ✓
- 唯一临时文件 → Task 4 ✓
- 版本比较 → Task 5 ✓
- 并发锁 + stale → Task 6 ✓
- DeployStrategy + ProductionDeploy（冒烟/备份/rename/systemd/reexec）→ Task 7 ✓
- finalize 重构 + flag/lock 路径可配置 + 集成测试 → Task 8 ✓
- 回归测试（sha256 清理 / 版本相同跳过）→ Task 9 ✓
- YAGNI 排除项（断点续传/看门狗/签名夹具）→ 无对应任务，符合 spec ✓

**2. 占位符扫描：** 无 TBD/TODO；每个代码步骤含完整代码。

**3. 类型一致性：**
- `DeployStrategy::deploy(&self, &Path, &Path)` 在 Task 7 定义、Task 8 `finalize_install` 与 `MockDeploy` 使用 —— 签名一致 ✓
- `needs_exit()` 默认 true，`MockDeploy` 覆写 false —— Task 7/8 一致 ✓
- `acquire_upgrade_lock(&Path)` 在 Task 6 定义、Task 8 `run()` 调用 —— 一致 ✓
- `unique_update_path(&Path) -> PathBuf` Task 4 定义、Task 8 集成测试断言 `.aegis-update-` 前缀 —— 一致 ✓
- `with_paths`/`with_deploy` 签名 Task 7/8 定义与测试使用一致 ✓
- i18n 键 `upgrade.bot_already_latest` 在 Task 5 添加，Task 9 跳过路径依赖该文案但不断言文案内容（仅断言 flag/deploy 行为）—— 无硬依赖 ✓

**已知注意点（执行时处理）：**
- Task 8 Step 1 的 happy path 依赖「无 minisig asset → 跳过签名」+「digest 与二进制匹配」两个前提，若 wiremock `browser_download_url` 未命中则集成测试报错，属预期 TDD 流程。
- `MockBotAdapter::expect_answer_callback` 在 `run()` 未直接调用（`handle_upgrade` 才调用），保留 expectation 无害；若 mockall 报未使用可删除该行。
- 测试中 `env::set_var` 均为 `unsafe` 块（edition 2024）+ `#[serial]`（serial_test），避免污染并行测试。

## Execution Handoff

计划已保存至 `docs/superpowers/plans/2026-08-31-bot-self-update-hardening.md`。两种执行方式：

1. **Subagent-Driven（推荐）** — 每个任务派发独立 subagent，任务间审查，快速迭代
2. **Inline Execution** — 本会话内用 executing-plans 批量执行，带检查点审查
