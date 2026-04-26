# tgbot 分层架构重构实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 tgbot 重构为三层架构（handlers/services/logic），将 main.rs 从 4500+ 行缩减到 <200 行

**Architecture:** 建立 handlers（UI层）、services（业务层）、logic（实现层）三层架构，保持现有 logic/ 模块不变，仅重新组织调用方式

**Tech Stack:** Rust 2024, teloxide (Telegram Bot), tokio, anyhow, serde

**Design Doc:** `.opencode/plans/2025-04-26-tgbot-refactor-design.md`

---

## Phase 1: 基础设施

### Task 1: 创建目录结构

**Files:**
- Create: `rust/tgbot/src/handlers/mod.rs`
- Create: `rust/tgbot/src/handlers/commands.rs`
- Create: `rust/tgbot/src/handlers/messages.rs`
- Create: `rust/tgbot/src/handlers/callbacks/mod.rs`
- Create: `rust/tgbot/src/services/mod.rs`
- Modify: `rust/tgbot/src/lib.rs`

- [ ] **Step 1: 创建 handlers 目录结构**

```bash
mkdir -p rust/tgbot/src/handlers/callbacks
mkdir -p rust/tgbot/src/services
```

- [ ] **Step 2: 创建 handlers/mod.rs 骨架**

```rust
//! Handlers 模块 - UI 层
//!
//! 处理所有 Telegram 交互，包括命令、回调和消息

pub mod commands;
pub mod messages;
pub mod callbacks;

use teloxide::prelude::*;
use crate::app::state::AppState;
use std::sync::Arc;

/// Handler 结果类型
pub type HandlerResult = ResponseResult<()>;
```

- [ ] **Step 3: 创建 handlers/commands.rs 骨架**

```rust
//! 命令处理模块
//!
//! 处理所有 Telegram Bot 命令：/start, /auth, /menu, /setsecurityfile

use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

use crate::app::state::AppState;
use std::sync::Arc;

/// Bot 命令定义
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "支持以下命令:")]
pub enum Command {
    #[command(description = "显示帮助信息")]
    Help,
    #[command(description = "启动机器人")]
    Start,
    #[command(description = "显示管理菜单")]
    Menu,
    #[command(description = "验证 TOTP 认证码")]
    Auth(String),
    #[command(description = "设置自毁验证文件 (需附带文件)")]
    SetSecurityFile,
}

/// 处理命令
pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    // 临时转发到 main.rs 的原有实现
    // 后续任务会迁移实际逻辑
    Ok(())
}
```

- [ ] **Step 4: 创建 handlers/messages.rs 骨架**

```rust
//! 消息处理模块
//!
//! 处理普通文本消息、TOTP 验证、WARP 输入等

use teloxide::prelude::*;
use crate::app::state::AppState;
use std::sync::Arc;

const MAX_INPUT_LENGTH: usize = 4096;

/// 处理普通消息
pub async fn handle_message(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    // 临时转发到 main.rs 的原有实现
    Ok(())
}
```

- [ ] **Step 5: 创建 handlers/callbacks/mod.rs 骨架**

```rust
//! 回调处理模块
//!
//! 处理所有内联键盘按钮回调

use teloxide::prelude::*;
use crate::app::state::AppState;
use std::sync::Arc;

/// 回调路由入口
pub async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    // 临时转发到 main.rs 的原有实现
    Ok(())
}
```

- [ ] **Step 6: 创建 services/mod.rs 骨架**

```rust
//! Services 模块 - 业务逻辑层
//!
//! 提供业务逻辑编排，协调多个 logic/ 模块的操作

use thiserror::Error;

/// Service 层错误类型
#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("配置生成失败: {0}")]
    ConfigGeneration(String),
    
    #[error("系统操作失败: {0}")]
    SystemOperation(String),
    
    #[error("验证失败: {0}")]
    Validation(String),
    
    #[error("资源未找到: {0}")]
    NotFound(String),
    
    #[error("权限不足")]
    Unauthorized,
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;
```

- [ ] **Step 7: 更新 lib.rs 导出新模块**

```rust
// 在 rust/tgbot/src/lib.rs 中添加
pub mod handlers;
pub mod services;
```

- [ ] **Step 8: 验证编译**

```bash
cd rust/tgbot
cargo check
```

Expected: 编译成功，可能有未使用警告

- [ ] **Step 9: 提交**

```bash
git add rust/tgbot/src/handlers/ rust/tgbot/src/services/ rust/tgbot/src/lib.rs
git commit -m "chore: create handlers and services directory structure

- Add handlers/ for UI layer (commands, messages, callbacks)
- Add services/ for business logic layer
- Create module skeletons with proper documentation"
```

---

### Task 2: 迁移 commands.rs 基础实现

**Files:**
- Modify: `rust/tgbot/src/handlers/commands.rs`
- Modify: `rust/tgbot/src/main.rs`

- [ ] **Step 1: 复制现有 Command 枚举到 commands.rs**

从 main.rs 复制以下到 commands.rs：
- `Command` 枚举定义
- `looks_like_totp_code` 函数
- `validate_hash_prefix` 函数
- `validate_idx` 函数
- `format_duration_human` 函数
- `escape_html` 函数

- [ ] **Step 2: 复制 handle_command 函数框架**

从 main.rs 复制 `handle_command` 函数到 commands.rs，但暂时保留所有内容

- [ ] **Step 3: 修改 main.rs 使用新的 commands 模块**

```rust
// 在 main.rs 中
use tgbot::handlers::commands::{Command, handle_command};

// 删除原有的 Command 枚举和 handle_command 函数定义
// 保留函数体内容作为参考，稍后删除
```

- [ ] **Step 4: 验证编译**

```bash
cd rust/tgbot
cargo check
```

Expected: 编译成功

- [ ] **Step 5: 提交**

```bash
git add rust/tgbot/src/handlers/commands.rs rust/tgbot/src/main.rs
git commit -m "refactor: migrate Command enum and helpers to handlers/commands.rs

- Move Command enum from main.rs
- Move helper functions (looks_like_totp_code, validate_hash_prefix, etc.)
- Update main.rs to use new module"
```

---

### Task 3: 提取 commands.rs 完整实现

**Files:**
- Modify: `rust/tgbot/src/handlers/commands.rs`

- [ ] **Step 1: 添加必要的 imports**

```rust
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use teloxide::net::Download;
use teloxide::types::{ParseMode, InputFile};

use crate::app::auth;
use crate::app::state::{AppState, TimeoutStatus};
use crate::bootstrap::save_config;
use crate::logic::config::{ConfigManager, WarpMode};
use crate::logic::installer::{RealityInstaller, RealityInstallOutcome, WarpInstaller};
use crate::logic::totp::TotpManager;
use tgbot::core::paths::{singbox, xray};
```

- [ ] **Step 2: 提取 Help 命令处理**

```rust
Command::Help => {
    bot.send_message(msg.chat.id, Command::descriptions().to_string())
        .await?;
}
```

- [ ] **Step 3: 提取 Start 命令处理**

```rust
Command::Start => {
    bot.send_message(
        msg.chat.id,
        "👋 欢迎使用 wwps 管理机器人！\n\n请发送 6 位 TOTP 验证码（或使用 /auth <验证码>）解锁 24 小时管理权限。",
    )
    .await?;
}
```

- [ ] **Step 4: 提取 Auth 命令处理**

```rust
Command::Auth(code) => {
    let _ = process_auth_code(&bot, msg.chat.id, user_id, &code, &state).await?;
}
```

- [ ] **Step 5: 提取 SetSecurityFile 命令处理**

从 main.rs 复制完整的 SetSecurityFile 处理逻辑（约 60 行）

- [ ] **Step 6: 提取 Menu 命令处理**

```rust
Command::Menu => {
    if !state.is_authorized(user_id).await {
        bot.send_message(
            msg.chat.id,
            "🔐 请先发送 6 位 TOTP 验证码进行认证（或 /auth <验证码>）。",
        )
        .await?;
        return Ok(());
    }
    // 调用发送主菜单的函数（稍后在 callbacks 中实现）
    crate::handlers::callbacks::send_main_menu(bot, msg.chat.id).await?;
}
```

- [ ] **Step 7: 添加 process_auth_code 辅助函数**

```rust
async fn process_auth_code(
    bot: &Bot,
    chat_id: ChatId,
    user_id: i64,
    code: &str,
    state: &Arc<AppState>,
) -> ResponseResult<bool> {
    use crate::app::auth;
    
    const TOTP_FAIL_MAX: u32 = 5;
    const TOTP_FAIL_WINDOW: Duration = Duration::from_secs(10 * 60);
    const LOCKOUT_DURATIONS: [Duration; 4] = [
        Duration::from_secs(15 * 60),
        Duration::from_secs(60 * 60),
        Duration::from_secs(24 * 60 * 60),
        Duration::from_secs(48 * 60 * 60),
    ];
    
    auth::process_auth_code(
        bot,
        chat_id,
        user_id,
        code,
        state,
        TOTP_FAIL_MAX,
        TOTP_FAIL_WINDOW,
        &LOCKOUT_DURATIONS,
    )
    .await
}
```

- [ ] **Step 8: 验证编译**

```bash
cd rust/tgbot
cargo check
```

Expected: 编译成功

- [ ] **Step 9: 提交**

```bash
git add rust/tgbot/src/handlers/commands.rs
git commit -m "refactor: complete commands.rs implementation

- Extract all command handlers from main.rs
- Add proper imports and helper functions
- Maintain existing logic and behavior"
```

---

## Phase 2: 回调处理

### Task 4: 创建回调路由系统

**Files:**
- Modify: `rust/tgbot/src/handlers/callbacks/mod.rs`
- Create: `rust/tgbot/src/handlers/callbacks/main_menu.rs`

- [ ] **Step 1: 设计回调路由**

回调数据模式分析：
- `m_*` - 菜单导航 (m_main, m_mon, m_usr, m_ops_center, m_settings)
- `a_*` - 动作 (a_geo, a_bbr3, a_fw, a_reload)
- `u_*` - 用户管理 (u_batch_init, u_l:*)
- `sb_*` - Sing-box 管理 (sb_install, sb_h2_init)
- `s_*` - 调度任务 (s_add_custom_menu, s_custom_ui:*)
- `l_*` - 日志管理 (l_tgl, l_tail_acc)

- [ ] **Step 2: 创建回调分发器**

```rust
// rust/tgbot/src/handlers/callbacks/mod.rs
use futures_util::future::BoxFuture;
use teloxide::prelude::*;
use crate::app::state::AppState;
use std::sync::Arc;

pub mod main_menu;

pub type CallbackHandler = fn(Bot, CallbackQuery, Arc<AppState>) -> BoxFuture<'static, ResponseResult<()>>;

/// 回调路由入口
pub fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    state: Arc<AppState>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        let data = match query.data.as_ref() {
            Some(d) => d.as_str(),
            None => return Ok(()),
        };
        
        match data {
            // 主菜单
            "m_main" | "m_mon" | "m_usr" | "m_ops_center" | "m_settings" => {
                main_menu::handle(bot, query, state, data).await
            }
            _ => {
                // 未处理的回调
                Ok(())
            }
        }
    })
}
```

- [ ] **Step 3: 创建 main_menu.rs**

```rust
//! 主菜单回调处理

use futures_util::future::BoxFuture;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use crate::app::state::AppState;
use std::sync::Arc;

/// 处理主菜单回调
pub fn handle(
    bot: Bot,
    query: CallbackQuery,
    state: Arc<AppState>,
    action: &str,
) -> BoxFuture<'static, ResponseResult<()>> {
    let action = action.to_string();
    Box::pin(async move {
        let chat_id = query.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
        let msg_id = query.message.as_ref().map(|m| m.id()).unwrap_or_default();
        
        match action.as_str() {
            "m_main" => send_main_menu(bot, chat_id, msg_id).await,
            "m_mon" => show_monitor_menu(bot, chat_id, msg_id).await,
            "m_usr" => show_user_menu(bot, chat_id, msg_id).await,
            "m_ops_center" => show_ops_center(bot, chat_id, msg_id).await,
            "m_settings" => show_settings(bot, chat_id, msg_id, &state).await,
            _ => Ok(()),
        }
    })
}

/// 发送主菜单
pub async fn send_main_menu(bot: Bot, chat_id: ChatId, msg_id: MessageId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("📊 系统状态", "m_mon"),
            InlineKeyboardButton::callback("👥 用户管理", "m_usr"),
        ],
        vec![InlineKeyboardButton::callback("🛠 运维中心 (Ops)", "m_ops_center")],
        vec![InlineKeyboardButton::callback("⚙️ 系统设置", "m_settings")],
    ]);
    
    bot.edit_message_text(chat_id, msg_id, "🏠 <b>主菜单</b>\n请选择操作类目:")
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

/// 显示监控菜单
async fn show_monitor_menu(bot: Bot, chat_id: ChatId, msg_id: MessageId) -> ResponseResult<()> {
    use crate::logic::system::SystemMonitor;
    use crate::bootstrap::BOT_VERSION;
    
    let report = SystemMonitor::get_status_report()
        .await
        .unwrap_or_else(|e| format!("❌ 获取状态失败: {}", e));
    let (wwps_core, wwps_box) = SystemMonitor::get_core_status().await;
    
    let status_text = format!(
        "{}\n\n🤖 <b>Bot 版本</b>: v{}\n\n⚙️ <b>核心进程</b>:\n- Xray-core: {}\n- Sing-box: {}",
        report,
        BOT_VERSION,
        if wwps_core { "🟢" } else { "🔴" },
        if wwps_box { "🟢" } else { "🔴" }
    );
    
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🔄 刷新", "m_mon")],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_main")],
    ]);
    
    bot.edit_message_text(chat_id, msg_id, status_text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

// 其他菜单函数占位，后续任务实现
async fn show_user_menu(bot: Bot, chat_id: ChatId, msg_id: MessageId) -> ResponseResult<()> {
    Ok(())
}

async fn show_ops_center(bot: Bot, chat_id: ChatId, msg_id: MessageId) -> ResponseResult<()> {
    Ok(())
}

async fn show_settings(bot: Bot, chat_id: ChatId, msg_id: MessageId, state: &Arc<AppState>) -> ResponseResult<()> {
    Ok(())
}
```

- [ ] **Step 4: 更新 main.rs 使用新的回调系统**

```rust
// 在 main.rs 的 Update::filter_callback_query 分支
Update::filter_callback_query().endpoint(
    move |bot: Bot, q: CallbackQuery, state: Arc<AppState>| async move {
        handlers::callbacks::handle_callback(bot, q, state).await
    }
),
```

- [ ] **Step 5: 验证编译**

```bash
cd rust/tgbot
cargo check
```

Expected: 编译成功

- [ ] **Step 6: 提交**

```bash
git add rust/tgbot/src/handlers/callbacks/
git commit -m "feat: create callback routing system with main_menu handler

- Add callback dispatcher in callbacks/mod.rs
- Extract main_menu handlers (m_main, m_mon)
- Move send_main_menu function from main.rs"
```

---

### Task 5: 提取用户管理回调 (callbacks/user_mgmt.rs)

**Files:**
- Create: `rust/tgbot/src/handlers/callbacks/user_mgmt.rs`
- Modify: `rust/tgbot/src/handlers/callbacks/mod.rs`

- [ ] **Step 1: 创建 user_mgmt.rs 骨架**

```rust
//! 用户管理回调处理

use futures_util::future::BoxFuture;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, MessageId};
use crate::app::state::AppState;
use crate::core::types::IpVersion;
use crate::logic::config::{ConfigManager, RealityProto};
use crate::logic::installer::RealityInstaller;
use crate::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use std::path::Path;
use std::sync::Arc;

pub fn handle(
    bot: Bot,
    query: CallbackQuery,
    state: Arc<AppState>,
    action: &str,
) -> BoxFuture<'static, ResponseResult<()>> {
    let action = action.to_string();
    Box::pin(async move {
        let chat_id = query.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));
        let msg_id = query.message.as_ref().map(|m| m.id()).unwrap_or_default();
        
        if action == "m_usr" {
            return show_user_menu(bot, chat_id, msg_id).await;
        }
        
        if action.starts_with("m_xray") || action.starts_with("u_") {
            return handle_xray_callbacks(bot, query, chat_id, msg_id, &action).await;
        }
        
        if action.starts_with("m_singbox") || action.starts_with("sb_") {
            return handle_singbox_callbacks(bot, query, chat_id, msg_id, &action).await;
        }
        
        Ok(())
    })
}
```

- [ ] **Step 2: 提取用户主菜单逻辑**

从 main.rs 的 `"m_usr"` 分支提取完整的逻辑（约 50 行）

- [ ] **Step 3: 提取 Xray 管理菜单**

从 main.rs 的 `"m_xray_mgmt"` 分支提取逻辑（约 60 行）

- [ ] **Step 4: 提取批量配置回调**

从 main.rs 提取：
- `u_batch_init`
- `u_xhttp_batch_init`
- `u_xdns_init`
- `u_batch_exec:*` 模式

- [ ] **Step 5: 提取 Sing-box 回调**

从 main.rs 提取：
- `m_singbox_mgmt`
- `sb_install`
- `sb_h2_init`, `sb_h2_ip:*`, `sb_h2_exec:*`
- `sb_tu_init`, `sb_tu_ip:*`, `sb_tu_exec:*`
- `sb_del_cfg` 及其子回调

- [ ] **Step 6: 更新 callbacks/mod.rs 添加路由**

```rust
pub mod user_mgmt;

// 在 match data 中添加
"m_usr" | "m_xray_mgmt" | "m_singbox_mgmt" => {
    user_mgmt::handle(bot, query, state, data).await
}
```

- [ ] **Step 7: 验证编译**

```bash
cd rust/tgbot
cargo check
```

- [ ] **Step 8: 提交**

```bash
git add rust/tgbot/src/handlers/callbacks/user_mgmt.rs rust/tgbot/src/handlers/callbacks/mod.rs
git commit -m "refactor: extract user management callbacks to user_mgmt.rs

- Move Xray-core management callbacks
- Move Sing-box management callbacks
- Move batch config creation handlers"
```

---

### Task 6: 提取运维中心回调 (callbacks/ops_center.rs)

**Files:**
- Create: `rust/tgbot/src/handlers/callbacks/ops_center.rs`
- Modify: `rust/tgbot/src/handlers/callbacks/mod.rs`

- [ ] **Step 1: 创建 ops_center.rs**

提取以下回调：
- `m_ops_center` - 运维中心主菜单
- `m_net_opt` - 网络优化
- `m_security` - 安全防护
- `m_sys_cmd` - 系统指令
- `m_log` - 日志管理
- `m_warp` - WARP 管理
- 所有 `a_*` 动作回调

- [ ] **Step 2: 提取 WARP 相关回调**

从 main.rs 提取完整的 WARP 管理逻辑：
- `m_warp`
- `a_warp_add_input`, `a_warp_del_menu`, `a_warp_del:*`
- `a_warp_switch_mode`, `a_warp_status`
- `a_warp_clear_confirm`, `a_warp_clear_exec`
- `a_warp_restart`, `a_warp_uninstall`

- [ ] **Step 3: 提取系统操作回调**

- `a_bbr3` - BBR3 安装（包含进度回调）
- `a_fw` - 防火墙加固
- `a_sys_reboot` - 系统重启
- `a_reload` - 重启核心
- `a_sys_maint` - 系统维护
- `a_inst_warp` - 安装 WARP
- `a_inst_base` - 初始化环境

- [ ] **Step 4: 提取日志管理回调**

- `l_tgl` - 切换日志
- `l_tail_acc` - 查看 Access 日志
- `l_tail_err` - 查看 Error 日志

- [ ] **Step 5: 更新 mod.rs 添加路由**

- [ ] **Step 6: 验证编译并提交**

```bash
git add rust/tgbot/src/handlers/callbacks/ops_center.rs rust/tgbot/src/handlers/callbacks/mod.rs
git commit -m "refactor: extract ops center callbacks to ops_center.rs

- Move network optimization handlers
- Move security and system command handlers
- Move WARP management callbacks
- Move log management callbacks"
```

---

### Task 7: 提取设置回调 (callbacks/settings.rs)

**Files:**
- Create: `rust/tgbot/src/handlers/callbacks/settings.rs`
- Modify: `rust/tgbot/src/handlers/callbacks/mod.rs`

- [ ] **Step 1: 创建 settings.rs**

提取以下回调：
- `m_settings` - 设置主菜单
- `m_session_timeout` - 会话超时设置
- `set_timeout:*` - 设置具体超时时间
- `m_sched` - 定时任务管理
- `s_add_custom_menu` - 添加自定义任务
- `s_custom_ui:*`, `s_custom_set:*`, `s_custom_confirm`, `s_custom_cancel`
- `a_wwps_core_menu` - Xray-core 管理
- `a_wwps_box_menu` - Sing-box 管理
- `a_geo_menu`, `a_geo`, `a_geo_sched_menu` - Geo 数据管理
- `a_upgrade` - Bot 更新
- `m_danger` - 危险区域菜单

- [ ] **Step 2: 提取定时任务回调**

从 main.rs 提取完整的定时任务逻辑：
- 自定义任务创建流程
- 星期、小时、分钟选择
- 时区选择
- 任务确认和取消

- [ ] **Step 3: 提取核心管理回调**

- Xray-core 版本管理
- Sing-box 版本管理
- Geo 数据更新

- [ ] **Step 4: 更新 mod.rs 添加路由**

- [ ] **Step 5: 验证编译并提交**

```bash
git add rust/tgbot/src/handlers/callbacks/settings.rs rust/tgbot/src/handlers/callbacks/mod.rs
git commit -m "refactor: extract settings callbacks to settings.rs

- Move settings menu handlers
- Move scheduler/cron task handlers
- Move core management (Xray/Sing-box) handlers
- Move Geo data and upgrade handlers"
```

---

### Task 8: 提取自毁流程回调 (callbacks/destruct.rs)

**Files:**
- Create: `rust/tgbot/src/handlers/callbacks/destruct.rs`
- Modify: `rust/tgbot/src/handlers/callbacks/mod.rs`

- [ ] **Step 1: 创建 destruct.rs**

从 `app/destruct_flow.rs` 和 main.rs 提取：
- `a_destroy_ask` - 发起自毁确认
- `a_destroy_confirm` - 确认自毁
- `a_destroy_code:*` - 输入销毁码
- `a_destroy_cancel` - 取消自毁
- 所有与自毁相关的辅助函数

- [ ] **Step 2: 确保自毁逻辑完整性**

保持现有的安全检查和执行流程不变

- [ ] **Step 3: 更新 mod.rs 添加路由**

- [ ] **Step 4: 验证编译并提交**

```bash
git add rust/tgbot/src/handlers/callbacks/destruct.rs rust/tgbot/src/handlers/callbacks/mod.rs
git commit -m "refactor: extract self-destruct flow to destruct.rs

- Move all self-destruct callbacks
- Maintain existing safety checks and execution flow"
```

---

### Task 9: 提取消息处理 (messages.rs)

**Files:**
- Modify: `rust/tgbot/src/handlers/messages.rs`
- Modify: `rust/tgbot/src/main.rs`

- [ ] **Step 1: 添加完整的消息处理逻辑**

从 main.rs 的 `handle_message` 函数提取：
- 输入长度验证
- 调度任务超时检查
- WARP 输入处理
- 自毁流程消息处理
- TOTP 验证码处理

- [ ] **Step 2: 更新 main.rs 使用新的 messages 模块**

```rust
// 在 Update::filter_message() 分支
Update::filter_message().endpoint(
    move |bot: Bot, msg: Message, state: Arc<AppState>| async move {
        handlers::messages::handle_message(bot, msg, state).await
    }
),
```

- [ ] **Step 3: 验证编译并提交**

```bash
git add rust/tgbot/src/handlers/messages.rs rust/tgbot/src/main.rs
git commit -m "refactor: extract message handling to messages.rs

- Move TOTP verification logic
- Move WARP input handling
- Move schedule task input handling
- Move self-destruct message flow"
```

---

## Phase 3: Service 层

### Task 10: 创建 config_service.rs

**Files:**
- Create: `rust/tgbot/src/services/config_service.rs`
- Modify: `rust/tgbot/src/services/mod.rs`

- [ ] **Step 1: 创建 config_service.rs**

```rust
//! 配置生成服务

use crate::core::types::{BatchCreationResult, IpVersion};
use crate::logic::config::{ConfigManager, RealityProto};
use crate::logic::installer::RealityInstaller;
use crate::logic::singbox::SingBoxConfigManager;
use crate::services::{ServiceError, ServiceResult};

pub struct ConfigService;

impl ConfigService {
    pub fn new() -> Self {
        Self
    }
    
    /// 批量创建 Reality 配置
    pub async fn create_batch_reality(
        &self,
        ip_version: IpVersion,
        count: usize,
        proto: RealityProto,
    ) -> ServiceResult<BatchCreationResult> {
        // 1. 确保 Reality 环境就绪
        RealityInstaller::ensure_ready().await
            .map_err(|e| ServiceError::ConfigGeneration(format!("环境初始化失败: {}", e)))?;
        
        // 2. 生成配置
        let result = match proto {
            RealityProto::Vision => {
                ConfigManager::generate_reality_batch(ip_version, count).await
            }
            RealityProto::XHTTP => {
                ConfigManager::generate_xhttp_batch(ip_version, count).await
            }
            RealityProto::XdnsMkcp => {
                ConfigManager::generate_xdns_batch(ip_version, count).await
            }
        };
        
        result.map_err(|e| ServiceError::ConfigGeneration(e.to_string()))
    }
    
    /// 批量创建 Hysteria2 配置
    pub async fn create_batch_hysteria2(
        &self,
        ip_version: IpVersion,
        count: usize,
        obfs_enabled: bool,
    ) -> ServiceResult<BatchCreationResult> {
        SingBoxConfigManager::batch_create_hysteria2(count, ip_version, obfs_enabled)
            .await
            .map_err(|e| ServiceError::ConfigGeneration(e.to_string()))
    }
    
    /// 批量创建 TUIC 配置
    pub async fn create_batch_tuic(
        &self,
        ip_version: IpVersion,
        count: usize,
    ) -> ServiceResult<BatchCreationResult> {
        SingBoxConfigManager::batch_create_tuic(count, ip_version)
            .await
            .map_err(|e| ServiceError::ConfigGeneration(e.to_string()))
    }
}

impl Default for ConfigService {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 更新 services/mod.rs 导出**

```rust
pub mod config_service;
pub use config_service::ConfigService;
```

- [ ] **Step 3: 验证编译**

- [ ] **Step 4: 提交**

```bash
git add rust/tgbot/src/services/config_service.rs rust/tgbot/src/services/mod.rs
git commit -m "feat: create ConfigService for batch config generation

- Add create_batch_reality for Vision/XHTTP/XDNS
- Add create_batch_hysteria2 and create_batch_tuic
- Centralize config generation logic"
```

---

### Task 11: 创建 system_service.rs

**Files:**
- Create: `rust/tgbot/src/services/system_service.rs`
- Modify: `rust/tgbot/src/services/mod.rs`

- [ ] **Step 1: 创建 system_service.rs**

```rust
//! 系统操作服务

use crate::logic::maintenance::MaintenanceManager;
use crate::logic::operations::Operations;
use crate::logic::system::{StatusReport, SystemMonitor};
use crate::services::{ServiceError, ServiceResult};

pub struct SystemService;

impl SystemService {
    pub fn new() -> Self {
        Self
    }
    
    /// 获取系统状态报告
    pub async fn get_status(&self) -> ServiceResult<StatusReport> {
        SystemMonitor::get_status_report()
            .await
            .map_err(|e| ServiceError::SystemOperation(e.to_string()))
    }
    
    /// 执行系统维护
    pub async fn perform_maintenance(&self) -> ServiceResult<String> {
        Operations::perform_maintenance()
            .await
            .map_err(|e| ServiceError::SystemOperation(e.to_string()))
    }
    
    /// 重启系统
    pub async fn reboot(&self) -> ServiceResult<()> {
        Operations::reboot_system()
            .await
            .map_err(|e| ServiceError::SystemOperation(e.to_string()))
    }
    
    /// 安装 BBR3
    pub async fn install_bbr3<F>(&self, progress: F) -> ServiceResult<()>
    where
        F: Fn(u8, &str) + Send + 'static,
    {
        MaintenanceManager::install_bbr3_with_progress(progress)
            .await
            .map_err(|e| ServiceError::SystemOperation(e.to_string()))?
            .map_err(|e| ServiceError::SystemOperation(e.to_string()))
    }
}

impl Default for SystemService {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 更新 services/mod.rs**

- [ ] **Step 3: 验证编译并提交**

```bash
git add rust/tgbot/src/services/system_service.rs
git commit -m "feat: create SystemService for system operations

- Add get_status, perform_maintenance, reboot methods
- Add install_bbr3 with progress callback"
```

---

### Task 12: 重构 main.rs

**Files:**
- Modify: `rust/tgbot/src/main.rs`

- [ ] **Step 1: 清理已提取的代码**

删除以下已迁移到 handlers/ 的内容：
- `handle_command` 函数
- `handle_message` 函数  
- `handle_callback` 函数
- `send_main_menu` 函数
- 所有菜单相关的辅助函数
- `Command` 枚举
- 所有常量和辅助函数（已移到 commands.rs）

- [ ] **Step 2: 简化后的 main.rs 结构**

```rust
mod app;
mod bootstrap;

use bootstrap::{/* imports */};
use tgbot::handlers::{commands, messages, callbacks};
use tgbot::logic;

#[tokio::main]
async fn main() {
    // 1. 初始化日志
    // 2. 运行设置向导（如果需要）
    // 3. 验证完整性
    // 4. 创建 bot 和 dispatcher
    // 5. 启动调度器
    // 6. 运行 bot
}

async fn run_bot(bot: Bot, state: Arc<AppState>) {
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(messages::handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback))
        .branch(Update::filter_message().filter_command::<commands::Command>().endpoint(commands::handle_command));
    
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;
}
```

- [ ] **Step 3: 验证 main.rs 行数 < 200**

```bash
wc -l rust/tgbot/src/main.rs
```

Expected: < 200 行

- [ ] **Step 4: 验证编译**

```bash
cd rust/tgbot
cargo build
```

- [ ] **Step 5: 运行测试**

```bash
cargo test
```

- [ ] **Step 6: 提交**

```bash
git add rust/tgbot/src/main.rs
git commit -m "refactor: cleanup main.rs to <200 lines

- Remove migrated handler code
- Simplified bot initialization
- All handlers now in handlers/ module"
```

---

## Phase 4: 代码优化

### Task 13: 修复 clippy 警告

**Files:**
- Multiple files in `rust/tgbot/src/`

- [ ] **Step 1: 运行 clippy 并修复警告**

```bash
cd rust/tgbot
cargo clippy 2>&1 | head -100
```

主要修复：
- 可折叠的 if 语句
- 函数参数过多（>7）
- 未使用的关联常量
- 未使用的函数
- 缺少 Default trait 实现
- `to_string` 在 Display 类型上的使用

- [ ] **Step 2: 修复 if 语句折叠**

例如，将：
```rust
if x {
    if y {
        // code
    }
}
```
改为：
```rust
if x && y {
    // code
}
```

- [ ] **Step 3: 添加 Default 实现**

为以下结构体添加 Default：
- `SchedulerState`
- `SchedulerValidator`
- `GeoIPService`

- [ ] **Step 4: 修复函数参数过多**

考虑使用结构体包装参数：
```rust
// 之前
fn build_reality_config(a, b, c, d, e, f, g, h, i, j, k, l) {}

// 之后
struct RealityConfigParams { a, b, c, ... }
fn build_reality_config(params: &RealityConfigParams) {}
```

- [ ] **Step 5: 验证 clippy 通过**

```bash
cargo clippy 2>&1 | grep -E "(warning|error)" | wc -l
```

Expected: 0

- [ ] **Step 6: 提交**

```bash
git add rust/tgbot/src/
git commit -m "style: fix all clippy warnings

- Collapsible if statements
- Add Default impl for structs
- Fix unused constants and functions
- Reduce function parameters where possible"
```

---

### Task 14: 添加文档注释

**Files:**
- Multiple files

- [ ] **Step 1: 为 public API 添加文档**

为所有以下项目添加 `///` 文档注释：
- handlers/ 模块中的所有 public 函数
- services/ 模块中的所有 public 函数
- 主要类型和结构体

- [ ] **Step 2: 验证文档构建**

```bash
cargo doc --no-deps
```

- [ ] **Step 3: 提交**

```bash
git add rust/tgbot/src/
git commit -m "docs: add rustdoc comments for public APIs

- Document all public functions in handlers/
- Document all public functions in services/"
```

---

### Task 15: 最终验证

**Files:**
- All files

- [ ] **Step 1: 完整构建验证**

```bash
cd rust/tgbot
cargo clean
cargo build --release
```

- [ ] **Step 2: 运行所有测试**

```bash
cargo test
```

Expected: 所有测试通过

- [ ] **Step 3: 验证代码质量指标**

```bash
# 统计 main.rs 行数
wc -l src/main.rs
# Expected: < 200

# 统计 clippy 警告
cargo clippy 2>&1 | grep -c "warning"
# Expected: 0

# 统计模块数量
find src -name "*.rs" | wc -l
# Expected: 比原来多 8-10 个文件
```

- [ ] **Step 4: 创建最终提交**

```bash
git add .
git commit -m "refactor: complete tgbot architecture refactor

- Extract 4500+ lines from main.rs to <200 lines
- Create three-layer architecture: handlers/services/logic
- Add proper error handling and documentation
- Fix all clippy warnings
- Maintain all existing functionality

BREAKING CHANGE: Module structure reorganized
See .opencode/plans/2025-04-26-tgbot-refactor-design.md"
```

---

## 验收标准

### 功能性
- [ ] 所有 Telegram Bot 命令正常工作 (/start, /auth, /menu, /setsecurityfile)
- [ ] 所有菜单导航正常工作
- [ ] 所有用户管理功能正常工作
- [ ] 所有系统操作正常工作
- [ ] TOTP 验证正常工作
- [ ] 自毁流程正常工作

### 代码质量
- [ ] `main.rs` < 200 行
- [ ] `cargo clippy` 0 警告
- [ ] `cargo test` 全部通过
- [ ] 每个文件 < 300 行
- [ ] 所有 public API 有文档注释

### 架构
- [ ] 清晰的 handlers/services/logic 分层
- [ ] handlers 只处理 UI，不直接调用 logic
- [ ] services 封装业务逻辑
- [ ] logic 层保持不变

---

## 附录

### 文件变更清单

**新增文件 (10+):**
- `src/handlers/mod.rs`
- `src/handlers/commands.rs`
- `src/handlers/messages.rs`
- `src/handlers/callbacks/mod.rs`
- `src/handlers/callbacks/main_menu.rs`
- `src/handlers/callbacks/user_mgmt.rs`
- `src/handlers/callbacks/ops_center.rs`
- `src/handlers/callbacks/settings.rs`
- `src/handlers/callbacks/destruct.rs`
- `src/services/mod.rs`
- `src/services/config_service.rs`
- `src/services/system_service.rs`

**修改文件:**
- `src/lib.rs` - 添加新模块导出
- `src/main.rs` - 精简到 <200 行

**未修改文件:**
- `src/logic/` 所有模块（保持原有实现）
- `src/core/` 所有模块（保持原有实现）
- `src/app/` 所有模块（保持原有实现）
- `src/bootstrap.rs`（保持原有实现）
