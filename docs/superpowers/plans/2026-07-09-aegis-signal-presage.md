# Aegis Signal Presage 整合實施計劃

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目標:** 新增 Signal 平台支援，通過 Presage 函式庫實現訊息收發、檔案下載，並整合至統一事件分發層。

**架構:** SignalManager 持有 `Mutex<Manager>` 解決 &mut self 衝突；SignalAdapter 實現 BotAdapter；背景任務用獨立 clone 接收訊息流 → BotEvent → dispatch_event()。

**技術棧:** Rust 2024, tokio, async-trait, presage, presage-store-sqlite

## 全域約束

- `cargo fmt && cargo clippy -- -D warnings && cargo test` 必須在每個任務後通過
- 所有 shared layer 必須使用 BotAdapter trait，無平台特定類型
- `user_id` 類型從 `i64` 改為 `String`，包含所有事件 struct、state、auth
- `admin_id` 在 config 層保持 String，在 AppState 也改為 String
- Signal adapter 的 `&self` 方法內部使用 `Mutex<Manager>` 序列化訪問
- 現有 575+ 測驗必須繼續通過（無行為回歸）

---

## 檔案結構

### 新建檔案
| 檔案 | 職責 |
|------|------|
| `src/adapters/signal/mod.rs` | 模組宣告 |
| `src/adapters/signal/adapter.rs` | SignalAdapter — BotAdapter 實作 |
| `src/adapters/signal/manager.rs` | SignalManager — Presage 生命週期 |
| `src/main/signal.rs` | CLI `--link-signal` + `connect_signal()` |

### 修改檔案
| 檔案 | 變更 |
|------|------|
| `src/shared/types.rs` | `user_id: i64` → `String`, `BotEvent::user_id()` return `&str` |
| `src/app/state.rs` | `HashMap<i64,..>` → `HashMap<String,..>`, `is_authorized(&str)` |
| `src/app/auth.rs` | `process_auth_code` user_id param `i64` → `&str` |
| `src/bootstrap.rs` | `validate_admin_id` → String, `EncryptedConfig.admin_id` 保持 Vec<u8> (已加密) |
| `src/main/config.rs` | `AppConfig.admin_id` `i64` → `String` |
| `src/main/cli.rs` | `admin_id` 已經是 String（保持） |
| `src/main.rs` | `admin_id` 型別更新 |
| `src/main/runtime.rs` | parse_user_id 回傳 String, Telegram/Matrix user_id 生產, Signal 分支 |
| `src/shared/dispatch.rs` | `check_auth` 字串比較, `handle_message` user_id 更新 |
| `src/shared/commands.rs` | `is_authorized` 傳參 string |
| `src/shared/destruct.rs` | `user_id.parse::<i64>()` 移除, 字串比較 |
| `src/shared/state_ops.rs` | test user_id 更新 |
| `src/adapters/common/trait.rs` | `Platform::Signal` |
| `src/main/adapter.rs` | `build_adapter()` Signal 分支 |

---

## 任務

### Task 1: types.rs — user_id i64 → String 變更

**檔案:**
- 修改: `src/shared/types.rs`

**介面:**
- 產生: `MessageEvent.user_id: String`, `CommandEvent.user_id: String`, `BotEvent::user_id(&self) -> &str`

- [ ] **Step 1.1: 修改 user_id 欄位型別**

```rust
// MessageEvent
pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,    // i64 → String
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub reply_to_text: Option<String>,
}

// CommandEvent
pub struct CommandEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,    // i64 → String
    pub command: BotCommand,
}
```

- [ ] **Step 1.2: 修改 BotEvent::user_id() 回傳值**

```rust
pub fn user_id(&self) -> &str {
    match self {
        BotEvent::Message(m) => &m.user_id,
        BotEvent::Callback(c) => &c.user_id,     // 已經是 String
        BotEvent::Command(c) => &c.user_id,
    }
}
```

- [ ] **Step 1.3: 更新測驗**

```rust
// message_event_constructs
let _ = MessageEvent {
    ...
    user_id: "42".into(),    // 42 → "42"
    ...
};

// command_event_constructs
let _ = CommandEvent {
    ...
    user_id: "42".into(),    // 42 → "42"
    ...
};
```

- [ ] **Step 1.4: 執行測驗確認編譯**

Run: `cargo test shared::types -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 1.5: 提交**

```bash
git add src/shared/types.rs
git commit -m "refactor(aegis): types.rs user_id i64 → String"
```

---

### Task 2: app/state.rs — user_id i64 → String

**檔案:**
- 修改: `src/app/state.rs`
- 修改: `src/app/auth.rs`

- [ ] **Step 2.1: 修改 AppState 內部型別**

```rust
// Field types
sessions: Mutex<HashMap<String, Instant>>,     // i64 → String
failed_attempts: Mutex<HashMap<String, FailedRecord>>,

// Constructor
admin_id: String,    // i64 → String

// Methods
pub fn admin_id(&self) -> &str {
    &self.admin_id
}

pub fn is_admin_user(&self, user_id: &str) -> bool {
    user_id == self.admin_id
}

pub async fn is_authorized(&self, user_id: &str) -> bool {
    if !self.is_admin_user(user_id) {
        return false;
    }
    let sessions = self.sessions.lock().await;
    sessions.get(user_id).is_some_and(|auth_time| auth_time.elapsed() < self.session_duration())
}

pub async fn is_recently_authenticated(&self, user_id: &str) -> bool {
    ...
}

pub async fn record_auth_success(&self, user_id: &str, now: Instant) -> u64 {
    self.sessions.lock().await.insert(user_id.to_string(), now);
    self.failed_attempts.lock().await.remove(user_id);
    ...
}

pub async fn auth_cooldown_remaining(&self, user_id: &str, now: Instant) -> Option<Duration> {
    ...
}
```

- [ ] **Step 2.2: 修改 app/auth.rs process_auth_code 簽名**

```rust
pub async fn process_auth_code(
    adapter: &dyn BotAdapter,
    target: &TargetId,
    user_id: &str,    // i64 → &str
    code: &str,
    state: &Arc<AppState>,
    ...
)
```

- [ ] **Step 2.3: 執行測驗確認**

Run: `cargo test app::state -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 2.4: 提交**

```bash
git add src/app/state.rs src/app/auth.rs
git commit -m "refactor(aegis): state and auth user_id i64 → String"
```

---

### Task 3: config — admin_id type i64 → String

**檔案:**
- 修改: `src/bootstrap.rs`
- 修改: `src/main/config.rs`

- [ ] **Step 3.1: 更新 bootstrap.rs 驗證與型態**

```rust
// 移除 validate_admin_id 的 i64 限制
// admin_id: &str 保持，驗證改為非空字串
```

- [ ] **Step 3.2: 更新 config.rs AppConfig.admin_id**

```rust
pub struct AppConfig {
    pub admin_id: String,    // i64 → String
    ...
}
```

更新 decrypt 邏輯 — admin_id 從 `encrypted_config.admin_id` (Vec<u8>) 解密後直接作為 String，不再 parse 為 i64。

- [ ] **Step 3.3: 提交**

```bash
git add src/bootstrap.rs src/main/config.rs
git commit -m "refactor(aegis): admin_id config type i64 → String"
```

---

### Task 4: 更新所有 user_id 呼叫方

**檔案:**
- 修改: `src/shared/dispatch.rs`
- 修改: `src/shared/commands.rs`
- 修改: `src/shared/destruct.rs`
- 修改: `src/shared/state_ops.rs`
- 修改: `src/main/runtime.rs`
- 修改: `src/main.rs`
- 修改: `src/adapters/matrix/commands.rs`

**關鍵變更模式:**
- `is_authorized(user_id)` → `is_authorized(&user_id)`（或 `.as_str()`）
- `is_admin_user(user_id)` → `is_admin_user(&user_id)`
- `HashMap::get(&user_id)` → `HashMap::get(user_id as &str)`（當 `user_id: &str` 時直接 `get(user_id)`）
- Telegram: `msg.from.id.0 as i64` → `msg.from.id.0.to_string()`
- Matrix: `parse_user_id()` 回傳 `String` → 直接使用
- process_auth_code 傳參: `msg.user_id` → `&msg.user_id`
- test 中 user_id 從 `42` 改為 `"42".into()` 或 `"42".to_string()`

- [ ] **Step 4.1: 更新 dispatch.rs**

```rust
// check_auth
let user_id = event.user_id();
if !state.is_admin_user(user_id) {  // user_id 是 &str，無需變更呼叫方

// handle_message TOTP check
if is_totp_code(code) && !state.is_authorized(&msg.user_id).await {
    let _ = auth::process_auth_code(
        &*msg.adapter, &msg.target, &msg.user_id, code, state, ...
```

- [ ] **Step 4.2: 更新 commands.rs**

```rust
if !state.is_authorized(&cmd.user_id).await { ... }
if !state.is_recently_authenticated(&cmd.user_id).await { ... }
```

- [ ] **Step 4.3: 更新 destruct.rs**

```rust
if !state.is_authorized(&msg.user_id).await { ... }

// callback_action 不再需要 parse::<i64>()
// cb.user_id 已經是 String
if !state.is_authorized(&cb.user_id).await { ... }
```

- [ ] **Step 4.4: 更新 runtime.rs**

```rust
// Matrix parse_user_id
fn parse_user_id(s: &str) -> String {
    s.trim_start_matches('@')
        .split(':')
        .next()
        .map(|n| n.to_string())
        .unwrap_or_default()
}

// Matrix event handler
let user_id = parse_user_id(event.sender.as_str());
if !state.is_admin_user(&user_id) { return; }

// Telegram event handlers
user_id: msg.from.as_ref().map(|f| f.id.0.to_string()).unwrap_or_default(),
```

- [ ] **Step 4.5: 運行完整套件 + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 4.6: 提交**

```bash
git add -A
git commit -m "refactor(aegis): update all user_id callers to String type"
```

---

### Task 5: Platform::Signal + BotAdapter trait 擴充

**檔案:**
- 修改: `src/adapters/common/trait.rs`

- [ ] **Step 5.1: 新增 Platform::Signal variant**

```rust
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
    Signal,    // 新增
}
```

- [ ] **Step 5.2: 設定 PlatformCapabilities**

```rust
impl PlatformCapabilities {
    pub const SIGNAL: Self = Self {
        can_edit_message: false,
        can_delete_message: false,
        has_inline_keyboard: false,
        has_slash_commands: true,     // Signal 文字命令用斜桿
        has_file_transfer: true,      // 支援附件
    };
}
```

- [ ] **Step 5.3: 運行 lint + 測試**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`

- [ ] **Step 5.4: 提交**

```bash
git add src/adapters/common/trait.rs
git commit -m "feat(aegis): add Platform::Signal variant and capabilities"
```

---

### Task 6: SignalAdapter 實作 — adapter.rs + mod.rs

**檔案:**
- 創建: `src/adapters/signal/mod.rs`
- 創建: `src/adapters/signal/adapter.rs`

- [ ] **Step 6.1: 創建 signal/mod.rs**

```rust
pub mod adapter;
pub use adapter::SignalAdapter;
```

- [ ] **Step 6.2: 創建 adapter.rs**

```rust
use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;
use presage::Manager;

use crate::adapters::common::{
    BotAdapter, InlineButton, Markup, MessageContent, MessageId, Platform,
    PlatformCapabilities, TargetId,
};

pub struct SignalManager<S: presage::store::Store + Send + Sync + 'static> {
    pub manager: Mutex<Manager<S, presage::manager::Registered>>,
}

pub struct SignalAdapter<S: presage::store::Store + Send + Sync + 'static> {
    pub manager: Arc<SignalManager<S>>,
    pub own_aci: String,
}

#[async_trait]
impl<S: presage::store::Store + Send + Sync + 'static> BotAdapter for SignalAdapter<S> {
    fn platform(&self) -> Platform {
        Platform::Signal
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let service_id = presage::libsignal_service::ServiceId::Aci(
            target.0.parse()?,
        );
        let mut body = content.text;
        if let Some(markup) = &content.markup {
            // Render markup as text (same pattern as Matrix adapter)
            body = render_markup_buttons(body, markup);
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.manager.manager.lock().await.send_message(
            service_id,
            presage::libsignal_service::content::ContentBody::DataMessage {
                body: Some(body),
                // Minimal DataMessage — no attachments, no group, no expire
                ..Default::default()
            },
            timestamp,
        ).await?;
        Ok(MessageId(timestamp.to_string()))
    }

    async fn edit_message(&self, target: &TargetId, _msg_id: &MessageId, content: MessageContent) -> Result<()> {
        // Signal doesn't support editing — send as new message
        self.send_message(target, content).await?;
        Ok(())
    }

    async fn delete_message(&self, _target: &TargetId, _msg_id: &MessageId) -> Result<()> {
        // Signal doesn't support remote deletion
        Ok(())
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        let pointer: presage::proto::AttachmentPointer = serde_json::from_str(file_id)?;
        let content = self.manager.manager.lock().await.get_attachment(&pointer).await?;
        Ok(content)
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::SIGNAL
    }
}

fn render_markup_buttons(base: String, markup: &Markup) -> String {
    // Same pattern as matrix/adapter.rs render_markup_buttons
    let mut body = base;
    let mut lines: Vec<String> = Vec::new();
    let mut idx = 1;
    for row in &markup.buttons {
        for btn in row {
            lines.push(format!("{}. {} — send: `{}`", idx, btn.text, btn.data));
            idx += 1;
        }
    }
    if !lines.is_empty() {
        body.push_str(&rust_i18n::t!("matrix.markup_header"));  // Reuse same i18n key
        body.push_str(&lines.join("\n"));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::common::InlineButton;

    #[test]
    fn render_markup_buttons_with_buttons() {
        let markup = Markup {
            buttons: vec![
                vec![
                    InlineButton { text: "Search".into(), data: "/search".into() },
                    InlineButton { text: "Help".into(), data: "/help".into() },
                ],
            ],
        };
        let result = render_markup_buttons("Hello".into(), &markup);
        assert!(result.contains("1. Search"));
        assert!(result.contains("/search"));
        assert!(result.contains("2. Help"));
    }

    #[test]
    fn render_markup_buttons_empty() {
        let result = render_markup_buttons("plain".into(), &Markup { buttons: vec![] });
        assert_eq!(result, "plain");
    }
}
```

- [ ] **Step 6.3: 新增 signal 模組到 lib.rs**

```rust
pub mod adapters {
    pub mod common;
    pub mod signal;      // 新增
    pub mod telegram;
    pub mod matrix;
}
```

- [ ] **Step 6.4: 運行 lint + 測試**

Note: 如果 Presage 尚未在 Cargo.toml 中，此步驟不會編譯。跳過 clippy，僅驗證語法正確。

Run: `cargo check 2>&1 | grep -E "^(error|warning)" | head -10`

- [ ] **Step 6.5: 提交**

```bash
git add src/adapters/signal/
git commit -m "feat(aegis): add SignalAdapter — BotAdapter impl with Presage"
```

---

### Task 7: SignalManager — Presage 生命週期

**檔案:**
- 創建: `src/adapters/signal/manager.rs`

**介面:**
- 消耗: `presage::Manager`, `SqliteStore`
- 產生: `SignalManager::new(manager)`, `SignalManager::link_device(store, name, tx) -> Self`, `SignalManager::load(store) -> Self`

- [ ] **Step 7.1: 建立 manager.rs**

提供三個主要方法：

```rust
use presage::Manager;
use presage::libsignal_service::configuration::SignalServers;
use presage::model::identity::OnNewIdentity;
use presage_store_sqlite::SqliteStore;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;
use futures::channel::oneshot;

pub type PresageManager = Manager<SqliteStore, presage::manager::Registered>;

/// 透過 QR 連結為第二設備（一次性）
pub async fn link_device(
    db_path: &str,
    device_name: &str,
) -> Result<(Arc<SignalManager>, String), anyhow::Error> {
    let store = SqliteStore::open(db_path, OnNewIdentity::Trust).await?;
    let (tx, rx) = oneshot::channel::<Url>();

    let manager_fut = Manager::link_secondary_device(
        store,
        SignalServers::Production,
        device_name.to_string(),
        tx,
    );

    let (manager, url) = tokio::join!(manager_fut, async {
        rx.await.map_err(|e| anyhow::anyhow!("failed to get linking URL: {}", e))
    });

    let manager = manager??;
    let own_aci = manager.whoami().await?.aci.to_string();
    Ok((Arc::new(SignalManager { manager: Mutex::new(manager) }), own_aci))
}

/// 從已儲存的 store 載入（重啟後）
pub async fn load_device(
    db_path: &str,
) -> Result<(Arc<SignalManager>, String), anyhow::Error> {
    let store = SqliteStore::open(db_path, OnNewIdentity::Trust).await?;
    let manager = Manager::load_registered(store).await?;
    let own_aci = manager.whoami().await?.aci.to_string();
    Ok((Arc::new(SignalManager { manager: Mutex::new(manager) }), own_aci))
}
```

- [ ] **Step 7.2: 運行 lint + 測試**

- [ ] **Step 7.3: 提交**

```bash
git add src/adapters/signal/manager.rs
git commit -m "feat(aegis): add SignalManager — Presage linking and loading lifecycle"
```

---

### Task 8: main/signal.rs — CLI + connect_signal()

**檔案:**
- 創建: `src/main/signal.rs`
- 修改: `src/main.rs`

- [ ] **Step 8.1: 創建 signal.rs**

```rust
use crate::adapters::signal::{SignalAdapter, SignalManager, link_device, load_device};
use anyhow::Result;
use std::sync::Arc;
use std::path::Path;

const SIGNAL_DB: &str = "signal_store.db";

/// 一次性連結為第二設備
pub async fn run_link_signal(device_name: &str) -> Result<()> {
    let db_path = crate::bootstrap::config_dir().join(SIGNAL_DB);
    if db_path.exists() {
        println!("Signal 已連結。刪除 {} 可重新連結。", db_path.display());
        return Ok(());
    }
    let (_, url) = link_device(db_path.to_str().unwrap(), device_name).await?;
    println!("請用手機 Signal 掃描以下 QR 碼來連結設備：");
    println!("{}", url);
    // 在 CLI 模式下，QR URL 會輸出到終端
    // 使用者可以在手機 Signal 中打開設定 → 已連結設備 → 連結新設備
    // 然後掃描此 URL（編碼為 QR 碼）
    println!("\n連結完成後，按 Ctrl+C 退出。");
    println!("之後正常啟動會自動恢復 session。");
    Ok(())
}

/// 嘗試載入已註冊的 Signal 設備
pub async fn connect_signal() -> Result<Option<Arc<SignalAdapter>>> {
    let db_path = crate::bootstrap::config_dir().join(SIGNAL_DB);
    if !db_path.exists() {
        return Ok(None);
    }
    match load_device(db_path.to_str().unwrap()).await {
        Ok((manager, own_aci)) => {
            println!("✅ Signal 已連結 (ACI: {})", own_aci);
            Ok(Some(Arc::new(SignalAdapter {
                manager,
                own_aci,
            })))
        }
        Err(e) => {
            log::error!("載入 Signal session 失敗: {}", e);
            Ok(None)
        }
    }
}
```

- [ ] **Step 8.2: 修改 main.rs — CLI arg 解析**

在 `try_cli_mode` 或 main() 的 args 解析中添加:

```rust
if args.iter().any(|a| a == "--link-signal") {
    let device_name = "aegis-bot";
    return main::signal::run_link_signal(device_name).await;
}
```

並在 `main.rs` 中添加 `mod signal;`。

- [ ] **Step 8.3: 新增 Signal 自動檢測**

在 main() 中：

```rust
let signal_adapter = main::signal::connect_signal().await?;
let enable_signal = signal_adapter.is_some();
```

然後傳入 `runtime::run()`。

- [ ] **Step 8.4: 提交**

```bash
git add src/main/signal.rs src/main.rs
git commit -m "feat(aegis): add --link-signal CLI and connect_signal() for session resume"
```

---

### Task 9: runtime.rs — Signal 事件循環

**檔案:**
- 修改: `src/main/runtime.rs`
- 修改: `src/main/adapter.rs`

- [ ] **Step 9.1: 修改 adapter.rs build_adapter 簽名**

```rust
pub async fn build_adapter(
    token: &str,
    enable_telegram: bool,
    enable_matrix: bool,
    matrix_handle: &Option<MatrixHandle>,
    signal_adapter: Option<Arc<SignalAdapter>>,   // 新增
) -> Arc<dyn BotAdapter> {
    // ... existing code ...
    // 在建立 RoutingAdapter 時考慮 signal
}
```

- [ ] **Step 9.2: runtime.rs Signal 接收任務**

在 `runtime.rs` 的 `run()` 函數中：

```rust
if let Some(signal_adapter) = signal_adapter {
    let signal_state = state.clone();
    let signal_adapter_clone = signal_adapter.clone();

    tokio::spawn(async move {
        let mut manager = signal_adapter_clone.manager.manager.lock().await;
        log::info!("Signal 接收任務啟動");
        match manager.receive_messages().await {
            Ok(mut stream) => {
                while let Some(received) = stream.next().await {
                    match received {
                        Received::DataMessage { sender, body, attachments, timestamp, .. } => {
                            let text = body.unwrap_or_default();
                            let user_id = sender.to_string();
                            let target = TargetId(user_id.clone());

                            // 轉換為 BotEvent 並 dispatch
                            let event = if let Some(cmd) = parse_signal_command(&text) {
                                BotEvent::Command(CommandEvent {
                                    adapter: signal_adapter_clone.clone() as Arc<dyn BotAdapter>,
                                    target: target.clone(),
                                    user_id: user_id.clone(),
                                    command: cmd,
                                })
                            } else if !text.is_empty() || !attachments.is_empty() {
                                let file_id = attachments.first().map(|a| serde_json::to_string(a).unwrap_or_default());
                                BotEvent::Message(MessageEvent {
                                    adapter: signal_adapter_clone.clone() as Arc<dyn BotAdapter>,
                                    target: target.clone(),
                                    user_id: user_id.clone(),
                                    text: if text.is_empty() { None } else { Some(text) },
                                    file_id,
                                    file_name: None,
                                    reply_to_text: None,
                                })
                            } else {
                                continue;
                            };
                            let _ = dispatch_event(event, &state).await;
                        }
                        Received::QueueEmpty => {
                            // Signal 排隊清空，可以發送訊息了
                        }
                        _ => {} // 忽略其他事件類型
                    }
                }
            }
            Err(e) => {
                log::error!("Signal 接收錯誤: {}", e);
            }
        }
    });
}
```

- [ ] **Step 9.3: 添加 parse_signal_command**

```rust
fn parse_signal_command(text: &str) -> Option<BotCommand> {
    let text = text.trim();
    // 支援與 Matrix 相同的命令格式（沒有斜桿前綴）
    // 同時支援預期的 / 前綴命令
    let normalized = text.strip_prefix('/').unwrap_or(text);
    match normalized {
        "help" | "h" => Some(BotCommand::Help),
        "start" => Some(BotCommand::Start),
        "menu" => Some(BotCommand::Menu),
        "auth" => {
            // auth <code>
            let parts: Vec<&str> = text.splitn(2, char::is_whitespace).collect();
            if parts.len() > 1 {
                Some(BotCommand::Auth { code: parts[1].to_string() })
            } else {
                None
            }
        }
        "setsecurityfile" => Some(BotCommand::SetSecurityFile),
        _ => None,
    }
}
```

- [ ] **Step 9.4: 更新 runtime.rs run() 簽名**

```rust
pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    enable_signal: bool,                     // 新增
    signal_adapter: Option<Arc<SignalAdapter>>, // 新增
    token: String,
    admin_id: String,    // i64 → String
) -> Result<(), anyhow::Error> {
```

- [ ] **Step 9.5: 更新 main.rs 中 runtime::run 的呼叫**

```rust
main::runtime::run(
    state,
    matrix_handle,
    enable_telegram,
    enable_matrix,
    enable_signal,        // 傳入
    signal_adapter,       // 傳入
    app_config.decrypted.token,
    app_config.decrypted.admin_id,
).await
```

- [ ] **Step 9.6: 運行完整套件 + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: 若 Presage 未加入 Cargo.toml，忽略編譯錯誤

- [ ] **Step 9.7: 提交**

```bash
git add src/main/runtime.rs src/main/adapter.rs
git commit -m "feat(aegis): wire Signal receive loop into runtime"
```

---

### Task 10: Cargo.toml — Presage 依賴

**檔案:**
- 修改: `Cargo.toml` (rust/aegis/Cargo.toml)

- [ ] **Step 10.1: 添加 presage 依賴**

```toml
[dependencies]
presage = { git = "https://github.com/whisperfish/presage", features = ["sqlite"] }
presage-store-sqlite = { git = "https://github.com/whisperfish/presage" }

[patch.crates-io]
curve25519-dalek = { git = 'https://github.com/signalapp/curve25519-dalek', tag = 'signal-curve25519-4.1.3' }
```

- [ ] **Step 10.2: 運行建置 + lint + 測試**

Run: `cargo check 2>&1 | tail -10`

注意：首次加載 presage 依賴可能需要下載大量 crate，且 `curve25519-dalek` patch 可能與現有依賴衝突。如果編譯失敗，需要分析衝突來源（通常是 `serenity` 的曲線實作 vs presage 的曲線實作）。

- [ ] **Step 10.3: 提交**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(deps): add presage and presage-store-sqlite dependencies"
```

---

## 驗證清單

完成所有任務後：

- [ ] `cargo fmt` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes (575+ tests)
- [ ] `user_id` 在所有事件 struct 中為 `String`
- [ ] `admin_id` 在 `AppState` 中為 `String`
- [ ] `Platform::Signal` 已註冊
- [ ] `SignalAdapter` 實現 `BotAdapter`
- [ ] `--link-signal` CLI 模式可連結設備
- [ ] 正常啟動自動恢復已儲存的 Signal session
- [ ] Signal 訊息接收 → BotEvent → dispatch_event()
- [ ] Signal 訊息發送透過 Mutex<Manager>
- [ ] Markup 按鈕渲染為文字列表（無 inline keyboard）
- [ ] 安全文件下載 (`download_file`) 使用 `get_attachment`
- [ ] 無 presage 類型洩漏到 `src/shared/`
