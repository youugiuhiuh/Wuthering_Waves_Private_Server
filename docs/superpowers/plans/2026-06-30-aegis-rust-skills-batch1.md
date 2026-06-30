# Aegis Rust Skills Batch 1 改进实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 按 RUST_SKILLS_REVIEW.md 的 Batch 1 范围，修复违反规则并优化代码质量。

**Architecture:** 5 个独立模块并行改进：Clippy Lint、unwrap 清理、unsafe 文档化、API 增强、CI 配置。每个模块影响独立文件集，可独立验证。

**Tech Stack:** Rust, cargo

**Worktree:** `rust/aegis/` under `.worktrees/rust-skills-batch1/`

---

### Task A: Clippy Lint 基础设施增强

**Files:**
- Modify: `rust/aegis/Cargo.toml`
- Modify: `rust/aegis/src/lib.rs`
- Modify: `rust/aegis/src/main.rs`

- [ ] **Step A1: 在 Cargo.toml 添加 `[lints]` 段**

在 `Cargo.toml` 末尾添加：

```toml
[lints.rust]
# undocumented_unsafe_blocks: 强制 unsafe 块提供 SAFETY 注释
unsafe_code = "deny"

[lints.clippy]
correctness = "deny"
suspicious = "warn"
style = "warn"
complexity = "warn"
perf = "warn"
# undocumented_unsafe_blocks: 强制每个 unsafe 块有 // SAFETY: 注释
undocumented_unsafe_blocks = "deny"
```

注意：这不会破坏构建，因为当前代码中 `unsafe` 块都没有 `// SAFETY:` 注释，`undocumented_unsafe_blocks = "deny"` 会在 Task C 完成文档化后通过。

- [ ] **Step A2: 更新 lib.rs 添加 lint 属性**

将 `src/lib.rs` 从：
```rust
rust_i18n::i18n!("src/resources/i18n");

pub mod adapters;
pub mod core;
```
改为：
```rust
#![deny(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
// 选择性压制不必要的 pedantic lint
#![allow(
    clippy::module_name_repetitions,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::too_many_arguments,
)]

rust_i18n::i18n!("src/resources/i18n");

pub mod adapters;
pub mod core;
```

- [ ] **Step A3: 移除 main.rs 中的 allow 属性**

将 `src/main.rs` 开头的：
```rust
#![recursion_limit = "256"]
#![allow(clippy::vec_init_then_push)]
```
改为：
```rust
#![recursion_limit = "256"]
```
因为 `clippy::vec_init_then_push` 已经被全局 `style = "warn"` 覆盖。

- [ ] **Step A4: 验证 lint 变更**

```bash
cargo check 2>&1
cargo clippy -- -D warnings 2>&1
```
Expected: 编译成功。Clippy 无 `-D warnings` 错误（可能有一些警告被 `style = "warn"` 触发，但不阻止构建）。

- [ ] **Step A5: 提交**

```bash
git add rust/aegis/Cargo.toml rust/aegis/src/lib.rs rust/aegis/src/main.rs
git commit -m "ci(clippy): add lint infrastructure with cargo lints section"
```

---

### Task B: 清理生产代码中 unwrap

**Files:**
- Modify: `rust/aegis/src/core/security/firewalld.rs`
- Modify: `rust/aegis/src/bootstrap.rs`
- Modify: `rust/aegis/src/adapters/telegram/handlers/xray/batch.rs`
- Modify: `rust/aegis/src/core/xray/kcp.rs`

- [ ] **Step B1: 修复 firewalld.rs 中 5 处 format! + try_from unwrap**

`src/core/security/firewalld.rs:94-95` — 将 `OwnedObjectPath::try_from(...).unwrap()` 改为 `?`：

```rust
            Err(_) => {
                zbus::zvariant::OwnedObjectPath::try_from(
                    "/org/fedoraproject/FirewallD1/config",
                )?
            }
```

这个方法有 2 处相同位置（`add_port` 和 `remove_port` 函数各一处）。另外 3 处是 `format!(...).unwrap()` 类似模式，搜索 `unwrap()` 在 firewalld.rs 中的位置并逐个替换为 `?`。

执行：
```bash
grep -n "unwrap()" rust/aegis/src/core/security/firewalld.rs
```
找到所有 5 处，逐行分析是否可改为 `?`。所有 zbus 调用相关的 unwrap 都应传播为 `?`。

- [ ] **Step B2: 修复 bootstrap.rs 中 5 处 Option unwrap**

`src/bootstrap.rs:249-253` — 由于已经检查过 `is_some()`，可以使用 `unwrap()` 但语义上不安全。替换为使用 `if let Some(...)` 模式：

```rust
    let matrix = match (
        input.matrix_homeserver.as_ref(),
        input.matrix_username.as_ref(),
        input.matrix_password.as_ref(),
        input.matrix_room_id.as_ref(),
        input.matrix_store_passphrase.as_ref(),
    ) {
        (Some(homeserver), Some(username), Some(password), Some(room_id), Some(store_passphrase)) => {
            Some(MatrixSetupConfig {
                homeserver: homeserver.clone(),
                username: username.clone(),
                password: password.clone(),
                room_id: room_id.clone(),
                store_passphrase: store_passphrase.clone(),
            })
        }
        _ => None,
    };
```

- [ ] **Step B3: 修复 batch.rs 中 KcpMask unwrap**

`src/adapters/telegram/handlers/xray/batch.rs:496` — `KcpMask::from_code(code).unwrap()`，由于 `code` 已经通过 `strip_prefix("u_kcp_add:").unwrap_or("ml")` 保证有效，改为 `.expect()`：

```rust
        let m = KcpMask::from_code(code)
            .expect("kcp code from validated callback data");
```

- [ ] **Step B4: 修复 batch.rs 中 last().unwrap()**

`src/adapters/telegram/handlers/xray/batch.rs:629,634` — `.last().unwrap()` 改为 `.last().unwrap_or(&vec![])` 或处理空 case。

读取上下文后确定合适的空列表处理方式。

- [ ] **Step B5: 修复 kcp.rs 中 serde_json unwrap**

`src/core/xray/kcp.rs:76` — `serde_json::to_string(...).unwrap()` 在 `build_proxy_link` 函数中，改为 `expect()`：

```rust
        let fm_str = serde_json::to_string(&finalmask_json)
            .expect("finalmask JSON should always serialize");
```

- [ ] **Step B6: 修复 kcp.rs:168-173 中生产和开发 unwrap**

```rust
        let file = std::fs::File::create(&config_path)
            .expect("tempdir should be writable");
        serde_json::to_writer_pretty(file, &full)
            .expect("config should serialize");
        // ...
            .arg(config_path.to_str()
                .expect("tempdir path must be valid UTF-8"))
```

- [ ] **Step B7: 验证**

```bash
cargo build 2>&1
cargo test 2>&1
cargo clippy -- -D warnings 2>&1
```
Expected: 编译通过，445 tests passed，clippy clean。

- [ ] **Step B8: 提交**

```bash
git add rust/aegis/src/core/security/firewalld.rs rust/aegis/src/bootstrap.rs rust/aegis/src/adapters/telegram/handlers/xray/batch.rs rust/aegis/src/core/xray/kcp.rs
git commit -m "fix: replace production unwraps with error propagation or expect"
```

---

### Task C: unsafe 块文档化

**Files:**
- Modify: `rust/aegis/src/core/security/crypto.rs`
- Modify: `rust/aegis/src/core/system/core_upgrade.rs`
- Modify: `rust/aegis/src/bootstrap.rs`

- [ ] **Step C1: 文档化 bootstrap.rs 中 unsafe 块**

`src/bootstrap.rs:290` — `libc::setrlimit`：

```rust
        // SAFETY: setrlimit 是安全的 POSIX 调用。
        // RLIMIT_CORE + 0 设置 core dump 大小限制为 0 (禁用)。
        // 此操作在同一进程内仅执行一次，无竞态。
        let setrlimit_ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) };
```

`src/bootstrap.rs:295` — `libc::prctl`：

```rust
        // SAFETY: prctl(PR_SET_DUMPABLE, 0) 禁止当前进程产生 core dump。
        // 单线程启动阶段执行，无并发。
        // 参数 0/0/0 是 PR_SET_DUMPABLE 的未使用保留参数。
        let prctl_ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
```

- [ ] **Step C2: 文档化 crypto.rs 中 unsafe 块**

`src/core/security/crypto.rs:91` — `libc::mlock`：

```rust
        // SAFETY: data 是 &[u8] 引用，保证指针有效且长度正确。
        // mlock 将内存页锁定在 RAM 中防止被换出到磁盘。
        // 成功时数据完全锁定，失败返回非零不修改内存。
        // 此操作不会创建新指针或改变内存状态，仅锁定已有的有效内存。
        let ret = unsafe { ... };
```

`src/core/security/crypto.rs:113` — `mlock` 直接调用：

```rust
    // SAFETY: data.as_ptr() 指向有效的 Vec<u8> 缓冲区。
    // mlock 的 addr+length 必须对齐到页边界，Vec 的分配器保证至少页对齐。
    // 调用一次 lock 后再无其他竞态。
    let ret = unsafe { mlock(data.as_ptr() as *const libc::c_void, data.len()) };
```

`src/core/security/crypto.rs:124` — `munlock`：

```rust
    // SAFETY: data.as_ptr() 指向之前通过 mlock 锁定过的相同缓冲区。
    // munlock 解锁内存页，不影响内存内容或生命周期。
    // 与 mlock 在同一线程同步调用，无竞态。
    let ret = unsafe { munlock(data.as_ptr() as *const libc::c_void, data.len()) };
```

- [ ] **Step C3: 文档化 core_upgrade.rs 中 unsafe 块**

`src/core/system/core_upgrade.rs:751,756,763,774,776,783,793,795` — `std::env::set_var/remove_var`：

每个 unsafe 块添加 `// SAFETY: set_var/remove_var 在多线程环境中不安全。此处为核心升级模块，在 main.rs 启动早期、tokio 运行时初始化前或升级专用上下文中调用，保证没有其他线程并发读取或修改此环境变量。`

- [ ] **Step C4: 验证**

```bash
cargo clippy -- -D warnings 2>&1
```
Expected: no `undocumented_unsafe_blocks` 错误。

- [ ] **Step C5: 提交**

```bash
git add -A
git commit -m "docs: add SAFETY comments to all unsafe blocks"
```

---

### Task D: API 设计增强

**Files:**
- Modify: `rust/aegis/src/core/error.rs`
- Modify: `rust/aegis/src/core/types.rs`
- Modify: `rust/aegis/src/core/events.rs`
- Modify: `rust/aegis/src/app/state.rs`
- Modify: `rust/aegis/src/adapters/common/trait.rs`
- Modify: `rust/aegis/src/core/search/error.rs`
- (scan for more enums)

- [ ] **Step D1: 为公开枚举添加 `#[non_exhaustive]`**

搜索所有 `pub enum`，为以下关键枚举添加 `#[non_exhaustive]`：

**`src/core/error.rs`**:
```rust
/// 应用统一错误类型
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum AppError {
    // ... existing variants ...
```

**`src/adapters/common/trait.rs`**:
```rust
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Platform {
```

**`src/app/state.rs`**:
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum DestructStep {

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ScheduleFrequency {
```

**`src/core/events.rs`**:
```rust
#[non_exhaustive]
pub enum Severity {
#[non_exhaustive]
pub enum Component {
#[non_exhaustive]
pub enum Status {
#[non_exhaustive]
pub enum CoreEvent {
```

顺序执行：为每个 `pub enum` 添加 `#[non_exhaustive]`，运行 `cargo build` 检查是否破坏 match。

```bash
grep -rn "^pub enum" rust/aegis/src/ --include="*.rs"
```
逐一处理每个匹配结果。

- [ ] **Step D2: 为公开 `Result` 函数添加 `#[must_use]`**

搜索所有返回 `Result<_, AppError>` 或 `anyhow::Result<()>` 的 `pub fn`，确保触发 `#[must_use]`。

在 `src/core/security/crypto.rs` 中已有返回值不检查的模式。添加：

```rust
// 在 Cargo.toml 的 [lints.rust] 中已经包含：
// unused_must_use = "deny"
```

由于大多数 `pub fn` 返回 `Result`，直接在 `lib.rs` 顶部添加 `#![warn(unused_results)]` 不够精确。更好的方式是在 `Cargo.toml` 中添加：

```toml
[lints.rust]
unused_must_use = "deny"
```

然后在 `src/lib.rs` 中当前已有隐藏的返回值不处理场景，逐个修复后：

```bash
cargo build 2>&1 | grep "unused.*Result" || echo "clean"
```

- [ ] **Step D3: 为 AppError 添加 #[source] 链**

**`src/core/error.rs`**: 将字符串字段变体包装 `#[source]`:

```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("服务错误: {0}")]
    Service(String),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 解析错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0} 未安装")]
    NotInstalled(String),

    #[error("端口 {0} 不可用")]
    PortUnavailable(u16),

    #[error("无效参数: {0}")]
    InvalidParameter(String),

    #[error("操作超时")]
    Timeout,

    #[error("未知错误: {0}")]
    Unknown(String),
}
```

`AppError` 当前已通过 `thiserror` 正确使用 `#[from]`，但 `Config(String)` 等字符串变体不保留根本原因。重点关注那些间接包装其他错误的变体。由于这是第一阶段，保持简单——先确保 `Io` 和 `Json` 变体使用 `#[from]`（已是现状）。如果需要追溯，可后续增加 `Config(#[source] Box<dyn std::error::Error + Send + Sync>)`。

- [ ] **Step D4: 验证**

```bash
cargo build 2>&1
cargo test 2>&1
cargo clippy -- -D warnings 2>&1
```

- [ ] **Step D5: 提交**

```bash
git add -A
git commit -m "feat(api): add #[non_exhaustive] and #[must_use] to public types"
```

---

### Task E: CI 工作流

**Files:**
- Create: `.github/workflows/rust.yml`

- [ ] **Step E1: 创建 CI workflow**

创建 `.github/workflows/rust.yml`：

```yaml
name: Rust CI

on:
  push:
    branches: ["main", "rust-skills-*"]
  pull_request:
    branches: ["main"]
  workflow_dispatch:

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: Check & Lint
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: rust/aegis

    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        with:
          components: clippy, rustfmt

      - name: Check formatting
        run: cargo fmt --check

      - name: Clippy lint
        run: cargo clippy -- -D warnings

  test:
    name: Test
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: rust/aegis

    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1

      - name: Build
        run: cargo build --verbose

      - name: Run tests
        run: cargo test --verbose
```

- [ ] **Step E2: 验证**

```bash
# 确认 workflow 文件格式正确
cat .github/workflows/rust.yml | head -5
```

- [ ] **Step E3: 提交**

```bash
git add .github/workflows/rust.yml
git commit -m "ci: add Rust CI workflow with fmt, clippy, and test"
```

---

### 最终验证

- [ ] **运行完整质量门禁**

```bash
cd rust/aegis && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

Expected: format 无输出, clippy 无警告出错, tests 445 passed 0 failed.
