# Aegis Rust Skills Batch 4 — 设计文档

> 修复 API Design + Type Safety + Testing + Project Structure + Anti-patterns 剩余违规项
> 日期: 2026-06-30 | 基于 RUST_SKILLS_REVIEW.md (Categories 4, 8, 9, 12, 14)

---

## 1. 范围概述

基于 main 分支当前状态分析，以下项目已通过之前批次修复，**不纳入**此批次：

| 已完成 | 批次 | 确认 |
|--------|------|------|
| `#[non_exhaustive]` on Platform, IpVersion, Severity, Component, Status, CoreEvent, DestructStep, ScheduleFrequency, AppError | Batch 1 (#133) | main 已验证 |
| `[lints.clippy]` centralized config | Batch 1 (#133) | main 已验证 |
| Documentation -- module-level `//!`, trait/struct `///`, intra-doc links, `# Errors`, `# Examples` | Batch 2 (#134) | main 已验证 |
| Error Handling -- unwrap->expect, `#[source]` on AppError, `# Errors` sections | Batch 3 (#135) | main 已验证 |

---

## 2. 剩余工作分解

### Section A: API Design (Category 4)

#### A.1 `#[must_use]` on builders and Result-returning functions
- **规则:** `api-builder-must-use`, `api-must-use`
- **当前状态:** main 上无任何 `#[must_use]` 属性；`Cargo.toml` 无 `[lints.rust]` 段
- **改动:**
  1. `Cargo.toml`: 添加 `[lints.rust]` 含 `unused_must_use = "deny"`
  2. 为关键公开函数添加 `#[must_use]` -- 至少覆盖 `EventBus::new`, `SecurityManager::new`, `TotpManager::new`, bot handler 返回 `HandlerResult` 的函数
  3. `Hysteria2Config`: 链式 builder 方法添加 `#[must_use]`
  4. `ConfigManager`: key methods on ConfigManager
- **影响文件:** Cargo.toml, hysteria2.rs, config.rs, events.rs, crypto.rs, totp.rs
- **风险:** 低 -- 纯注解，无逻辑变更

#### A.2 `#[non_exhaustive]` on missing enums
- **规则:** `api-non-exhaustive`
- **当前状态:** main 上已验证 9 个枚举有此属性，仍有 3 个遗漏
- **改动:** 为 `Proto` (xray/config.rs), `AuthFailureOutcome` (state.rs), `TimeoutStatus` (state.rs) 添加 `#[non_exhaustive]`
- **影响文件:** `xray/config.rs`, `app/state.rs` + 消费端 match 适配
- **风险:** 中 -- 消费者需要添加通配分支

#### A.3 serde -> optional feature gate
- **规则:** `api-serde-optional`
- **当前状态:** `serde` 和 `serde_json` 在 `[dependencies]` 中为必须依赖
- **改动:**
  1. `Cargo.toml`: `serde = { optional = true }`, `serde_json = { optional = true }`
  2. 所有 `#[derive(Serialize, Deserialize)]` 添加 `#[cfg_attr(feature = "serde", derive(...))]`
  3. 所有 `use serde::*` / `use serde_json::*` 添加 `#[cfg(feature = "serde")]`
  4. 依赖 serde 的依赖也需 feature gate: `secrecy`, `totp-rs`, `chrono`, `rmp-serde`
  5. `features` 段: 添加 `"serde"` 到 `default` feature list
- **影响文件:** Cargo.toml + 15-20 文件
- **风险:** **高** -- 大面积改动，可能破坏编译

#### A.4 Newtype priv fields (TargetId, MessageId)
- **规则:** `api-newtype-safety`
- **当前状态:** `TargetId(pub String)` 和 `MessageId(pub String)` 字段公开
- **改动:** 私有化字段 + 构造函数 + `as_str()`:
  ```rust
  pub struct TargetId(String);
  pub struct MessageId(String);
  impl TargetId {
      pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
      pub fn as_str(&self) -> &str { &self.0 }
  }
  ```
- **影响文件:** `trait.rs` + 所有使用者 (构造/字段访问)
- **风险:** 中 -- API 变更，消费者需适配

#### A.5 Builder pattern for complex constructors
- **规则:** `api-builder-pattern`
- **当前状态:** `ConfigManager::build_reality_vless_inbound()` 接收 12+ 参数
- **改动:** 为 `RealityVlessInbound` 创建 Builder struct
- **影响文件:** `xray/config.rs` + callers
- **风险:** 中 -- 重构公开 API

#### A.6 Debug for AppState
- **规则:** `api-common-traits`
- **当前状态:** `AppState` 无可派生 trait（字段含 `Mutex`）
- **改动:** 手动 `impl fmt::Debug for AppState` -- Mutex 字段显示 `"<locked>"`
- **影响文件:** `app/state.rs`
- **风险:** 低

#### A.7 Parse-don't-validate types
- **规则:** `api-parse-dont-validate`
- **当前状态:** `ConfigValidator` 验证后返回 `Result<(), String>`
- **改动:** 合并到 C.1 实现 -- 验证后返回语义类型
- **联动:** 合并到 Section C.1
- **风险:** 高 -- 需要重新设计配置验证流程

---

### Section C: Type Safety + Testing + Project Structure + Anti-patterns

#### C.1 Newtype validated IDs
- **规则:** `type-newtype-validated` (合并 A.7)
- **当前状态:** 无验证后 newtype；ID 以裸 `String`/`&str` 传递
- **改动:** 创建 `src/core/validated.rs`，定义 `BotToken(String)`, `AdminId(i64)` 等
- **影响文件:** 新文件 + `bootstrap.rs` + ~10 consumer 文件
- **风险:** **高** -- 跨模块 API 重构

#### C.2 Proptest for SecurityManager + TotpManager
- **规则:** `test-proptest-properties`
- **改动:**
  1. `Cargo.toml [dev-dependencies]` 添加 `proptest = "1"`
  2. 新测试文件 `tests/proptest_crypto.rs`: 加密->解密往返
  3. 新测试文件 `tests/proptest_totp.rs`: TOTP 生成->验证
- **风险:** 低 -- 纯测试代码

#### C.3 Doc test examples
- **规则:** `test-doctest-examples`
- **改动:** 为 `IpVersion`, `AppError`, `SecurityManager::encrypt/decrypt`, `TotpManager::verify` 添加 `/// ```rust` 示例
- **影响文件:** `types.rs`, `error.rs`, `crypto.rs`, `totp.rs`
- **风险:** 低

#### C.4 Prelude module + pub use re-exports
- **规则:** `proj-pub-use-reexport`, `proj-prelude-module`
- **改动:**
  1. 新文件 `src/prelude.rs`，重导出高频类型
  2. `lib.rs` 添加 `pub mod prelude;`
- **风险:** 低 -- 纯新增

#### C.5 mockall actual usage
- **规则:** `test-mockall-mocking`
- **改动:** 用 `#[automock]` 替换手写 `MockAdapter`
- **影响文件:** `trait.rs` (add `#[cfg_attr(test, mockall::automock)]`), `state.rs` tests
- **风险:** 低

#### C.6 Clone reduction (hot path)
- **规则:** `anti-clone-excessive`
- **改动:** `tls_probe.rs`: 调整引用语义；`hy2_batch.rs`: 循环外 clone 后引用
- **风险:** 低

#### C.7 Collect chain optimization
- **规则:** `anti-collect-intermediate`
- **改动:** `split().collect::<Vec<&str>>()` -> 直接链式 `split().filter_map()`
- **影响文件:** `firewall_scanner.rs`, `anti_debug.rs`, `ufw.rs`
- **风险:** 低

---

## 3. 实施分层

```
Layer 1: 注解/派生/新文件（低风险，6 项）
  A.1 #[must_use] + [lints.rust], A.2 #[non_exhaustive], A.6 Debug for AppState,
  C.2 Proptest, C.3 Doc tests, C.4 Prelude
  -> 2 个任务

Layer 2: 局部重构（中低风险，5 项）
  A.4 Newtype priv fields, C.5 mockall, C.6 Clone reduction, C.7 Collect chain
  -> 2 个任务

Layer 3: 跨模块重构（高风险，3 项）
  A.3 serde feature gate, A.5 Builder pattern, A.7+C.1 Newtype validated IDs
  -> 2 个任务
```

**总计:** 14 项 -> 6 个任务，每任务独立提交

---

## 4. 约束

- 所有 447 测试保持通过
- `cargo fmt --check` + `cargo clippy -D warnings` clean
- 分层提交，每层 1-2 个 commit
- 高风险项单独任务、单独提交

---

## 5. 自我审查

| 检查项 | 状态 |
|--------|------|
| Placeholder/TODO | 无 |
| 矛盾设计 | 无 -- A.7 与 C.1 明确合并 |
| 范围合理 | 14 项, 25-30 文件 |
| 模糊需求 | 每项明确给出改动 |
| 与之前批次重复 | 已验证 main 分支排除 |
