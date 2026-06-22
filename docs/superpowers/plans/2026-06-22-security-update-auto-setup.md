# 安全更新自动化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 删除 SecurityUpdate cron 注册，让 systemd 完全接管安全更新流程。

**Architecture:** 语言选择和一键部署做完首次初始化后，systemd apt-daily timers 和 Automatic-Reboot-Time 接管后续全自动流程。

**Tech Stack:** Rust

---

### 文件结构

| 文件 | 职责 | 改动 |
|------|------|------|
| `rust/aegis/src/adapters/telegram/handlers/callback.rs` | 删除 SecurityUpdate cron 注册 | ~7 行删除 |

---

### Task 1: 删除 SecurityUpdate cron

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/callback.rs`

- [ ] **Step 1: 读文件确认上下文**

  读 `callback.rs`，找到语言选择 handler 中 SecurityUpdate task 注册位置。

- [ ] **Step 2: 删除以下 7 行**

  ```rust
  let sec_task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
      aegis::core::system::scheduler::TaskType::SecurityUpdate,
      "0 23 * * *",
      tz,
  );
  let _ = manager.add_new_task(sec_task).await;
  ```

- [ ] **Step 3: 验证**

  ```bash
  cargo fmt && cargo clippy -- -D warnings && cargo test
  ```

  Expected: 全绿

- [ ] **Step 4: Commit**

  ```bash
  git add rust/aegis/src/adapters/telegram/handlers/callback.rs && git commit -m "refactor: remove redundant SecurityUpdate cron, systemd handles everything"
  ```

---

### Task 2: 最终验证

**Files:** N/A

- [ ] **Step 1: 全量验证**

  ```bash
  cargo fmt && cargo clippy -- -D warnings && cargo test
  ```

  Expected: 全绿，0 警告
