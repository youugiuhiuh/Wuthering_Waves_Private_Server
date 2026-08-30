# wwps-core 重启/状态回调未注册修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 1.2.7（commit `adb28ef`）引入的缺陷：菜单按钮「🔄 重启 wwps-core」/「📊 wwps-core Status」点击后完全无响应。同时添加回归测试与文档，防止「新增按钮但忘记注册回调」这类问题再次发生。

**Root Cause:** `rust/aegis/src/shared/handlers/mod.rs::dispatch` 使用硬编码白名单分发回调。`adb28ef` 在 `menu.rs` 中添加了两个按钮（`data: "a_wwps_core_restart"` / `data: "a_wwps_core_status"`）及对应 handler 分支，但**未在白名单中注册这两个 ID**（白名单只有 sing-box 的 `a_wwps_box_restart` / `a_wwps_box_status`）。`dispatch` 对未注册 data 返回 `Ok(None)`，`shared/dispatch.rs` 直接 `break`，从不调用 `answer_callback`/`edit_message` → 按钮失效（Telegram/Matrix 均如此）。服务器实测：`systemctl is-active wwps-core` 返回 active、`systemctl restart wwps-core.service` 以 root 执行成功，证明底层逻辑正常，问题 100% 在分发层。

**Architecture:** 将 `dispatch` 的内联白名单提取为**纯函数** `route_callback(data: &str) -> Option<CallbackRoute>`（enum: `Log/Singbox/Warp/Schedule/Ops/Xray/Menu`），`dispatch` 只做路由分发。纯函数可被单元测试直接验证，不依赖 `AppState`。回归测试用 `include_str!("menu.rs")` 扫描所有按钮 `data: "..."` 字面量，断言每个按钮都有路由目标（`route_callback` 返回 Some 或由前置拦截层处理），从机制上杜绝「加按钮忘注册」。

**Tech Stack:** Rust (edition 2024), tokio, rust_i18n。无新依赖（`regex` 已存在于 Cargo.toml，但扫描测试采用零依赖手写解析器）。

**Spec:** 无独立 spec（bugfix，根因与方案见本计划 + 代码注释）。

## Global Constraints

- 工作目录：主工作区 `rust/aegis`（normal 模式，不使用 worktree）
- 保持 `dispatch` 对外行为完全一致，仅修复两个未注册 ID
- 每个任务结束运行 `cargo test -p aegis` 确认全绿
- Rust 质量门禁（完成时执行）：`cargo fmt`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo nextest run`、`cargo test --doc`
- 不引入与本次 bugfix 无关的重构（OpenRC/systemctl 兼容问题仅记录为已知限制，见文档注释）

---

### Task 1: 提取 `route_callback` 纯函数（行为不变的重构）

**Files:**
- Modify: `rust/aegis/src/shared/handlers/mod.rs`（新增 `CallbackRoute` enum + `route_callback`；`dispatch` 改为调用它）

**Interfaces:**
- Produces: `pub(crate) enum CallbackRoute { Log, Singbox, Warp, Schedule, Ops, Xray, Menu }`（derive `Debug, Clone, Copy, PartialEq, Eq`）；`pub(crate) fn route_callback(data: &str) -> Option<CallbackRoute>`
- 逻辑：逐字迁移 `dispatch` 现有 if/matches 白名单，**暂不**包含 `a_wwps_core_restart` / `a_wwps_core_status`

- [ ] **Step 1:** 将 `dispatch` 中的分发条件迁移为 `route_callback`，`dispatch` 改为 `match route_callback(data)` 调用对应 handler
- [ ] **Step 2:** `cargo test -p aegis` 全绿（纯重构，行为不变）

### Task 2: RED —— 写失败测试（TDD）

**Files:**
- Modify: `rust/aegis/src/shared/handlers/mod.rs`（文件尾部新增 `#[cfg(test)] mod tests`）

- [ ] **Step 1:** `test_wwps_core_restart_and_status_route_to_menu`：断言 `route_callback("a_wwps_core_restart") == Some(CallbackRoute::Menu)` 且 `route_callback("a_wwps_core_status") == Some(CallbackRoute::Menu)`
- [ ] **Step 2:** `test_every_menu_button_data_is_routed`：手写解析器提取 `include_str!("menu.rs")` 中所有 `data: "..."` 字面量，断言每个 ID 要么 `route_callback(id).is_some()`，要么由前置拦截层处理（`lang:`/`set_timeout:`/`a_destroy_` 前缀、`a_warp_add_input`）
- [ ] **Step 3:** 运行测试，确认**按预期失败**（两个 ID 返回 None / 扫描到 2 个未注册按钮）——失败原因正确，非拼写/编译错误

### Task 3: GREEN —— 注册两个回调 ID

**Files:**
- Modify: `rust/aegis/src/shared/handlers/mod.rs`（`route_callback` 的 Menu 分支 matches! 列表增加两个 ID）

- [ ] **Step 1:** 在 Menu 路由的 `matches!` 列表中追加 `"a_wwps_core_restart"`、`"a_wwps_core_status"`
- [ ] **Step 2:** 运行 Task 2 的测试，全部通过
- [ ] **Step 3:** `cargo test -p aegis` 全绿（其余既有测试不受影响）

### Task 4: 文档 + 质量门禁

**Files:**
- Modify: `rust/aegis/src/shared/handlers/mod.rs`（`route_callback` 上方添加契约注释：新增菜单按钮必须在此注册，并引用回归测试与 1.2.7 事故）
- Add: `docs/superpowers/plans/2026-08-30-wwps-core-restart-status-fix.md`（本计划即修复文档，含根因与预防机制说明）

- [ ] **Step 1:** 添加 doc comment：说明回调分发契约（按钮 → `route_callback` 或前置拦截层）、预防测试、OpenRC 已知限制（restart/status 当前假定 systemd，安装器支持 OpenRC，需在 OpenRC 部署时扩展）
- [ ] **Step 2:** 执行质量门禁：`cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`
- [ ] **Step 3:** 自审（对照计划逐条核验）+ 输出修复摘要
