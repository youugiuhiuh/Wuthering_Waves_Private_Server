# SecurityUpdate 定时任务实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将「配置自动更新」从一键操作改造为分步交互式流程（周期→时区→小时→分钟→确认），复用现有 ScheduleInputState + 调度器，新增 `SecurityUpdate` 任务类型，确认后一体化执行安装包+写配置+注册调度任务。

**Architecture:** 在现有调度系统中新增 `TaskType::SecurityUpdate`，复用 `ScheduleInputState` 的分步选择 UI 流程。用户点击「配置自动更新」后不再直接执行 maintenance，而是进入分步选择界面。确认后：先执行 `Operations::perform_maintenance_with_reboot_time()` 安装包和写配置（配置中写入用户选择的重启时间），再通过调度器注册定时任务。

**Tech Stack:** Rust, teloxide (Telegram Bot), tokio-cron-scheduler, serde (JSON 序列化)

---

## 文件结构

| 文件 | 职责 | 操作 |
|---|---|---|
| `src/logic/system/scheduler/task_types.rs` | 定义 `TaskType` 枚举，`execute()`，`get_display_name()` | 修改：新增 `SecurityUpdate` 变体 |
| `src/logic/system/operations.rs` | `AutoUpdateConfigurator` 生成配置，`Operations::perform_maintenance()` | 修改：`debian_config()` 改为接受 `reboot_time` 参数；新增 `perform_maintenance_with_reboot_time()` 和 `perform_security_update_task()` |
| `src/handlers/schedule.rs` | 定时任务 UI handler（分步选择、确认） | 修改：新增 `SecurityUpdate` UI 处理，确认后触发 maintenance |
| `src/handlers/ops.rs` | 系统运维 handler（a_sys_maint） | 修改：改为跳转到分步选择流程入口 |
| `src/handlers/mod.rs` | handler 路由分发 | 无修改（现有路由已覆盖） |

---

### Task 1: 在 TaskType 枚举中新增 SecurityUpdate 变体

**Files:**
- Modify: `rust/tgbot/src/logic/system/scheduler/task_types.rs:8-14`

- [ ] **Step 1: 在 `TaskType` 枚举中新增 `SecurityUpdate`**

在 `TaskType` 枚举中，在 `ReloadCore` 和 `Unknown` 之间添加 `SecurityUpdate`：

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum TaskType {
    GeoUpdate,
    Reboot,
    ReloadCore,
    SecurityUpdate,
    #[serde(other)]
    Unknown,
}
```

- [ ] **Step 2: 在 `get_display_name()` 中添加 SecurityUpdate 分支**

```rust
pub fn get_display_name(&self) -> &str {
    match self {
        TaskType::GeoUpdate => "GeoData 更新 (Update GeoData)",
        TaskType::Reboot => "系统重启 (Reboot)",
        TaskType::ReloadCore => "重载核心 (Reload Core)",
        TaskType::SecurityUpdate => "安全更新 (Security Update)",
        TaskType::Unknown => "未知任务 (已弃用)",
    }
}
```

- [ ] **Step 3: 在 `execute()` 中添加 SecurityUpdate 分支**

```rust
TaskType::SecurityUpdate => {
    log::info!("执行安全更新定时任务...");
    let _ = bot
        .send_message(chat_id, "⏳ [定时任务] 开始执行安全更新...")
        .await;

    let result = Operations::perform_security_update_task().await;

    report_result(
        bot,
        chat_id,
        "安全更新",
        "✅ [定时任务] 安全更新执行完成。",
        result,
    )
    .await
}
```

注意：需确认文件顶部已有 `use crate::logic::operations::Operations;`（当前已有）。

- [ ] **Step 4: 添加 SecurityUpdate 相关测试**

在 `tests` 模块中添加：

```rust
#[test]
fn test_security_update_display_name() {
    assert_eq!(
        TaskType::SecurityUpdate.get_display_name(),
        "安全更新 (Security Update)"
    );
}

#[test]
fn test_security_update_serialization() {
    let json = r#""SecurityUpdate""#;
    let task_type: TaskType = serde_json::from_str(json).unwrap();
    assert_eq!(task_type, TaskType::SecurityUpdate);
}
```

- [ ] **Step 5: 运行测试**

Run: `cd /home/kde/Github/Wuthering_Waves_Private_Server/rust/tgbot && cargo test --lib scheduler::task_types -- --nocapture`
Expected: 所有测试通过

- [ ] **Step 6: 提交**

```bash
git add rust/tgbot/src/logic/system/scheduler/task_types.rs
git commit -m "feat: add SecurityUpdate variant to TaskType enum"
```

---

### Task 2: 修改 AutoUpdateConfigurator 支持自定义重启时间 + 新增 maintenance/更新方法

**Files:**
- Modify: `rust/tgbot/src/logic/system/operations.rs`

- [ ] **Step 1: 修改 `debian_config()` 接受 `reboot_time` 参数**

将 `fn debian_config() -> String` 改为 `fn debian_config(reboot_time: &str) -> String`：

```rust
fn debian_config(reboot_time: &str) -> String {
    format!(
        r#"Unattended-Upgrade::Allowed-Origins {{
    "${{distro_id}}:${{distro_codename}}-security";
}};
Unattended-Upgrade::AutoFixInterruptedDpkg "true";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "true";
Unattended-Upgrade::Automatic-Reboot-Time "{}";
"#,
        reboot_time
    )
}
```

- [ ] **Step 2: 修改 `generate_config()` 接受 `reboot_time` 参数**

```rust
pub fn generate_config(distro: DistroFamily, reboot_time: &str) -> String {
    match distro {
        DistroFamily::Debian => Self::debian_config(reboot_time),
        DistroFamily::Rhel => Self::rhel_config(),
    }
}
```

- [ ] **Step 3: 修改 `write_config()` 接受 `reboot_time` 参数**

```rust
pub async fn write_config(distro: DistroFamily, reboot_time: &str) -> Result<()> {
    let config = Self::generate_config(distro, reboot_time);
    let path = distro.auto_update_config_path();
    tokio::fs::write(path, &config)
        .await
        .with_context(|| format!("❌ 写入配置文件 {} 失败", path))?;
    Ok(())
}
```

- [ ] **Step 4: 拆分 `perform_maintenance()` — 新增 `perform_maintenance_with_reboot_time()`**

将现在的 `perform_maintenance()` 重构为：

```rust
pub async fn perform_maintenance() -> Result<String> {
    Self::perform_maintenance_with_reboot_time("03:00").await
}

pub async fn perform_maintenance_with_reboot_time(reboot_time: &str) -> Result<String> {
    // ... 与现有 perform_maintenance 逻辑完全一致
    // 唯一区别：步骤4中调用 write_config(distro, reboot_time) 而非 write_config(distro)
}
```

核心改动只是在步骤4中将 `AutoUpdateConfigurator::write_config(distro)` 改为 `AutoUpdateConfigurator::write_config(distro, reboot_time)`。

- [ ] **Step 5: 新增 `perform_security_update_task()`**

在 `Operations` impl 中添加：

```rust
pub async fn perform_security_update_task() -> Result<()> {
    let distro = DistroFamily::detect().await?;
    match distro {
        DistroFamily::Debian => {
            run_cmd_checked(
                "sh",
                &["-c", "unattended-upgrade -v"],
                TIMEOUT_APT,
            )
            .await
            .context("执行 unattended-upgrade 失败")?;
        }
        DistroFamily::Rhel => {
            run_cmd_checked(
                "sh",
                &["-c", "dnf automatic --installupdates"],
                TIMEOUT_APT,
            )
            .await
            .context("执行 dnf automatic 失败")?;
        }
    }
    Ok(())
}
```

- [ ] **Step 6: 更新测试**

```rust
#[test]
fn test_debian_config_content() {
    let config = AutoUpdateConfigurator::generate_config(DistroFamily::Debian, "03:00");
    assert!(config.contains("Allowed-Origins"));
    assert!(config.contains("security"));
    assert!(config.contains("AutoFixInterruptedDpkg"));
    assert!(config.contains("Automatic-Reboot"));
    assert!(config.contains("Automatic-Reboot-Time"));
    assert!(config.contains("03:00"));
    assert!(config.contains("Remove-Unused-Dependencies"));
    assert!(!config.contains("MailOnlyOnError"));
    assert!(!config.contains("Unattended-Upgrade \"1\""));
}
```

同时新增测试：

```rust
#[test]
fn test_debian_config_custom_reboot_time() {
    let config = AutoUpdateConfigurator::generate_config(DistroFamily::Debian, "05:30");
    assert!(config.contains("Automatic-Reboot-Time \"05:30\";"));
}
```

- [ ] **Step 7: 运行测试**

Run: `cd /home/kde/Github/Wuthering_Waves_Private_Server/rust/tgbot && cargo test --lib -- operations`
Expected: 所有测试通过

- [ ] **Step 8: 提交**

```bash
git add rust/tgbot/src/logic/system/operations.rs
git commit -m "feat: parameterize reboot_time in AutoUpdateConfigurator, add perform_security_update_task"
```

---

### Task 3: 修改 schedule.rs — 支持 SecurityUpdate 任务类型

**Files:**
- Modify: `rust/tgbot/src/handlers/schedule.rs`

- [ ] **Step 1: 在 `schedule_task_name()` 中添加 SecurityUpdate**

```rust
pub(crate) fn schedule_task_name(task_type: &TaskType) -> &'static str {
    match task_type {
        TaskType::Unknown => "未知任务",
        TaskType::Reboot => "系统重启",
        TaskType::GeoUpdate => "GeoData 更新",
        TaskType::ReloadCore => "重载核心",
        TaskType::SecurityUpdate => "安全更新",
    }
}
```

- [ ] **Step 2: 在 `s_add_custom_menu` 中添加 SecurityUpdate 选项**

在 `s_add_custom_menu` 的 keyboard 构建中，在系统重启行之后添加：

```rust
vec![
    InlineKeyboardButton::callback("安全更新 - 每天", "s_custom:secupd:daily"),
    InlineKeyboardButton::callback("安全更新 - 每周", "s_custom:secupd:weekly"),
],
```

- [ ] **Step 3: 在 `s_custom:` 分支解析中添加 SecurityUpdate**

在 `d if d.starts_with("s_custom:")` 的 match 中添加：

```rust
(Some("secupd"), Some("daily")) => {
    (TaskType::SecurityUpdate, ScheduleFrequency::Daily)
}
(Some("secupd"), Some("weekly")) => {
    (TaskType::SecurityUpdate, ScheduleFrequency::Weekly)
}
```

- [ ] **Step 4: 提交**

```bash
git add rust/tgbot/src/handlers/schedule.rs
git commit -m "feat: add SecurityUpdate task type to schedule UI"
```

---

### Task 4: 修改 ops.rs — 「配置自动更新」按钮跳转到分步选择流程

**Files:**
- Modify: `rust/tgbot/src/handlers/ops.rs`

- [ ] **Step 1: 修改 `a_sys_maint` handler**

将 `a_sys_maint` match arm 替换为跳转到分步选择流程：

```rust
"a_sys_maint" => {
    ctx.state.remove_schedule_input(ctx.chat_id).await;
    ctx.state
        .insert_schedule_input(
            ctx.chat_id,
            ScheduleInputState {
                updated_at: Instant::now(),
                task_type: TaskType::SecurityUpdate,
                frequency: ScheduleFrequency::Daily,
                timezone: "UTC".to_string(),
                day_of_week: None,
                hour: None,
                minute: None,
                return_to: "m_sys_cmd".to_string(),
            },
        )
        .await;

    let Some(input_state) = ctx.state.schedule_input_snapshot(ctx.chat_id).await else {
        ctx.bot
            .answer_callback_query(ctx.q.id.clone())
            .text("⚠️ 初始化配置状态失败，请重试。")
            .await?;
        return Ok(HandlerAction::Done);
    };
    let text = build_custom_schedule_text(&input_state);
    let ret = input_state.return_to.clone();

    ctx.bot
        .edit_message_text(ctx.chat_id, ctx.msg_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(build_custom_schedule_keyboard(&ret))
        .await?;
}
```

- [ ] **Step 2: 添加需要的 import**

在 `ops.rs` 顶部添加：

```rust
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use crate::logic::scheduler::task_types::TaskType;
use std::time::Instant;
use super::schedule::{build_custom_schedule_text, build_custom_schedule_keyboard};
```

需要确保 `build_custom_schedule_text` 和 `build_custom_schedule_keyboard` 在 `schedule.rs` 中是 `pub(crate)` 可见的（当前它们已经是 `pub(crate)`）。

- [ ] **Step 3: 提交**

```bash
git add rust/tgbot/src/handlers/ops.rs
git commit -m "feat: redirect auto-update button to step-by-step scheduler flow"
```

---

### Task 5: 修改 schedule.rs 确认逻辑 — SecurityUpdate 确认后执行维护

**Files:**
- Modify: `rust/tgbot/src/handlers/schedule.rs`

- [ ] **Step 1: 修改 `s_custom_confirm` — 提取 hour/minute，SecurityUpdate 时触发 maintenance**

在 `s_custom_confirm` handler 中，修改闭包提取额外的 `hour` 和 `minute`：

将：
```rust
let Some((cron, task_type, timezone, return_to)) = ctx
    .state
    .with_schedule_input(ctx.chat_id, |input| {
        input.updated_at = Instant::now();
        (
            build_cron_from_custom_state(input),
            input.task_type.clone(),
            input.timezone.clone(),
            input.return_to.clone(),
        )
    })
    .await
else { ... };
```

改为：
```rust
let Some((cron, task_type, timezone, return_to, hour, minute)) = ctx
    .state
    .with_schedule_input(ctx.chat_id, |input| {
        input.updated_at = Instant::now();
        (
            build_cron_from_custom_state(input),
            input.task_type.clone(),
            input.timezone.clone(),
            input.return_to.clone(),
            input.hour,
            input.minute,
        )
    })
    .await
else { ... };
```

- [ ] **Step 2: 在任务添加成功后，SecurityUpdate 触发维护**

在 `Ok(_)` 分支中添加 SecurityUpdate 的维护操作：

```rust
Ok(_) => {
    ctx.bot
        .answer_callback_query(ctx.q.id.clone())
        .text("✅ 任务添加成功")
        .await?;

    if task_type == TaskType::SecurityUpdate {
        let reboot_time = format!("{:02}:{:02}", hour.unwrap_or(3), minute.unwrap_or(0));
        let bot_clone = ctx.bot.clone();
        let chat_id_clone = ctx.chat_id;
        tokio::spawn(async move {
            match Operations::perform_maintenance_with_reboot_time(&reboot_time).await {
                Ok(log) => {
                    let log_tail = if log.len() > 3000 {
                        format!("... (Truncated)\n{}", &log[log.len() - 2000..])
                    } else {
                        log
                    };
                    let _ = bot_clone
                        .send_message(
                            chat_id_clone,
                            format!("📋 <b>安全更新初始配置日志</b>\n\n<pre>{}</pre>", log_tail),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                Err(e) => {
                    let _ = bot_clone
                        .send_message(chat_id_clone, format!("❌ <b>安全更新初始配置失败</b>\n\n{}", e))
                        .parse_mode(ParseMode::Html)
                        .await;
                }
            }
        });
    }

    let back_label = if return_to == "a_geo_sched_menu" {
        "⬅️ 返回 Geo 调度"
    } else if return_to == "m_sys_cmd" {
        "⬅️ 返回系统指令"
    } else {
        "⬅️ 返回定时任务"
    };
    // ... rest of existing success response ...
}
```

- [ ] **Step 3: 添加 import**

在 schedule.rs 顶部添加：

```rust
use crate::logic::operations::Operations;
```

- [ ] **Step 4: 提交**

```bash
git add rust/tgbot/src/handlers/schedule.rs
git commit -m "feat: execute maintenance on SecurityUpdate task confirmation"
```

---

### Task 6: 集成验证 — 编译和测试

- [ ] **Step 1: 编译项目**

Run: `cd /home/kde/Github/Wuthering_Waves_Private_Server/rust/tgbot && cargo build 2>&1`
Expected: 编译成功，无错误

- [ ] **Step 2: 运行所有测试**

Run: `cd /home/kde/Github/Wuthering_Waves_Private_Server/rust/tgbot && cargo test --lib 2>&1`
Expected: 所有测试通过

- [ ] **Step 3: 最终确认 — 验证路由完整性**

确认以下用户交互流程可以端到端走通：
1. 点击「系统指令」→ 「配置自动更新」(`a_sys_maint`)
2. 进入分步选择界面（显示默认 `SecurityUpdate` + `Daily` + `UTC`）
3. 选择周期（每天/每周）
4. 选择时区
5. 选择小时
6. 选择分钟
7. 确认创建任务
8. 首次确认后：安装包 + 写配置（含用户选择的重启时间） + 注册 cron 定时任务
9. 后续定时执行时：仅执行安全更新命令