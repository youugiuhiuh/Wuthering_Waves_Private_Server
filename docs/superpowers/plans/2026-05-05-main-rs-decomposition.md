# main.rs 分解重构实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 main.rs（5,193 行）拆分为 6 个领域处理器文件 + 1 个生命周期文件，降低单文件复杂度。

**Architecture:** 严格只移动代码，不重写逻辑。将 main.rs 中的函数按领域分类搬迁到 handlers/ 和 app/lifecycle.rs，每步执行 cargo check 验证编译通过。

**Tech Stack:** Rust, teloxide 0.13, tokio

**Working directory:** `rust/tgbot/`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/main.rs` | Modify (5,193 → ~150 行) | 仅 main() + tests + 常量 + Dispatcher 组装 |
| `src/handlers/mod.rs` | Create | 模块声明 + re-export |
| `src/handlers/command.rs` | Create | Command 枚举 + handle_command + handle_message + process_auth_code + send_main_menu + register_bot_commands |
| `src/handlers/callback.rs` | Create | handle_callback 路由 + 独立处理函数 |
| `src/handlers/proxy.rs` | Create | show_reality_batch_prompt + show_reality_qty_prompt + trigger_reality_auto_init |
| `src/handlers/security.rs` | Create | validate_idx |
| `src/handlers/system.rs` | Create | schedule_* 系列函数 + build_* 系列函数 + format_duration_human + save_config + escape_html + validate_hash_prefix |
| `src/app/lifecycle.rs` | Create | notify_online + notify_upgrade_success + notify_bbr3_reboot_result + run_startup_checks |
| `src/app/mod.rs` | Modify (加 pub mod lifecycle) | 新增模块声明 |

---

## Task 1: 创建目录骨架

**Files:**
- Create: `src/handlers/mod.rs`
- Create: `src/handlers/command.rs`
- Create: `src/handlers/callback.rs`
- Create: `src/handlers/proxy.rs`
- Create: `src/handlers/security.rs`
- Create: `src/handlers/system.rs`
- Create: `src/app/lifecycle.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 创建 handlers 目录和空模块文件**

创建 `src/handlers/` 目录，在其中创建所有空模块文件：

`src/handlers/mod.rs`:
```rust
pub mod callback;
pub mod command;
pub mod proxy;
pub mod security;
pub mod system;
```

`src/handlers/command.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

`src/handlers/callback.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

`src/handlers/proxy.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

`src/handlers/security.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

`src/handlers/system.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

- [ ] **Step 2: 创建 app/lifecycle.rs 空文件**

`src/app/lifecycle.rs`:
```rust
// 暂时留空，将在后续任务中填充
```

- [ ] **Step 3: 更新 app/mod.rs**

在 `src/app/mod.rs` 末尾追加：
```rust
pub mod lifecycle;
```

现有内容：
```rust
pub mod auth;
pub mod destruct_flow;
pub mod state;
```
改为：
```rust
pub mod auth;
pub mod destruct_flow;
pub mod lifecycle;
pub mod state;
```

- [ ] **Step 4: 在 main.rs 添加 mod handlers**

在 `src/main.rs` 的 `mod app;` 后添加 `mod handlers;`。

当前第1-3行：
```rust
// mod logic; // Moved to lib.rs
mod app;
mod bootstrap;
```
改为：
```rust
// mod logic; // Moved to lib.rs
mod app;
mod bootstrap;
mod handlers;
```

- [ ] **Step 5: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功（空模块文件不应产生错误）

- [ ] **Step 6: 提交**

```bash
git add src/handlers/ src/app/lifecycle.rs src/app/mod.rs src/main.rs
git commit -m "refactor: create handlers module scaffolding for main.rs decomposition"
```

---

## Task 2: 迁移 lifecycle 通知函数

**Files:**
- Modify: `src/main.rs` (删除 notify_online, notify_upgrade_success, notify_bbr3_reboot_result)
- Modify: `src/app/lifecycle.rs` (填充这三个函数 + run_startup_checks 入口)

- [ ] **Step 1: 将三个通知函数移入 app/lifecycle.rs**

从 `src/main.rs` 剪切以下函数（行 5107-5193）：
- `async fn notify_online(bot: &Bot, admin_id: i64) -> Result<()>`
- `async fn notify_upgrade_success(bot: &Bot, admin_id: i64) -> Result<()>`
- `async fn notify_bbr3_reboot_result(bot: &Bot, admin_id: i64) -> Result<()>`

粘贴到 `src/app/lifecycle.rs`，并添加必要的 use 语句和统一入口函数：

```rust
use std::fs;
use std::path::Path;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};
use tgbot::logic::system::maintenance::BBR3_PENDING_FLAG_FILE;
use tgbot::logic::system::monitor::SystemMonitor;
use tgbot::logic::upgrade::UPGRADE_FLAG_FILE;
use crate::bootstrap::BOT_VERSION;

pub async fn run_startup_checks(bot: &Bot, admin_id: i64) -> Result<()> {
    let _ = notify_upgrade_success(bot, admin_id).await;
    let _ = notify_bbr3_reboot_result(bot, admin_id).await;
    let _ = notify_online(bot, admin_id).await;
    Ok(())
}

// 以下三个函数从 main.rs 原样搬入，不做任何逻辑修改
async fn notify_online(bot: &Bot, admin_id: i64) -> Result<()> {
    // ... 原样搬入 ...
}

async fn notify_upgrade_success(bot: &Bot, admin_id: i64) -> Result<()> {
    // ... 原样搬入 ...
}

async fn notify_bbr3_reboot_result(bot: &Bot, admin_id: i64) -> Result<()> {
    // ... 原样搬入 ...
}
```

注意：这三个函数使用的 import 需要重新声明。原 main.rs 中这些函数使用的类型：
- `std::fs` → `fs::read_to_string`, `fs::remove_file`
- `std::path::Path`
- `anyhow::Result`
- `teloxide::prelude::*` (Bot)
- `teloxide::types::{ChatId, ParseMode}`
- `tgbot::logic::system::maintenance::BBR3_PENDING_FLAG_FILE` (原路径 `tgbot::logic::maintenance::BBR3_PENDING_FLAG_FILE`，注意 re-export)
- `tgbot::logic::system::monitor::SystemMonitor` (原路径 `tgbot::logic::system::SystemMonitor`)
- `tgbot::logic::upgrade::UPGRADE_FLAG_FILE` (原路径 `tgbot::logic::upgrade::UPGRADE_FLAG_FILE`)

具体 import 路径需要参照原 main.rs 的 use 声明来定。完成后从 main.rs 中删除这三个函数。

- [ ] **Step 2: 更新 main.rs 中的调用**

在 main.rs 的 `main()` 函数中，将：
```rust
let _ = notify_upgrade_success(&bot_for_init, admin_id).await;
let _ = notify_bbr3_reboot_result(&bot_for_init, admin_id).await;
let _ = notify_online(&bot_for_init, admin_id).await;
```
改为：
```rust
crate::app::lifecycle::run_startup_checks(&bot_for_init, admin_id).await;
```

同时从 main.rs 删除已迁移走的 `use` 导入（仅删除那些仅被这三个函数使用的导入）。

- [ ] **Step 3: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功

- [ ] **Step 4: 验证测试**

Run: `cd rust/tgbot && cargo test`
Expected: 全部测试通过

- [ ] **Step 5: 提交**

```bash
git add src/main.rs src/app/lifecycle.rs
git commit -m "refactor: extract lifecycle notification functions to app/lifecycle.rs"
```

---

## Task 3: 迁移工具函数到 handlers/system.rs

**Files:**
- Modify: `src/main.rs` (删除工具函数)
- Modify: `src/handlers/system.rs` (填充工具函数)

- [ ] **Step 1: 将 format_duration_human 移入 handlers/system.rs**

从 main.rs 剪切行 61-77 的 `fn format_duration_human(secs: u64) -> String` 函数，粘贴到 `src/handlers/system.rs`。

同时将 main.rs 中 `#[cfg(test)] mod tests` 里的三个 `format_duration_human` 测试（行 5087-5104）移入 `src/handlers/system.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_human_seconds() {
        assert_eq!(format_duration_human(0), "0秒");
        assert_eq!(format_duration_human(45), "45秒");
    }

    #[test]
    fn format_duration_human_minutes() {
        assert_eq!(format_duration_human(60), "1分钟");
        assert_eq!(format_duration_human(90), "1分钟");
        assert_eq!(format_duration_human(120), "2分钟");
    }

    #[test]
    fn format_duration_human_hours_and_days() {
        assert_eq!(format_duration_human(3600), "1小时");
        assert_eq!(format_duration_human(3661), "1小时1分");
        assert!(format_duration_human(86400).starts_with("1天"));
    }
}
```

在 system.rs 顶部添加 `pub(crate)` 可见性：
```rust
pub(crate) fn format_duration_human(secs: u64) -> String {
```

在 main.rs 中添加对 system 模块的引用：
```rust
use crate::handlers::system::format_duration_human;
```

- [ ] **Step 2: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功

- [ ] **Step 3: 将 schedule_* 系列函数移入 handlers/system.rs**

从 main.rs 剪切以下函数（约行 562-755）：
- `fn schedule_task_name(task_type: &TaskType) -> &'static str`
- `fn schedule_frequency_name(frequency: &ScheduleFrequency) -> &'static str`
- `fn weekday_label(day: &str) -> &'static str`
- `fn timezone_label(timezone: &str) -> &'static str`
- `fn build_custom_schedule_text(input: &ScheduleInputState) -> String`
- `fn build_custom_schedule_keyboard(return_to: &str) -> InlineKeyboardMarkup`
- `fn build_custom_day_keyboard() -> InlineKeyboardMarkup`
- `fn build_custom_hour_keyboard() -> InlineKeyboardMarkup`
- `fn build_custom_minute_keyboard() -> InlineKeyboardMarkup`
- `fn build_custom_timezone_keyboard() -> InlineKeyboardMarkup`
- `fn build_cron_from_custom_state(input: &ScheduleInputState) -> Option<String>`

粘贴到 `handlers/system.rs`，将可见性改为 `pub(crate)`。

需要添加的 import（从 main.rs 中对照）：
```rust
use teloxide::types::InlineKeyboardButton;
use teloxide::types::InlineKeyboardMarkup;
use crate::app::state::{ScheduleFrequency, ScheduleInputState};
use tgbot::logic::scheduler::task_types::TaskType;
```

在 main.rs 中添加：
```rust
use crate::handlers::system::*;
```
（或者在 callback.rs 和其他需要的地方单独 import）

- [ ] **Step 4: 将 save_config 移入 handlers/system.rs**

从 main.rs 剪切 `async fn save_config(state: &Arc<AppState>) -> Result<()>` 函数（约行 4905-4919），粘贴到 `handlers/system.rs`，改为 `pub(crate)`。

需要添加的 import：
```rust
use std::sync::Arc;
use anyhow::Result;
use crate::app::state::AppState;
```

- [ ] **Step 5: 将 escape_html 移入 handlers/system.rs**

从 main.rs 剪切 `fn escape_html(s: &str) -> String`（行 79-84），粘贴到 `handlers/system.rs`，改为 `pub(crate)`。

注：escape_html 主要被 callback.rs 使用，但放在 system.rs 作为共享工具函数更合理（多个模块可能用到），callback.rs 通过 `use crate::handlers::system::escape_html` 引用。

- [ ] **Step 6: 将 validate_hash_prefix 移入 handlers/system.rs**

从 main.rs 剪切 `fn validate_hash_prefix(prefix: &str) -> Result<&str>`（行 86-97），粘贴到 `handlers/system.rs`，改为 `pub(crate)`。

- [ ] **Step 7: 将 validate_idx 移入 handlers/system.rs**

从 main.rs 剪切 `fn validate_idx(idx: usize, max: usize, field_name: &str) -> Result<()>`（行 99-110），粘贴到 `handlers/system.rs`，改为 `pub(crate)`。

- [ ] **Step 8: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功

- [ ] **Step 9: 验证测试**

Run: `cd rust/tgbot && cargo test`
Expected: 全部测试通过

- [ ] **Step 10: 提交**

```bash
git add src/main.rs src/handlers/system.rs
git commit -m "refactor: extract utility functions to handlers/system.rs"
```

---

## Task 4: 迁移 proxy 辅助函数到 handlers/proxy.rs

**Files:**
- Modify: `src/main.rs` (删除 proxy 函数)
- Modify: `src/handlers/proxy.rs` (填充)

- [ ] **Step 1: 将三个 proxy 函数移入 handlers/proxy.rs**

从 main.rs 剪切以下函数：

1. `async fn show_reality_batch_prompt(bot: &Bot, chat_id: ChatId, msg_id: MessageId, proto: Proto) -> ResponseResult<()>` （约行 121-244）
2. `async fn show_reality_qty_prompt(bot: &Bot, chat_id: ChatId, msg_id: MessageId, proto: Proto) -> ResponseResult<()>` （约行 246-269）
3. `fn trigger_reality_auto_init(bot: Bot, chat_id: ChatId, msg_id: MessageId)` （约行 270-274）

粘贴到 `src/handlers/proxy.rs`，改为 `pub(crate)`：

```rust
pub(crate) async fn show_reality_batch_prompt(...) -> ResponseResult<()> { ... }
pub(crate) async fn show_reality_qty_prompt(...) -> ResponseResult<()> { ... }
pub(crate) fn trigger_reality_auto_init(bot: Bot, chat_id: ChatId, msg_id: MessageId) { ... }
```

需要添加的 import（参照原 main.rs 中的使用）：
```rust
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tgbot::logic::config::{ConfigManager, Proto};
use tgbot::logic::system::monitor::SystemMonitor;
use crate::handlers::system::escape_html;
```

- [ ] **Step 2: 更新 main.rs 引用**

从 main.rs 中删除这三个函数的定义，并在 callback.rs（后续 Task 6 中将搬迁到此）需要引用它们的地方添加：
```rust
use crate::handlers::proxy::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};
```

目前这些函数仍在 main.rs 中被 callback 部分（尚未搬迁）使用。暂时在 main.rs 中添加 import：
```rust
use crate::handlers::proxy::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};
```

- [ ] **Step 3: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功

- [ ] **Step 4: 提交**

```bash
git add src/main.rs src/handlers/proxy.rs
git commit -m "refactor: extract proxy helper functions to handlers/proxy.rs"
```

---

## Task 5: 迁移 command handlers 到 handlers/command.rs

**Files:**
- Modify: `src/main.rs` (删除 command 相关代码)
- Modify: `src/handlers/command.rs` (填充)

这是最大的一步非 callback 搬迁。

- [ ] **Step 1: 将 Command 枚举和命令处理函数移入 handlers/command.rs**

从 main.rs 剪切以下项目：

1. `const MAX_FILE_DOWNLOAD_SIZE: u64 = 10 * 1024 * 1024;` （行 111）
2. `const MAX_INPUT_LENGTH: usize = 4096;` （行 438）
3. `async fn register_bot_commands(bot: &Bot) -> Result<()>` （行 113-119）
4. `fn looks_like_totp_code(text: &str) -> bool` （行 291-293）
5. `async fn process_auth_code(...)` （行 295-313）
6. `async fn handle_command(...)` （行 315-436）
7. `async fn handle_message(...)` （行 440-541）
8. `async fn send_main_menu(...)` （行 543-560）

粘贴到 `src/handlers/command.rs`，全部改为 `pub(crate)`。

需要的 import（参照 main.rs 中的使用，需要重新整理）：
```rust
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use sha2::{Digest, Sha256};
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use teloxide::utils::command::BotCommands;
use teloxide::net::Download;
use crate::app::auth;
use crate::app::destruct_flow::{self, MessageFlowOutcome};
use crate::app::state::AppState;
use crate::bootstrap;
use crate::handlers::system::format_duration_human;
```

Command 枚举的 process_auth_code 需要 TOTP 常量。这两种常量目前在 main.rs 中：
```rust
const TOTP_FAIL_MAX: u32 = 5;
const TOTP_FAIL_WINDOW: Duration = Duration::from_secs(10 * 60);
const LOCKOUT_DURATIONS: [Duration; 4] = [...];
```

**方案：** 将 TOTP 常量移入 `handlers/command.rs`（因为它们只被 process_auth_code 使用）。添加：
```rust
const TOTP_FAIL_MAX: u32 = 5;
const TOTP_FAIL_WINDOW: Duration = Duration::from_secs(10 * 60);
const LOCKOUT_DURATIONS: [Duration; 4] = [
    Duration::from_secs(15 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(24 * 60 * 60),
    Duration::from_secs(48 * 60 * 60),
];
```

- [ ] **Step 2: 更新 main.rs**

从 main.rs 中删除以上所有代码和它们的专有 import。

main.rs 中的 `main()` 函数需要引用 Command 和 Dispatcher：
```rust
use crate::handlers::command::{Command, handle_command, handle_message};
```

Dispatcher 构建改为：
```rust
let handler = dptree::entry()
    .branch(
        Update::filter_message()
            .filter_command::<Command>()
            .endpoint(handle_command),
    )
    .branch(Update::filter_message().endpoint(handle_message))
    .branch(Update::filter_callback_query().endpoint(handle_callback));
```

注意：`handle_callback` 仍在 main.rs 中，需要保留 `fn handle_callback` 的定义或引入。在本阶段，handle_callback 仍在 main.rs 中。

- [ ] **Step 3: 更新 handlers/mod.rs 的 re-export**

`src/handlers/mod.rs` 更新为：
```rust
pub mod callback;
pub mod command;
pub mod proxy;
pub mod security;
pub mod system;

pub use command::{handle_command, handle_message, Command};
```

- [ ] **Step 4: 更新 main.rs 中的 use 语句**

main.rs 中原本通过 `super::*` 引用这些函数，现在改为通过 `crate::handlers` 引用：
```rust
use crate::handlers::command::{Command, register_bot_commands};
```

- [ ] **Step 5: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功

- [ ] **Step 6: 验证测试**

Run: `cd rust/tgbot && cargo test`
Expected: 全部测试通过

- [ ] **Step 7: 提交**

```bash
git add src/main.rs src/handlers/command.rs src/handlers/mod.rs
git commit -m "refactor: extract command handlers to handlers/command.rs"
```

---

## Task 6: 迁移 handle_callback 到 handlers/callback.rs（核心步骤）

这是最大的一步。handle_callback 函数约 4,163 行。

**Files:**
- Modify: `src/main.rs` (删除 handle_callback 及其所有相关代码)
- Modify: `src/handlers/callback.rs` (填充整个 handle_callback)

- [ ] **Step 1: 将 handle_callback 整体搬入 handlers/callback.rs**

从 main.rs 剪切 `fn handle_callback(...)` 整个函数（约行 756-4919），粘贴到 `src/handlers/callback.rs`。

将可见性改为 `pub(crate)`：
```rust
pub(crate) fn handle_callback(
    bot: Bot,
    mut q: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
```

添加所有必要的 import。这个函数引用了大量类型和模块，需要仔细整理。主要 import 包括：
```rust
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode};
use teloxide::net::Download;
use tgbot::logic::config::{ConfigManager, KcpMask, Proto, WarpMode};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::system::maintenance::BBR3_PENDING_FLAG_FILE;
use tgbot::logic::system::monitor::SystemMonitor;
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::task_types::TaskType;
use tgbot::logic::security::SecurityManager;
use tgbot::logic::self_destruct::production_executor;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::system::SystemMonitor as SystemMonitorAlias;
use tgbot::logic::upgrade::{UPGRADE_FLAG_FILE, UpgradeManager, wwps_core::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager}};
use tgbot::logic::log_audit::{LogAudit, SERVICE_WWPS_CORE, SERVICE_SING_BOX};
use crate::app::destruct_flow::{self, MessageFlowOutcome};
use crate::app::state::{AppState, ScheduleFrequency, ScheduleInputState, TimeoutStatus};
use crate::handlers::proxy::{show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init};
use crate::handlers::system::{
    escape_html, format_duration_human, save_config, validate_hash_prefix, validate_idx,
    schedule_task_name, schedule_frequency_name, weekday_label, timezone_label,
    build_custom_schedule_text, build_custom_schedule_keyboard,
    build_cron_from_custom_state,
};
use obfstr::obfstr;
```

注意：具体 import 路径需要根据编译错误逐步调整。以上是初始参考。

- [ ] **Step 2: 从 main.rs 删除已迁移的代码**

从 main.rs 中删除整个 `handle_callback` 函数定义。

添加 import：
```rust
use crate::handlers::callback::handle_callback;
```

- [ ] **Step 3: 验证编译**

Run: `cd rust/tgbot && cargo check`
Expected: 可能有多处 import 错误，需要逐步修复

- [ ] **Step 4: 修复编译错误**

根据 `cargo check` 输出，逐步修复：
1. 缺失的 import 语句
2. 可见性错误（添加 `pub(crate)`）
3. 路径引用错误

每修复一批错误后运行 `cargo check` 验证。

- [ ] **Step 5: 验证测试**

Run: `cd rust/tgbot && cargo test`
Expected: 全部测试通过

- [ ] **Step 6: 提交**

```bash
git add src/main.rs src/handlers/callback.rs
git commit -m "refactor: move handle_callback to handlers/callback.rs"
```

---

## Task 7: 清理 main.rs 和最终瘦身

**Files:**
- Modify: `src/main.rs` (最终清理)
- Modify: `src/handlers/mod.rs` (最终 re-export)
- Modify: `src/handlers/security.rs` (确认是否为空)

- [ ] **Step 1: 清理 main.rs 中的无用 import**

检查 main.rs，删除所有不再需要的 use 语句。main.rs 最终应只保留：
- `mod app;`, `mod bootstrap;`, `mod handlers;`
- `use crate::handlers::callback::handle_callback;`
- `use crate::handlers::command::{Command, handle_command, handle_message, register_bot_commands};`
- `use crate::app::state::AppState;`
- main() 函数需要的 `use teloxide::prelude::*;`, `use std::sync::Arc;`, `use anyhow::Result;` 等
- `#[cfg(test)] mod tests` 及其相关 use

- [ ] **Step 2: 确认 handlers/mod.rs 的 re-export 列表**

最终 `src/handlers/mod.rs`：
```rust
pub mod callback;
pub mod command;
pub mod proxy;
pub mod security;
pub mod system;

pub use command::{handle_command, handle_message, Command};
pub use callback::handle_callback;
```

- [ ] **Step 3: 检查 handlers/security.rs**

如果 `validate_hash_prefix` 和 `validate_idx` 已在 Task 3 中移入 `handlers/system.rs`，则 `handlers/security.rs` 可能为空。确认这一点。如果为空，可以保留一个空模块，或删除它。建议保留并在其中添加注释说明未来安全相关工具函数可放此处。

- [ ] **Step 4: 更新 handlers/mod.rs**

如果 security.rs 为空文件（只有注释），确保 mod.rs 仍然声明它：
```rust
pub mod security;  // 安全相关工具函数的归属（validate_hash_prefix, validate_idx 等已移至 system.rs）
```

- [ ] **Step 5: 验证最终编译**

Run: `cd rust/tgbot && cargo check`
Expected: 编译成功，0 错误

- [ ] **Step 6: 验证最终测试**

Run: `cd rust/tgbot && cargo test`
Expected: 全部测试通过

- [ ] **Step 7: 检查 main.rs 行数**

Run: `wc -l rust/tgbot/src/main.rs`
Expected: < 200 行

- [ ] **Step 8: 检查所有新文件行数**

Run: `wc -l rust/tgbot/src/handlers/*.rs rust/tgbot/src/app/lifecycle.rs`
Expected: 无任何文件超过 1,500 行

- [ ] **Step 9: 最终提交**

```bash
git add -A
git commit -m "refactor: complete main.rs decomposition - slim main.rs to ~150 lines"
```

---

## Verification Checklist

- [ ] `cargo check` 无错误
- [ ] `cargo test` 全部通过
- [ ] `main.rs` < 200 行
- [ ] 无任何文件超过 1,500 行
- [ ] `handlers/callback.rs` 中的 handle_callback 保持原有逻辑不变
- [ ] 所有 TOTP 认证逻辑正常工作
- [ ] 所有 inline keyboard 回调路由正常工作
