# Aegis Rust Skills Code Review 文档

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按 rust-skills 的 14 个类别、179 条规则系统性分析 `rust/aegis` 代码，输出带代码引用的审查文档。

**Architecture:** 使用 codegraph_explore + codebase-memory-mcp 读取关键代码路径，逐类别评估标记状态（通过/违反/不适用/待改进），编译为统一 Markdown 文档。

**Tech Stack:** Rust, codegraph, codebase-memory-mcp

**Output:** `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

---

### Task 1: 分析 Ownership & Borrowing + Error Handling（CRITICAL）

**Files:**
- Analyze: `rust/aegis/src/core/error.rs`, `rust/aegis/src/core/types.rs`, `rust/aegis/src/core/utils.rs`, `rust/aegis/src/adapters/common/trait.rs`
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 读取核心模块代码**

使用 `codegraph_explore` 读取：
- `rust/aegis/src/core/error.rs` — thiserror 使用、自定义错误类型
- `rust/aegis/src/core/types.rs` — Arc/Rc/Cow/Clone/Copy 使用
- `rust/aegis/src/adapters/common/trait.rs` — `&[T]` vs `&Vec<T>`、`&str` vs `&String`
- `rust/aegis/src/security/crypto.rs` — zeroize/secrecy 使用

- [ ] **Step 2: 分析 Ownership & Borrowing（12 条规则）**

检查项：
- `own-borrow-over-clone`: 检查 `.clone()` 调用频率，`codegraph_explore "clone"` 搜索
- `own-slice-over-vec`: 检查 fn 签名中 `&Vec<T>` 和 `&String`
- `own-cow-conditional`: 检查 Cow 使用场景
- `own-arc-shared`: 检查线程安全共享的 Arc 使用（如 `Config`, `AppState`）
- `own-refcell-interior` / `own-mutex-interior` / `own-rwlock-readers`: 检查内部可变性、锁类型选择
- `own-copy-small`: 检查小型类型是否 derive Copy
- `own-lifetime-elision`: 检查不必要的生命周期标注

每项记录：✅ 通过 / ❌ 违反 / ➖ 不适用，带文件:行号引用。

- [ ] **Step 3: 分析 Error Handling（12 条规则）**

检查项：
- `err-thiserror-lib`: 库错误类型用 thiserror
- `err-anyhow-app`: 应用层用 anyhow
- `err-result-over-panic`: 检查 `.unwrap()` 出现
- `err-question-mark`: `?` 传播使用情况
- `err-from-impl`: `#[from]` 自动转换
- `err-source-chain`: `#[source]` 错误链
- `err-lowercase-msg`: 错误消息风格
- `err-doc-errors`: `# Errors` 文档

重点读取 `core/error.rs`，搜索所有 `.unwrap()` / `.expect()` 出现。

- [ ] **Step 4: 写入 Ownership & Error Handling 章节到输出文档**

在 `rust/aegis/docs/RUST_SKILLS_REVIEW.md` 中写入 Category 1 和 2 的分析结果。

---

### Task 2: 分析 Memory Optimization（CRITICAL）

**Files:**
- Analyze: 扫描整个 `rust/aegis/src/`
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 搜索内存优化相关模式**

使用 `grep` 搜索：
- `with_capacity` — 预分配
- `SmallVec` / `ArrayVec` / `ThinVec` 使用
- `Box<dyn>` 使用场景
- `String` vs `&str` 热路径
- `clone_from` 使用
- `clear()` 在循环中复用集合

- [ ] **Step 2: 分析 15 条规则并写入文档**

检查项：
- `mem-with-capacity`: Vec/String 预分配
- `mem-smallvec` / `mem-arrayvec`: 小型集合优化
- `mem-box-large-variant`: 大枚举变体 boxing
- `mem-boxed-slice`: `Box<[T]>` vs `Vec<T>`
- `mem-reuse-collections`: 循环中复用
- `mem-avoid-format` / `mem-write-over-format`: 格式化优化
- `mem-smaller-integers`: 整数大小选择
- `mem-assert-type-size`: 热类型大小断言

写入 Category 3 分析结果到文档。

---

### Task 3: 分析 API Design + Async/Await（HIGH）

**Files:**
- Analyze: `rust/aegis/src/app/`, `rust/aegis/src/core/singbox/`, `rust/aegis/src/core/xray/`
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 读取 API 和 Async 模块代码**

使用 `codegraph_explore` 读取关键 API 定义：
- `app/state.rs` — AppState
- `core/singbox/hysteria2.rs` / `tuic.rs`
- `core/xray/reality.rs` / `config.rs`
- `adapters/common/routing.rs` — RoutingAdapter

- [ ] **Step 2: 分析 API Design（15 条规则）**

检查项：
- `api-builder-pattern`: Builder 模式使用（xray config、singbox config）
- `api-builder-must-use`: `#[must_use]` on builders
- `api-newtype-safety`: newtype 使用
- `api-parse-dont-validate`: 边界解析
- `api-must-use`: `#[must_use]` on Result
- `api-non-exhaustive`: `#[non_exhaustive]`
- `api-from-not-into`: From 实现
- `api-default-impl`: Default 实现
- `api-common-traits`: Debug/Clone/PartialEq
- `api-serde-optional`: serde feature gate

- [ ] **Step 3: 分析 Async/Await（15 条规则）**

检查项：
- `async-tokio-runtime`: Tokio 运行时配置
- `async-no-lock-await`: 锁跨 await 检查
- `async-spawn-blocking`: CPU 密集任务用 spawn_blocking
- `async-tokio-fs`: async 中 tokio::fs
- `async-cancellation-token`: 优雅关闭
- `async-join-parallel` / `async-try-join`: join!/try_join! 使用
- `async-bounded-channel`: 有界 channel
- `async-clone-before-await`: await 前 clone 数据

- [ ] **Step 4: 写入 API Design + Async 章节到文档**

---

### Task 4: 分析 Compiler Optimization（HIGH） + Naming Conventions（MEDIUM）

**Files:**
- Analyze: `Cargo.toml`, 全局扫描
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 读 Cargo.toml 和命名扫描**

读取 `Cargo.toml` 的 release profile 设置。
使用 `codegraph_explore` 扫描命名约定。

- [ ] **Step 2: 分析 Compiler Optimization（12 条规则）**

检查项：
- `opt-inline-small` / `opt-inline-always-rare` / `opt-inline-never-cold`
- `opt-cold-unlikely`: `#[cold]` 使用
- `opt-lto-release`: LTO 设置 — 检查 Cargo.toml
- `opt-codegen-units`: codegen-units 设置
- `opt-target-cpu`: target-cpu 设置
- `opt-bounds-check`: 迭代器 vs 索引
- `opt-simd-portable`: SIMD 使用
- `opt-cache-friendly`: SoA 布局

- [ ] **Step 3: 分析 Naming Conventions（16 条规则）**

检查项：
- `name-types-camel` / `name-variants-camel`
- `name-funcs-snake`
- `name-consts-screaming`
- `name-lifetime-short`
- `name-as-free` / `name-to-expensive` / `name-into-ownership`
- `name-no-get-prefix`
- `name-is-has-bool`
- `name-iter-convention`
- `name-acronym-word`: 如 Uuid vs UUID
- `name-crate-no-rs`: crate 命名

- [ ] **Step 4: 写入 Compiler Optimization + Naming 章节到文档**

---

### Task 5: 分析 Type Safety + Testing + Documentation（MEDIUM）

**Files:**
- Analyze: `rust/aegis/src/core/`, `rust/aegis/tests/`, 全局
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 读取类型定义和测试**

使用 `codegraph_explore` 读取：
- `core/types.rs` — 类型定义
- `tests/` — 测试文件
- 全局搜索 `pub fn` 文档缺失情况

- [ ] **Step 2: 分析 Type Safety（10 条规则）**

检查项：
- `type-newtype-ids`: ID 用 newtype 包装
- `type-newtype-validated`: 验证数据用 newtype
- `type-enum-states`: enum 表达互斥状态
- `type-option-nullable`: Option 表达可空
- `type-result-fallible`: Result 表达可能失败
- `type-phantom-marker`: PhantomData 类型标记
- `type-no-stringly`: 避免 stringly-typed API
- `type-repr-transparent`: FFI newtype 的 repr

- [ ] **Step 3: 分析 Testing（13 条规则）**

检查项：
- `test-cfg-test-module`: `#[cfg(test)] mod tests`
- `test-use-super`: `use super::*`
- `test-integration-dir`: `tests/` 目录
- `test-descriptive-names`: 描述性测试名
- `test-arrange-act-assert`: AAA 结构
- `test-proptest-properties`: property-based testing
- `test-mockall-mocking`: mock 使用
- `test-tokio-async`: `#[tokio::test]`
- `test-criterion-bench`: criterion benchmark
- `test-doctest-examples`: doc example 测试

- [ ] **Step 4: 分析 Documentation（11 条规则）**

检查项：
- `doc-all-public`: 所有 pub 项有文档
- `doc-module-inner`: 模块级 `//!`
- `doc-examples-section`: `# Examples`
- `doc-errors-section`: `# Errors`
- `doc-panics-section`: `# Panics`
- `doc-safety-section`: `# Safety` for unsafe
- `doc-intra-links`: 内部文档链接
- `doc-cargo-metadata`: Cargo.toml 元数据

- [ ] **Step 5: 写入 Type Safety + Testing + Documentation 章节到文档**

---

### Task 6: 分析 Performance Patterns + Project Structure（MEDIUM / LOW）

**Files:**
- Analyze: 全局扫描 + `Cargo.toml`
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 搜索性能模式 + 项目结构分析**

- 搜索 `entry()` / `drain()` / `extend()` 使用
- 搜索 `collect()` 中间迭代器
- 检查目录结构、workspace 配置

- [ ] **Step 2: 分析 Performance Patterns（11 条规则）**

检查项：
- `perf-iter-over-index`: 迭代器 vs 手动索引
- `perf-iter-lazy`: 延迟迭代器
- `perf-collect-once`: 避免 collect 中间迭代器
- `perf-entry-api`: HashMap entry API
- `perf-drain-reuse`: drain 复用
- `perf-extend-batch`: extend 批量插入
- `perf-release-profile`: release profile 优化

- [ ] **Step 3: 分析 Project Structure（11 条规则）**

检查项：
- `proj-lib-main-split`: lib.rs / main.rs 职责
- `proj-mod-by-feature`: 按 feature 组织模块
- `proj-flat-small`: 小项目扁平化
- `proj-pub-crate-internal`: `pub(crate)` visibility
- `proj-pub-use-reexport`: `pub use` 重新导出
- `proj-workspace-deps`: workspace 依赖继承

- [ ] **Step 4: 写入 Performance + Project Structure 章节到文档**

---

### Task 7: 分析 Clippy/Linting + Anti-patterns（LOW / REFERENCE）+ 汇总

**Files:**
- Analyze: `Cargo.toml`, 全局
- Output: `rust/aegis/docs/RUST_SKILLS_REVIEW.md`

- [ ] **Step 1: 搜索反模式 + 检查 lint 配置**

- 搜索 `.unwrap()`、`.expect()` 在非测试代码
- 搜索锁跨 await 模式
- 搜索 `&String` / `&Vec<T>` 参数
- 搜索 `Box<dyn Trait>` vs `impl Trait`
- 搜索 `format!()` 在热路径

- [ ] **Step 2: 分析 Clippy & Linting（11 条规则）**

检查项：
- `lint-deny-correctness`: `#![deny(clippy::correctness)]`
- `lint-warn-suspicious` / `lint-warn-style` / `lint-warn-complexity` / `lint-warn-perf`
- `lint-missing-docs`: `#![warn(missing_docs)]`
- `lint-unsafe-doc`: undocumented_unsafe_blocks
- `lint-rustfmt-check`: cargo fmt --check
- `lint-workspace-lints`: workspace lint 配置

- [ ] **Step 3: 分析 Anti-patterns（15 条规则）**

检查项：
- `anti-unwrap-abuse`: 检查非测试 unwrap
- `anti-expect-lazy`: 检查 expect 消息质量
- `anti-clone-excessive`: 检查不必要的 clone
- `anti-lock-across-await`: 锁跨 await
- `anti-string-for-str` / `anti-vec-for-slice`: 参数类型
- `anti-index-over-iter`: 索引 vs 迭代器
- `anti-panic-expected`: 对预期错误 panic
- `anti-type-erasure`: Box<dyn> vs impl Trait
- `anti-format-hot-path`: 热路径 format!
- `anti-stringly-typed`: stringly-typed API

- [ ] **Step 4: 写入 Clippy + Anti-patterns 章节 + 汇总**

写入 Categories 13 和 14。
添加汇总表：各类别通过/违反/待改进计数。
添加最高优先级改进建议。
