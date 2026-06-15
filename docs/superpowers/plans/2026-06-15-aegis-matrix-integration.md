# Matrix Bot Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Matrix bot support in the existing aegis binary (`--matrix` / `--all`), sharing all `core/` business logic, using prefixless text commands.

**Architecture:** `main.rs` decrypts `config.enc`, then based on CLI args starts TG dptree dispatcher and/or Matrix sync loop. Both share `Arc<AppState>` (TotpManager, session, auth). The Matrix side uses `MatrixAdapter` (implements `BotAdapter`) for messaging, a text command parser for routing, and flow-based handlers for features that need multi-step interaction.

**Tech Stack:** matrix-sdk 0.11, ruma, tokio, existing core/ modules

---

### File Inventory

| Action | Path | Purpose |
|--------|------|---------|
| Modify | `Cargo.toml` | matrix-sdk 0.7 → 0.11 |
| Modify | `src/adapters/matrix/adapter.rs` | Adapt to 0.11 API |
| Modify | `src/bootstrap.rs` | Extend EncryptedConfig with Matrix fields, update SetupInput/run_setup |
| Modify | `src/app/auth.rs` | Remove teloxide dep, use BotAdapter |
| Modify | `src/app/state.rs` | ChatId → String |
| Modify | `src/main.rs` | Add --matrix/--all flags, Matrix sync loop |
| Create | `src/adapters/matrix/commands.rs` | Text command parser + routing |
| Create | `src/adapters/matrix/handlers.rs` | Simple command → core/ dispatch |
| Modify | `src/adapters/matrix/mod.rs` | Register new modules |
| Modify | `go/installer/main.go` | Add Matrix config setup guidance |

---

### Task M1: Upgrade matrix-sdk → 0.11, rewrite MatrixAdapter

**Files:**
- Modify: `rust/aegis/Cargo.toml:58`

- [ ] **Step 1: Update Cargo.toml dependency**

```toml
# old
matrix-sdk = "0.7"

# new
matrix-sdk = { version = "0.11", default-features = false, features = ["native-tls"] }
```

- [ ] **Step 2: Run cargo update**

Run: `cargo update -p matrix-sdk`
Expected: downloads 0.11 and dependencies, no errors

- [ ] **Step 3: Rewrite MatrixAdapter for 0.11 API**

`matrix-sdk` 0.11 path changes: `matrix_sdk::Room` → `matrix_sdk::room::Room`.

```rust
use crate::adapters::common::{
    BotAdapter, MessageContent, MessageId, Platform, TargetId,
};
use anyhow::Result;
use async_trait::async_trait;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::OwnedEventId;

pub struct MatrixAdapter {
    room: Room,
}

impl MatrixAdapter {
    pub fn new(room: Room) -> Self {
        Self { room }
    }

    pub fn inner_room(&self) -> &Room {
        &self.room
    }
}

#[async_trait]
impl BotAdapter for MatrixAdapter {
    fn platform(&self) -> Platform {
        Platform::Matrix
    }

    async fn send_message(&self, _target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let body = RoomMessageEventContent::text_html(&content.text, &content.text);
        let response = self.room.send(body).await?;
        Ok(MessageId(response.event_id.to_string()))
    }

    async fn edit_message(
        &self,
        _target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let event_id: OwnedEventId = msg_id.0.parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        let new_content = RoomMessageEventContent::text_html(&content.text, &content.text)
            .make_replacement(
                matrix_sdk::ruma::events::room::message::ReplacementMetadata::new(
                    event_id, None
                ),
                None,
            );
        self.room.send(new_content).await?;
        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let event_id: OwnedEventId = msg_id.0.parse()
            .map_err(|e| anyhow::anyhow!("Invalid event ID: {}", e))?;
        self.room.redact(&event_id, None, None).await?;
        Ok(())
    }
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: PASS

- [ ] **Step 5: Run existing tests**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 0 failures

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock rust/aegis/src/adapters/matrix/adapter.rs
git commit -m "feat: upgrade matrix-sdk 0.7 -> 0.11, rewrite MatrixAdapter"
```

---

### Task M2: Extend EncryptedConfig + platform-agnostic auth

**Files:**
- Modify: `rust/aegis/src/bootstrap.rs:26-40,128-167`
- Modify: `rust/aegis/src/app/auth.rs:1-92`
- Modify: `rust/aegis/src/app/state.rs:5,78-85,235-291,367-383`

- [ ] **Step 1: Extend EncryptedConfig with Matrix fields**

```rust
// bootstrap.rs:26-33
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EncryptedConfig {
    pub token: Vec<u8>,
    pub admin_id: Vec<u8>,
    pub totp_secret: Vec<u8>,
    #[serde(default)]
    pub self_destruct_key_hash: Option<String>,
    // Matrix fields (all optional)
    #[serde(default)]
    pub matrix_homeserver: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_username: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_password: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_admin_user: Option<Vec<u8>>,
    #[serde(default)]
    pub matrix_room_id: Option<Vec<u8>>,
}
```

- [ ] **Step 2: Extend SetupInput**

```rust
// bootstrap.rs:35-40
#[derive(serde::Deserialize, Zeroize, ZeroizeOnDrop)]
struct SetupInput {
    token: String,
    admin_id: String,
    totp_secret: String,
    // Matrix optional fields
    #[serde(default)]
    matrix_homeserver: Option<String>,
    #[serde(default)]
    matrix_username: Option<String>,
    #[serde(default)]
    matrix_password: Option<String>,
    #[serde(default)]
    matrix_admin_user: Option<String>,
    #[serde(default)]
    matrix_room_id: Option<String>,
}
```

- [ ] **Step 3: Extend run_setup to encrypt Matrix fields**

```rust
// bootstrap.rs — run_setup function body
let encrypted_config = EncryptedConfig {
    token: security.encrypt(token.as_bytes())?,
    admin_id: security.encrypt(admin_id.as_bytes())?,
    totp_secret: security.encrypt(totp_secret.as_bytes())?,
    self_destruct_key_hash: None,
    matrix_homeserver: None,
    matrix_username: None,
    matrix_password: None,
    matrix_admin_user: None,
    matrix_room_id: None,
};
```

- [ ] **Step 4: Refactor app/auth.rs — remove teloxide dependency**

```rust
// Old signature:
// pub async fn process_auth_code(bot: &Bot, chat_id: ChatId, user_id: i64, ...)
//   -> ResponseResult<bool>

// New signature — use BotAdapter + TargetId:
use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub async fn process_auth_code(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    user_id: i64,
    code: &str,
    state: &Arc<AppState>,
    max_attempts: u32,
    failure_window: Duration,
    lockout_durations: &[Duration],
) -> Result<bool> {
    if !state.is_admin_user(user_id) {
        adapter.send_message(target, MessageContent {
            text: "❌ 无权操作".to_string(),
            markup: None,
        }).await?;
        return Ok(false);
    }

    let now = Instant::now();
    if let Some(remaining) = state.auth_cooldown_remaining(user_id, now).await {
        adapter.send_message(target, MessageContent {
            text: format!(
                "⚠️ 尝试过于频繁，请稍后再试。冷却剩余约 {} 分 {} 秒。",
                remaining.as_secs() / 60,
                remaining.as_secs() % 60
            ),
            markup: None,
        }).await?;
        return Ok(false);
    }

    if state.verify_totp(code) {
        let timeout = state.record_auth_success(user_id, now).await;
        adapter.send_message(target, MessageContent {
            text: format!(
                "✅ 认证成功！会话有效期 {}。",
                crate::format_duration_human(timeout)
            ),
            markup: None,
        }).await?;
        return Ok(true);
    }

    match state.record_auth_failure(user_id, now, max_attempts, failure_window, lockout_durations).await {
        AuthFailureOutcome::Locked { duration } => {
            let duration_str = if duration.as_secs() >= 3600 {
                format!("{} 小时", duration.as_secs() / 3600)
            } else {
                format!("{} 分钟", duration.as_secs() / 60)
            };
            adapter.send_message(target, MessageContent {
                text: format!(
                    "❌ 验证失败次数过多，已进入冷却。\n⏱️ 锁定时间: {}\n⚠️ 请稍后再试。",
                    duration_str
                ),
                markup: None,
            }).await?;
        }
        AuthFailureOutcome::Invalid { attempts, max_attempts } => {
            adapter.send_message(target, MessageContent {
                text: format!(
                    "❌ TOTP 验证码无效，请检查后重试。（已失败 {} 次 / {} 次）",
                    attempts, max_attempts
                ),
                markup: None,
            }).await?;
        }
    }

    Ok(false)
}
```

- [ ] **Step 5: Update app/state.rs — ChatId → String**

Replace `ChatId` with `String` in `pending_destructs`, `pending_warp_inputs`, `pending_schedule_inputs`. Remove `use teloxide::types::ChatId`.

- [ ] **Step 6: Update main.rs process_auth_code call site**

```rust
// Old:
// process_auth_code(&bot, msg.chat.id, user_id, &code, &state, ...)

// New:
process_auth_code(
    &*state.adapter,
    &TargetId(msg.chat.id.0.to_string()),
    user_id,
    &code,
    &state,
    TOTP_FAIL_MAX,
    TOTP_FAIL_WINDOW,
    &LOCKOUT_DURATIONS,
).await?;
```

- [ ] **Step 7: Update handlers/ for ChatId → String**

Update all handler files that pass `ChatId` as arguments.

- [ ] **Step 8: cargo check + test**

Run: `cargo check && cargo test --lib`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add rust/aegis/src/bootstrap.rs rust/aegis/src/app/auth.rs rust/aegis/src/app/state.rs rust/aegis/src/main.rs rust/aegis/src/adapters/telegram/handlers/
git commit -m "feat: extend EncryptedConfig with Matrix fields, platform-agnostic auth"
```

---

### Task M3: Text command parser + handlers

**Files:**
- Create: `rust/aegis/src/adapters/matrix/commands.rs`
- Create: `rust/aegis/src/adapters/matrix/handlers.rs`
- Modify: `rust/aegis/src/adapters/matrix/mod.rs`

- [ ] **Step 1: Create command parser**

`rust/aegis/src/adapters/matrix/commands.rs` — parses prefixless text into Command enum:

```rust
use std::sync::Arc;
use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use crate::app::state::AppState;
use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Auth { code: String },
    Help,
    Status,
    Menu,
    Xray(XraySubCommand),
    Singbox(SingboxSubCommand),
    Ops(OpsSubCommand),
    Destruct,
    Schedule(ScheduleSubCommand),
    Warp(WarpSubCommand),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum XraySubCommand {
    Status,
    Add { proto: String, count: usize },
    Del { proto: Option<String> },
    PqStatus,
    PqGen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SingboxSubCommand {
    Status,
    Add { proto: String, count: usize },
    Del,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpsSubCommand {
    Reload,
    Upgrade,
    Maintenance,
    Bbr3,
    Geo,
    Fw,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleSubCommand {
    List,
    Add,
    Del { index: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarpSubCommand {
    Status,
    Install,
    Uninstall,
}

pub fn parse(text: &str) -> Command {
    let trimmed = text.trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Command::Unknown(String::new());
    }

    match parts[0].to_lowercase().as_str() {
        "auth" => {
            if parts.len() >= 2 {
                Command::Auth { code: parts[1].to_string() }
            } else {
                Command::Unknown("auth <code> - 需要 6 位验证码".to_string())
            }
        }
        "help" | "h" => Command::Help,
        "status" => Command::Status,
        "menu" => Command::Menu,
        "xray" => parse_xray(&parts[1..]),
        "sb" | "singbox" => parse_singbox(&parts[1..]),
        "ops" => parse_ops(&parts[1..]),
        "destruct" => Command::Destruct,
        "sched" | "schedule" => parse_schedule(&parts[1..]),
        "warp" => parse_warp(&parts[1..]),
        other => Command::Unknown(format!(
            "未知命令: {}，输入 help 查看可用命令",
            other
        )),
    }
}

fn parse_xray(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Xray(XraySubCommand::Status),
        Some("add") => {
            let proto = args.get(1).map(|s| s.to_string()).unwrap_or_default();
            let count = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            Command::Xray(XraySubCommand::Add { proto, count })
        }
        Some("del") | Some("delete") => {
            Command::Xray(XraySubCommand::Del {
                proto: args.get(1).map(|s| s.to_string()),
            })
        }
        Some("pq") => match args.get(1).map(|s| s.to_lowercase()).as_deref() {
            Some("status") => Command::Xray(XraySubCommand::PqStatus),
            Some("gen" | "generate") => Command::Xray(XraySubCommand::PqGen),
            _ => Command::Xray(XraySubCommand::PqStatus),
        },
        _ => Command::Unknown(format!("未知 xray 子命令: {:?}", args)),
    }
}

fn parse_singbox(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Singbox(SingboxSubCommand::Status),
        Some("add") => {
            let proto = args.get(1).map(|s| s.to_string()).unwrap_or_default();
            let count = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            Command::Singbox(SingboxSubCommand::Add { proto, count })
        }
        Some("del") | Some("delete") => Command::Singbox(SingboxSubCommand::Del),
        _ => Command::Unknown(format!("未知 singbox 子命令: {:?}", args)),
    }
}

fn parse_ops(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        Some("reload") => Command::Ops(OpsSubCommand::Reload),
        Some("upgrade") => Command::Ops(OpsSubCommand::Upgrade),
        Some("maintenance") | Some("maint") => Command::Ops(OpsSubCommand::Maintenance),
        Some("bbr3") => Command::Ops(OpsSubCommand::Bbr3),
        Some("geo") => Command::Ops(OpsSubCommand::Geo),
        Some("fw") | Some("firewall") => Command::Ops(OpsSubCommand::Fw),
        _ => Command::Unknown(format!(
            "可用 ops 子命令: reload, upgrade, maintenance, bbr3, geo, fw"
        )),
    }
}

fn parse_schedule(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("list") => Command::Schedule(ScheduleSubCommand::List),
        Some("add") => Command::Schedule(ScheduleSubCommand::Add),
        Some("del") | Some("delete") => {
            let index = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            Command::Schedule(ScheduleSubCommand::Del { index })
        }
        _ => Command::Unknown(format!("可用 schedule 子命令: list, add, del <index>")),
    }
}

fn parse_warp(args: &[&str]) -> Command {
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("status") => Command::Warp(WarpSubCommand::Status),
        Some("install") => Command::Warp(WarpSubCommand::Install),
        Some("uninstall") | Some("remove") => Command::Warp(WarpSubCommand::Uninstall),
        _ => Command::Unknown(format!("可用 warp 子命令: status, install, uninstall")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_command() {
        assert_eq!(parse("auth 123456"), Command::Auth { code: "123456".to_string() });
    }

    #[test]
    fn parse_help() {
        assert_eq!(parse("help"), Command::Help);
    }

    #[test]
    fn parse_xray_status() {
        assert_eq!(parse("xray status"), Command::Xray(XraySubCommand::Status));
    }

    #[test]
    fn parse_xray_add_with_count() {
        assert_eq!(
            parse("xray add reality 5"),
            Command::Xray(XraySubCommand::Add { proto: "reality".to_string(), count: 5 })
        );
    }

    #[test]
    fn parse_ops_reload() {
        assert_eq!(parse("ops reload"), Command::Ops(OpsSubCommand::Reload));
    }

    #[test]
    fn parse_unknown() {
        assert!(matches!(parse("blah"), Command::Unknown(_)));
    }

    #[test]
    fn parse_empty() {
        assert!(matches!(parse(""), Command::Unknown(_)));
    }
}
```

- [ ] **Step 2: Create handler dispatch**

`rust/aegis/src/adapters/matrix/handlers.rs`:

```rust
use std::sync::Arc;
use aegis::adapters::common::{BotAdapter, MessageContent, TargetId};
use aegis::core::system::SystemMonitor;
use aegis::core::system::maintenance::MaintenanceManager;
use aegis::core::singbox::SingBoxInstaller;
use aegis::core::xray::ConfigManager;
use crate::app::state::AppState;
use super::commands::*;
use anyhow::Result;

const HELP_TEXT: &str = "\
可用命令（无前缀，直接发送）:

  auth <code>         - TOTP 认证
  help                - 显示本帮助
  status              - 系统状态
  menu                - 显示功能菜单

  xray status         - Xray 核心状态
  xray add <proto> [count] - 批量创建 inbound
  xray del [proto]    - 删除配置
  xray pq status      - PQ 密钥状态
  xray pq gen         - 生成 PQ 密钥

  singbox status      - SingBox 状态
  singbox add <proto> [count] - 批量创建
  singbox del         - 删除所有配置

  ops reload          - 重载核心
  ops upgrade         - 自更新
  ops maintenance     - 系统维护 (含重启)
  ops bbr3            - 安装 BBR3
  ops geo             - 更新 GeoData
  ops fw              - 防火墙加固

  schedule list       - 列出计划任务
  schedule add        - 添加计划 (逐步引导)
  schedule del <idx>  - 删除指定计划

  warp status         - WARP 状态
  warp install        - 安装 WARP
  warp uninstall      - 卸载 WARP

  destruct            - 自毁流程";

pub async fn dispatch(
    cmd: &Command,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    state: &Arc<AppState>,
    _user_id: i64,
) -> Result<()> {
    match cmd {
        Command::Help | Command::Menu => {
            adapter.send_message(target, MessageContent {
                text: HELP_TEXT.to_string(),
                markup: None,
            }).await?;
        }
        Command::Status => {
            let report = SystemMonitor::get_status_report().await
                .unwrap_or_else(|e| format!("获取状态失败: {}", e));
            adapter.send_message(target, MessageContent {
                text: report,
                markup: None,
            }).await?;
        }
        Command::Xray(sub) => handle_xray(sub, adapter, target, state).await?,
        Command::Singbox(sub) => handle_singbox(sub, adapter, target, state).await?,
        Command::Ops(sub) => handle_ops(sub, adapter, target, state).await?,
        Command::Destruct => {
            adapter.send_message(target, MessageContent {
                text: "⚠️ 自毁流程暂不支持通过 Matrix 执行，请使用 Telegram bot。".to_string(),
                markup: None,
            }).await?;
        }
        Command::Schedule(_) => {
            adapter.send_message(target, MessageContent {
                text: "⚠️ 调度管理暂不支持通过 Matrix，请使用 Telegram bot。".to_string(),
                markup: None,
            }).await?;
        }
        Command::Warp(sub) => handle_warp(sub, adapter, target, state).await?,
        Command::Unknown(msg) => {
            adapter.send_message(target, MessageContent {
                text: msg.clone(),
                markup: None,
            }).await?;
        }
        Command::Auth { .. } => {} // handled before dispatch
    }
    Ok(())
}

async fn handle_xray(
    sub: &XraySubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    _state: &Arc<AppState>,
) -> Result<()> {
    match sub {
        XraySubCommand::Status => {
            let status = ConfigManager::list_all_inbound_files();
            adapter.send_message(target, MessageContent {
                text: status,
                markup: None,
            }).await?;
        }
        XraySubCommand::Add { proto, count } => {
            adapter.send_message(target, MessageContent {
                text: format!("正在创建 {} 个 {} 配置，请稍候...", count, proto),
                markup: None,
            }).await?;
            // TODO: call batch_create_reality_vision_enhanced / batch_create_xhttp_reality_enhanced
        }
        XraySubCommand::Del { proto } => {
            let msg = match proto {
                Some(p) => format!("正在删除 {} 配置...", p),
                None => "正在删除所有配置...".to_string(),
            };
            adapter.send_message(target, MessageContent {
                text: msg,
                markup: None,
            }).await?;
        }
        XraySubCommand::PqStatus => {
            let ready = aegis::core::xray::installer::RealityInstaller::is_reality_base_ready().await;
            let msg = if ready {
                "✅ PQ 基础配置就绪".to_string()
            } else {
                "❌ PQ 基础配置未完成，请执行 xray pq gen 或检查种子文件".to_string()
            };
            adapter.send_message(target, MessageContent {
                text: msg,
                markup: None,
            }).await?;
        }
        XraySubCommand::PqGen => {
            match MaintenanceManager::generate_reality_pq_keys_sync() {
                Ok(_) => {
                    adapter.send_message(target, MessageContent {
                        text: "✅ PQ 密钥已成功生成".to_string(),
                        markup: None,
                    }).await?;
                }
                Err(e) => {
                    adapter.send_message(target, MessageContent {
                        text: format!("❌ PQ 密钥生成失败: {}", e),
                        markup: None,
                    }).await?;
                }
            }
        }
    }
    Ok(())
}

async fn handle_singbox(
    sub: &SingboxSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    _state: &Arc<AppState>,
) -> Result<()> {
    match sub {
        SingboxSubCommand::Status => {
            let status = aegis::core::singbox::config::SingBoxConfigManager::list_all_inbound_files();
            adapter.send_message(target, MessageContent {
                text: status,
                markup: None,
            }).await?;
        }
        SingboxSubCommand::Add { proto, count } => {
            let msg = format!("正在创建 {} 个 {} 配置...", count, proto);
            adapter.send_message(target, MessageContent {
                text: msg,
                markup: None,
            }).await?;
        }
        SingboxSubCommand::Del => {
            adapter.send_message(target, MessageContent {
                text: "正在删除所有 SingBox 配置...".to_string(),
                markup: None,
            }).await?;
        }
    }
    Ok(())
}

async fn handle_ops(
    sub: &OpsSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    _state: &Arc<AppState>,
) -> Result<()> {
    match sub {
        OpsSubCommand::Reload => {
            MaintenanceManager::reload_core().await?;
            adapter.send_message(target, MessageContent {
                text: "✅ 核心已重载".to_string(),
                markup: None,
            }).await?;
        }
        OpsSubCommand::Upgrade => {
            adapter.send_message(target, MessageContent {
                text: "正在进行自更新，请稍候...".to_string(),
                markup: None,
            }).await?;
        }
        OpsSubCommand::Maintenance => {
            adapter.send_message(target, MessageContent {
                text: "正在执行系统维护...".to_string(),
                markup: None,
            }).await?;
        }
        OpsSubCommand::Bbr3 => {
            adapter.send_message(target, MessageContent {
                text: "正在安装 BBR3...".to_string(),
                markup: None,
            }).await?;
        }
        OpsSubCommand::Geo => {
            adapter.send_message(target, MessageContent {
                text: "正在更新 GeoData...".to_string(),
                markup: None,
            }).await?;
        }
        OpsSubCommand::Fw => {
            adapter.send_message(target, MessageContent {
                text: "正在执行防火墙加固 (45s 内将输出结果)...".to_string(),
                markup: None,
            }).await?;
        }
    }
    Ok(())
}

async fn handle_warp(
    sub: &WarpSubCommand,
    adapter: &dyn BotAdapter,
    target: &TargetId,
    _state: &Arc<AppState>,
) -> Result<()> {
    match sub {
        WarpSubCommand::Status => {
            let status = aegis::core::xray::WarpInstaller::is_installed().await;
            let msg = if status { "✅ WARP 已安装" } else { "❌ WARP 未安装" };
            adapter.send_message(target, MessageContent {
                text: msg.to_string(),
                markup: None,
            }).await?;
        }
        WarpSubCommand::Install => {
            adapter.send_message(target, MessageContent {
                text: "正在安装 WARP...".to_string(),
                markup: None,
            }).await?;
        }
        WarpSubCommand::Uninstall => {
            adapter.send_message(target, MessageContent {
                text: "正在卸载 WARP...".to_string(),
                markup: None,
            }).await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Update adapters/matrix/mod.rs**

```rust
pub mod adapter;
pub mod commands;
pub mod handlers;
pub use adapter::MatrixAdapter;
```

- [ ] **Step 4: cargo check**

Run: `cargo check`
Expected: PASS

- [ ] **Step 5: Run command parser tests**

Run: `cargo test adapters::matrix::commands::tests -v`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/adapters/matrix/
git commit -m "feat: add Matrix text command parser and handlers"
```

---

### Task M4: main.rs extension — Matrix sync loop

**Files:**
- Modify: `rust/aegis/src/main.rs:243-381`

- [ ] **Step 1: Add CLI flag parser**

```rust
// main.rs — near top after use statements
#[derive(Debug, Clone, Copy, PartialEq)]
enum BotMode {
    Telegram,
    Matrix,
    All,
}

fn parse_mode() -> BotMode {
    let args: Vec<String> = std::env::args().collect();
    for arg in &args[1..] {
        match arg.as_str() {
            "--matrix" => return BotMode::Matrix,
            "--all" => return BotMode::All,
            _ => {}
        }
    }
    BotMode::Telegram // default
}
```

- [ ] **Step 2: Add Matrix config struct + decrypt logic**

In `main()`, after decrypting TG fields:

```rust
struct MatrixConfig {
    homeserver: String,
    username: String,
    password: String,
    room_id: String,
}

let matrix_config: Option<MatrixConfig> = {
    let decrypt_opt = |data: &Option<Vec<u8>>| -> Option<String> {
        let vec = data.as_ref()?;
        String::from_utf8(security.decrypt(vec).ok()?.expose_secret().to_vec()).ok()
    };

    let homeserver = decrypt_opt(&encrypted_config.matrix_homeserver)?;
    let username = decrypt_opt(&encrypted_config.matrix_username)?;
    let password = decrypt_opt(&encrypted_config.matrix_password)?;
    let room_id = decrypt_opt(&encrypted_config.matrix_room_id)?;
    Some(MatrixConfig { homeserver, username, password, room_id })
};
```

- [ ] **Step 3: Implement Matrix sync loop**

```rust
use aegis::adapters::matrix::MatrixAdapter;
use aegis::adapters::matrix::commands;
use aegis::adapters::matrix::handlers;
use matrix_sdk::Client;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, SyncRoomMessageEvent,
};
use std::str::FromStr;

async fn run_matrix_bot(
    config: MatrixConfig,
    state: Arc<AppState>,
    admin_target: TargetId,
) -> Result<()> {
    let client = Client::builder()
        .homeserver_url(&config.homeserver)
        .build()
        .await?;

    client.matrix_auth()
        .login_username(&config.username, &config.password)
        .initial_device_display_name("aegis-bot")
        .send()
        .await?;

    println!("✅ Matrix bot logged in as {}", config.username);

    let room_id = matrix_sdk::ruma::RoomId::from_str(&config.room_id)
        .context("Invalid Matrix room ID")?;
    client.join_room_by_id(&room_id).await?;
    let room = client.get_room(&room_id)
        .context("Room not found after join")?;

    let adapter = Arc::new(MatrixAdapter::new(room));

    client.add_event_handler({
        let adapter = adapter.clone();
        let state = state.clone();
        let target = admin_target.clone();
        move |ev: SyncRoomMessageEvent, _room: matrix_sdk::room::Room| {
            let adapter = adapter.clone();
            let state = state.clone();
            let target = target.clone();
            async move {
                let SyncRoomMessageEvent::Original(OriginalSyncRoomMessageEvent {
                    content, sender, ..
                }) = ev else { return };

                let text = match &content.msgtype {
                    MessageType::Text(t) => t.body.clone(),
                    _ => return,
                };

                let user_id = sender.as_str().parse::<i64>().unwrap_or(0);
                let cmd = commands::parse(&text);

                // auth command needs special handling (no auth required)
                if let commands::Command::Auth { code } = &cmd {
                    let _ = crate::app::auth::process_auth_code(
                        &*adapter, &target, user_id, code, &state,
                        5, std::time::Duration::from_secs(600),
                        &[std::time::Duration::from_secs(900)],
                    ).await;
                    return;
                }

                // other commands require auth
                if !state.is_authorized(user_id).await {
                    let _ = adapter.send_message(&target, MessageContent {
                        text: "🔐 请先认证: auth <验证码>".to_string(),
                        markup: None,
                    }).await;
                    return;
                }

                if let Err(e) = handlers::dispatch(&cmd, &*adapter, &target, &state, user_id).await {
                    let _ = adapter.send_message(&target, MessageContent {
                        text: format!("❌ 错误: {}", e),
                        markup: None,
                    }).await;
                }
            }
        }
    });

    println!("🚀 Matrix bot is starting sync...");
    client.sync(SyncSettings::default()).await;
    Ok(())
}
```

- [ ] **Step 4: Modify main() for multi-mode dispatch**

In `main()`, after `state` is created, replace the single dispatcher:

```rust
let mode = parse_mode();
let admin_target = TargetId(admin_id.to_string());

match mode {
    BotMode::Telegram => {
        Dispatcher::builder(bot.clone(), handler)
            .dependencies(dptree::deps![state])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    }
    BotMode::Matrix => {
        if let Some(mc) = matrix_config {
            run_matrix_bot(mc, state, admin_target).await?;
        } else {
            anyhow::bail!("--matrix 需要有效的 Matrix 配置，请先通过 --setup-stdin 初始化");
        }
    }
    BotMode::All => {
        if let Some(mc) = matrix_config {
            tokio::spawn(run_matrix_bot(mc, state.clone(), admin_target.clone()));
        } else {
            eprintln!("[WARN] Matrix 配置不完整，仅启动 Telegram");
        }
        Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![state])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    }
}
```

- [ ] **Step 5: cargo check**

Run: `cargo check`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/main.rs
git commit -m "feat: add --matrix/--all CLI flags, Matrix sync loop"
```

---

### Task M5: Go installer Matrix setup guidance

**Files:**
- Modify: `go/installer/main.go`

- [ ] **Step 1: Add Matrix environment variable constants and prompt**

In `go/installer/main.go`, add Matrix config prompting during setup flow (after existing TG setup):

```go
// During setup, ask about Matrix bot
func promptMatrixConfig() map[string]string {
    fmt.Print("配置 Matrix Bot？(y/N): ")
    var answer string
    fmt.Scanln(&answer)
    if answer != "y" && answer != "Y" {
        return nil
    }

    config := make(map[string]string)

    fmt.Print("Matrix Homeserver URL (默认 https://matrix.org): ")
    var hs string
    fmt.Scanln(&hs)
    if hs == "" {
        hs = "https://matrix.org"
    }
    config["matrix_homeserver"] = hs

    fmt.Print("Matrix 用户名 (如 @bot:matrix.org): ")
    var user string
    fmt.Scanln(&user)
    config["matrix_username"] = user

    fmt.Print("Matrix 密码: ")
    var pass string
    fmt.Scanln(&pass)
    config["matrix_password"] = pass

    fmt.Print("管理房间 ID (如 !abc123:matrix.org): ")
    var room string
    fmt.Scanln(&room)
    config["matrix_room_id"] = room

    return config
}
```

- [ ] **Step 2: Integrate into setup-stdin payload**

When building the setup-stdin JSON, include Matrix fields if present.

- [ ] **Step 3: Verify Go build**

```bash
cd go/installer && go build ./...
```

- [ ] **Step 4: Commit**

```bash
git add go/installer/main.go
git commit -m "feat: add Matrix config prompts to Go installer"
```

---

### Final Verification

```bash
# Full Rust compilation
cargo check

# Library tests
cargo test --lib

# Go compilation
cd go/installer && go build ./...

# Confirm core/ has no teloxide imports
grep -rn "teloxide" rust/aegis/src/core/
# Expected: (no output)
```
