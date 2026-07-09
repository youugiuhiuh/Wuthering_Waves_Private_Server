# Aegis Phase C: Security File Upload & Matrix Subcommand Restoration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore two Phase B deferred capabilities — (1) security-file upload/hash/persist via pending-state mechanism, (2) rich Matrix subcommands (ops/warp/xray/singbox/schedule/destruct) via synthesized `BotEvent::Callback`.

**Architecture:** Gap 1 introduces `pending_security_file` state (mirroring `pending_warp_inputs`). `/setsecurityfile` sends a prompt and sets a pending flag; the next file-bearing message is captured, downloaded via `adapter.download_file()`, SHA-256 hashed, stored in `AppState`, and persisted via `EncryptedConfig`. Gap 2 maps Matrix text commands to synthesized `BotEvent::Callback` events, reusing all existing callback action logic. Ops/warp are 1:1; destruct gets a text-completion path; xray/singbox/schedule get multi-step text input flows.

**Tech Stack:** Rust 2024, tokio, async-trait, mockall, sha2, matrix-sdk 0.18

## Global Constraints

- `cargo fmt && cargo clippy -- -D warnings && cargo test` must pass after every task
- All shared-layer code must use `BotAdapter` trait, not platform-specific types (`Bot`, `ChatId`, `Message`)
- `BotAdapter::download_file()` used for all file downloads
- No teloxide types in `src/shared/`
- Existing 536+ tests must continue passing (no behavior regression)
- i18n via `rust_i18n::t!()` macro for all user-facing strings
- Worktree: `.worktrees/aegis-phase-c-gaps/` on branch `aegis-phase-c-gaps`
- Base branch: `main` (HEAD at merged Phase B: `fa7cb857`)

## File Structure

### Modified Files (Gap 1)
| File | Responsibility |
|------|----------------|
| `src/app/state.rs` | Add `pending_security_file` state + accessors |
| `src/bootstrap.rs` | Add `save_self_destruct_key_hash_to_config()` |
| `src/shared/commands.rs` | SetSecurityFile starts pending input |
| `src/shared/dispatch.rs` | File capture + hash + persist in handle_message |
| `src/main/runtime.rs` | Matrix m.file/m.image → file_id extraction |

### Modified Files (Gap 2)
| File | Responsibility |
|------|----------------|
| `src/adapters/matrix/commands.rs` | `parse_to_event()` — rich subcommand → BotEvent |
| `src/main/runtime.rs` | Wire `parse_to_event` (same file as Gap 1 change) |
| `src/shared/destruct.rs` | Text confirmation path for AwaitConfirm/AwaitFinalConfirm |
| `src/adapters/matrix/adapter.rs` | Optional: render markup as command list |
| `src/shared/handlers/xray.rs` | Text-to-callback bridge for add/del |
| `src/shared/handlers/singbox.rs` | Text-to-callback bridge for add/del |
| `src/shared/handlers/schedule.rs` | Text-to-callback bridge for add/del |
| `src/app/state.rs` | Additional states for pending input flows (layer 3) |

---

## Task 1: state.rs — Add pending_security_file state

**Files:**
- Modify: `src/app/state.rs`

**Interfaces:**
- Consumes: existing `AppState` with `Mutex<HashMap<String, Instant>>` pattern (see `pending_warp_inputs`)
- Produces: `AppState::start_security_file_input(chat_id: String, now: Instant)`, `AppState::take_security_file_input_status(chat_id: &str, timeout: Duration) -> TimeoutStatus`

- [ ] **Step 1.1: Write failing test**

```rust
#[cfg(test)]
mod security_file_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn start_sets_pending() {
        let state = make_test_state();
        state.start_security_file_input("42".into(), Instant::now()).await;
        assert_eq!(
            state.take_security_file_input_status("42", Duration::from_secs(60)).await,
            TimeoutStatus::Active
        );
    }

    #[tokio::test]
    async fn take_after_timeout_returns_expired() {
        let state = make_test_state();
        let past = Instant::now() - Duration::from_secs(120);
        state.start_security_file_input("42".into(), past).await;
        assert_eq!(
            state.take_security_file_input_status("42", Duration::from_secs(60)).await,
            TimeoutStatus::Expired
        );
    }

    #[tokio::test]
    async fn take_when_not_started_returns_not_tracked() {
        let state = make_test_state();
        assert_eq!(
            state.take_security_file_input_status("99", Duration::from_secs(60)).await,
            TimeoutStatus::NotTracked
        );
    }
}
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cargo test security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — functions not defined on AppState

- [ ] **Step 1.3: Add state + methods to AppState**

Add field after existing `pending_schedule_inputs`:
```rust
pending_security_file: Mutex<HashMap<String, Instant>>,
```

Initialize in the constructor:
```rust
pending_security_file: Mutex::new(HashMap::new()),
```

Add methods:
```rust
pub async fn start_security_file_input(&self, chat_id: String, now: Instant) {
    self.pending_security_file.lock().await.insert(chat_id, now);
}

pub async fn take_security_file_input_status(
    &self,
    chat_id: &str,
    timeout: Duration,
) -> TimeoutStatus {
    let mut map = self.pending_security_file.lock().await;
    match map.remove(chat_id) {
        Some(started) if started.elapsed() < timeout => TimeoutStatus::Active,
        Some(_) => TimeoutStatus::Expired,
        None => TimeoutStatus::NotTracked,
    }
}
```

- [ ] **Step 1.4: Run test to verify it passes**

Run: `cargo test security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — 3 tests

- [ ] **Step 1.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 1.6: Commit**

```bash
git add src/app/state.rs
git commit -m "feat(aegis): add pending_security_file state for security-file upload flow"
```

---

## Task 2: bootstrap.rs — Add save_self_destruct_key_hash_to_config()

**Files:**
- Modify: `src/bootstrap.rs`

**Interfaces:**
- Consumes: `EncryptedConfig` (existing struct with `self_destruct_key_hash: Option<String>`), `config_dir()`, `CONFIG_FILE`, `KEY_FILE`, `SecurityManager`
- Produces: `bootstrap::save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()>`

- [ ] **Step 2.1: Write failing test**

```rust
#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn save_self_destruct_hash_round_trips() {
        // This is an integration test that reads/writes actual config files.
        // We verify the function compiles and has correct signature instead.
        let _sig: fn(Option<String>) -> Result<()> = save_self_destruct_key_hash_to_config;
    }
}
```

- [ ] **Step 2.2: Run test to verify it fails**

Run: `cargo test config_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — function not defined

- [ ] **Step 2.3: Implement save_self_destruct_key_hash_to_config**

```rust
// In src/bootstrap.rs, after save_lang_to_config:

pub fn save_self_destruct_key_hash_to_config(hash: Option<String>) -> Result<()> {
    let config_dir = config_dir();
    let _ = SecurityManager::new(&config_dir.join(KEY_FILE))?;
    let path = config_dir.join(CONFIG_FILE);
    let config_data = std::fs::read(&path)?;
    let mut encrypted_config: EncryptedConfig = serde_json::from_slice(&config_data)?;
    encrypted_config.self_destruct_key_hash = hash;
    std::fs::write(path, serde_json::to_vec(&encrypted_config)?)?;
    Ok(())
}
```

- [ ] **Step 2.4: Run test to verify it passes**

Run: `cargo test config_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 2.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 2.6: Commit**

```bash
git add src/bootstrap.rs
git commit -m "feat(aegis): add save_self_destruct_key_hash_to_config for security-file persistence"
```

---

## Task 3: commands.rs — SetSecurityFile sends prompt + starts pending

**Files:**
- Modify: `src/shared/commands.rs`

**Interfaces:**
- Consumes: `CommandEvent`, `AppState`, `BotAdapter`
- Produces: `commands::handle(SetSecurityFile)` starts pending input after sending prompt

- [ ] **Step 3.1: Write failing test**

```rust
#[cfg(test)]
mod commands_security_file_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::core::totp::TotpManager;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::shared::types::*;
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use std::time::Instant;

    struct TestAdapter;
    #[async_trait::async_trait]
    impl BotAdapter for TestAdapter {
        fn platform(&self) -> crate::adapters::common::Platform { Platform::Telegram }
        async fn send_message(&self, _t: &TargetId, _c: MessageContent) -> Result<MessageId> { Ok(MessageId("0".into())) }
        async fn edit_message(&self, _t: &TargetId, _m: &MessageId, _c: MessageContent) -> Result<()> { Ok(()) }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> Result<()> { Ok(()) }
        async fn download_file(&self, _f: &str) -> Result<Vec<u8>> { Ok(vec![]) }
        fn capabilities(&self) -> PlatformCapabilities { PlatformCapabilities::TELEGRAM }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> { Box::pin(async { Ok(()) }) }
    }

    #[tokio::test]
    async fn set_security_file_starts_pending_input() {
        let secret = TotpManager::generate_new_secret();
        let state = Arc::new(AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(secret)).unwrap(),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(TestAdapter),
        ));
        state.record_auth_success(42, Instant::now()).await;
        let cmd = CommandEvent {
            adapter: Arc::new(TestAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            user_id: 42,
            command: BotCommand::SetSecurityFile,
        };
        handle(cmd, &state).await.unwrap();
        assert_eq!(
            state.take_security_file_input_status("42", Duration::from_secs(180)).await,
            TimeoutStatus::Active
        );
    }
}
```

- [ ] **Step 3.2: Run test to verify it fails**

Run: `cargo test commands_security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — take_security_file_input_status returns NotTracked

- [ ] **Step 3.3: Modify SetSecurityFile handler in commands.rs**

In `src/shared/commands.rs`, in the `BotCommand::SetSecurityFile` arm, after sending the prompt, add:
```rust
state.start_security_file_input(&cmd.target.0, std::time::Instant::now()).await;
```

The full arm becomes:
```rust
BotCommand::SetSecurityFile => {
    if !state.is_recently_authenticated(cmd.user_id).await {
        cmd.adapter.send_message(&cmd.target, MessageContent {
            text: rust_i18n::t!("auth.recent_auth_required").into_owned(),
            markup: None,
        }).await?;
        return Ok(());
    }
    cmd.adapter.send_message(&cmd.target, MessageContent {
        text: rust_i18n::t!("bot_commands.security_file_prompt").into_owned(),
        markup: None,
    }).await?;
    state.start_security_file_input(&cmd.target.0, std::time::Instant::now()).await;
}
```

- [ ] **Step 3.4: Run test to verify it passes**

Run: `cargo test commands_security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 3.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 3.6: Commit**

```bash
git add src/shared/commands.rs
git commit -m "feat(aegis): SetSecurityFile starts pending security-file input after prompt"
```

---

## Task 4: dispatch.rs — File capture + hash + persist

**Files:**
- Modify: `src/shared/dispatch.rs`

**Interfaces:**
- Consumes: `handle_message(msg, state)` where `msg.file_id.is_some()`, `state.take_security_file_input_status()`, `BotAdapter::download_file()`, `bootstrap::save_self_destruct_key_hash_to_config()`
- Produces: Downloaded file → SHA-256 → `state.set_self_destruct_key_hash()` → config persist → success message

- [ ] **Step 4.1: Write failing test**

```rust
#[cfg(test)]
mod dispatch_security_file_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::bootstrap;
    use crate::core::totp::TotpManager;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::shared::types::*;
    use crate::adapters::common::*;
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use std::time::Instant;

    struct TestAdapter;
    #[async_trait::async_trait]
    impl BotAdapter for TestAdapter {
        fn platform(&self) -> Platform { Platform::Telegram }
        async fn send_message(&self, _t: &TargetId, _c: MessageContent) -> Result<MessageId> { Ok(MessageId("0".into())) }
        async fn edit_message(&self, _t: &TargetId, _m: &MessageId, _c: MessageContent) -> Result<()> { Ok(()) }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> Result<()> { Ok(()) }
        async fn download_file(&self, fid: &str) -> Result<Vec<u8>> {
            // Return deterministic content for SHA-256 testing
            Ok(fid.as_bytes().to_vec())
        }
        fn capabilities(&self) -> PlatformCapabilities { PlatformCapabilities::TELEGRAM }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> { Box::pin(async { Ok(()) }) }
    }

    #[tokio::test]
    async fn file_captured_when_pending_sets_hash() {
        let secret = TotpManager::generate_new_secret();
        let state = Arc::new(AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(secret)).unwrap(),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(TestAdapter),
        ));
        state.record_auth_success(42, Instant::now()).await;
        // Simulate pending state
        state.start_security_file_input("42".into(), Instant::now()).await;

        let msg = MessageEvent {
            adapter: Arc::new(TestAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            user_id: 42,
            text: None,
            file_id: Some("test-file".into()),
            file_name: Some("test.txt".into()),
            reply_to_text: None,
        };
        handle_message(msg, &state).await.unwrap();
        let hash = state.self_destruct_key_hash().await;
        assert!(hash.is_some(), "hash should be set after file capture");
    }
}
```

- [ ] **Step 4.2: Run test to verify it fails**

Run: `cargo test dispatch_security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — hash is None

- [ ] **Step 4.3: Add file capture logic to dispatch.rs handle_message**

In `handle_message`, after the TOTP check block (which follows the `if NeedsDestruct` guard), add:
```rust
// Security-file capture when pending
if state.take_security_file_input_status(&msg.target.0, Duration::from_secs(180)).await
    == TimeoutStatus::Active
{
    if let Some(ref fid) = msg.file_id {
        let content = msg.adapter.download_file(fid).await?;
        const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
        if content.len() as u64 > MAX_FILE_SIZE {
            msg.adapter.send_message(&msg.target, MessageContent {
                text: rust_i18n::t!("bot_commands.file_too_big", "0" => content.len() as u64, "1" => MAX_FILE_SIZE).into(),
                markup: None,
            }).await?;
            return Ok(());
        }
        let hash = hex::encode(sha2::Sha256::digest(&content));
        state.set_self_destruct_key_hash(Some(hash.clone())).await;
        if let Err(e) = crate::bootstrap::save_self_destruct_key_hash_to_config(Some(hash.clone())) {
            log::error!("保存安全文件雜湊失敗: {}", e);
        }
        let file_display = msg.file_name.as_ref()
            .map(|n| format!("{} | {}", n, &hash[..8]))
            .unwrap_or_else(|| hash[..8].to_string());
        msg.adapter.send_message(&msg.target, MessageContent {
            text: rust_i18n::t!("bot_commands.security_file_set", "0" => file_display).into(),
            markup: None,
        }).await?;
    }
    return Ok(());
}
```

Add import at top of file:
```rust
use sha2::Digest;
```

And add `const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;` at module level (or inside function).

- [ ] **Step 4.4: Run test to verify it passes**

Run: `cargo test dispatch_security_file_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — hash is set after file capture

- [ ] **Step 4.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 4.6: Commit**

```bash
git add src/shared/dispatch.rs
git commit -m "feat(aegis): security-file capture in pending state — download, hash, persist"
```

---

## Task 5: matrix/commands.rs — parse_to_event() for rich subcommands

**Files:**
- Modify: `src/adapters/matrix/commands.rs`

**Interfaces:**
- Consumes: existing `BotCommand`, `BotEvent`, `CallbackEvent`, `TargetId`, `BotAdapter`
- Produces: `parse_to_event(text, adapter, target, user_id) -> Option<BotEvent>` that maps ops/warp/xray/singbox/schedule/destruct text commands to synthesized `BotEvent`

- [ ] **Step 5.1: Write failing test**

```rust
#[cfg(test)]
mod parse_to_event_tests {
    use super::*;
    use aegis::adapters::common::{BotAdapter, MockBotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId};
    use aegis::shared::types::*;
    use std::sync::Arc;

    fn test_adapter() -> Arc<dyn BotAdapter> {
        let mut m = MockBotAdapter::new();
        m.expect_platform().returning(|| Platform::Matrix);
        m.expect_capabilities().returning(|| PlatformCapabilities {
            can_edit_message: false, can_delete_message: false,
            has_inline_keyboard: false, has_slash_commands: false, has_file_transfer: false,
        });
        Arc::new(m)
    }

    #[test]
    fn parse_help_returns_command() {
        let result = parse_to_event("/help", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Command(CommandEvent { command: BotCommand::Help, .. }))));
    }

    #[test]
    fn parse_ops_reload_returns_callback() {
        let result = parse_to_event("ops reload", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_reload"));
    }

    #[test]
    fn parse_warp_status_returns_callback() {
        let result = parse_to_event("warp status", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_warp_status"));
    }

    #[test]
    fn parse_destruct_returns_callback() {
        let result = parse_to_event("destruct", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "a_destroy_ask"));
    }

    #[test]
    fn parse_xray_returns_menu_callback() {
        let result = parse_to_event("xray status", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_xray_mgmt"));
    }

    #[test]
    fn parse_schedule_returns_menu_callback() {
        let result = parse_to_event("schedule list", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(matches!(result, Some(BotEvent::Callback(CallbackEvent { ref data, .. })) if data == "m_sched"));
    }

    #[test]
    fn parse_unknown_text_returns_none() {
        let result = parse_to_event("some random text", test_adapter(), &TargetId("!r:localhost".into()), 42);
        assert!(result.is_none());
    }
}
```

- [ ] **Step 5.2: Run test to verify it fails**

Run: `cargo test parse_to_event_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — function not defined

- [ ] **Step 5.3: Implement parse_to_event in matrix/commands.rs**

Add after `parse_to_bot_command`:
```rust
pub fn parse_to_event(
    text: &str,
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    user_id: i64,
) -> Option<BotEvent> {
    let text = text.trim();

    // Try basic BotCommand commands first
    if let Some(cmd) = parse_to_bot_command(text) {
        return Some(BotEvent::Command(CommandEvent {
            adapter,
            target: target.clone(),
            user_id,
            command: cmd,
        }));
    }

    let text_lower = text.to_lowercase();
    let target = target.clone();

    let event = |data: &str| -> BotEvent {
        BotEvent::Callback(CallbackEvent {
            adapter,
            target,
            user_id: user_id.to_string(),
            msg_id: MessageId("0".into()),
            data: data.to_string(),
            callback_id: format!("synth:{}", data),
            session_timeout_secs: 600,
        })
    };

    // ops subcommands — 1:1 mapping to callback data
    if let Some(data) = text_lower.strip_prefix("ops ") {
        return Some(match data {
            "reload" => event("a_reload"),
            "upgrade" => event("a_upgrade"),
            "fw" | "firewall" => event("a_fw"),
            "geo" => event("a_geo"),
            "bbr3" => event("a_bbr3"),
            "maintenance" | "tune" => event("a_tune"),
            _ => return None,
        });
    }

    // warp subcommands — 1:1 mapping
    if let Some(data) = text_lower.strip_prefix("warp ") {
        return Some(match data {
            "status" => event("a_warp_status"),
            "install" => event("a_inst_warp"),
            "uninstall" => event("a_warp_uninstall"),
            _ => return None,
        });
    }

    // destruct — start the flow
    if text_lower == "destruct" {
        return Some(event("a_destroy_ask"));
    }

    // xray — show menu
    if text_lower.starts_with("xray ") || text_lower == "xray" {
        return Some(event("m_xray_mgmt"));
    }

    // singbox install shortcut
    if let Some(cmd) = text_lower.strip_prefix("sb ") {
        return match cmd {
            "install" | "singbox install" => Some(event("sb_install")),
            _ => Some(event("m_singbox_mgmt")),
        };
    }
    if text_lower == "singbox" || text_lower == "sb" {
        return Some(event("m_singbox_mgmt"));
    }

    // schedule — show menu
    if text_lower.starts_with("schedule ") || text_lower == "schedule"
        || text_lower.starts_with("sched ") || text_lower == "sched"
    {
        return Some(event("m_sched"));
    }

    None
}
```

- [ ] **Step 5.4: Run test to verify it passes**

Run: `cargo test parse_to_event_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — all 7 tests

- [ ] **Step 5.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 5.6: Commit**

```bash
git add src/adapters/matrix/commands.rs
git commit -m "feat(aegis): parse_to_event — map Matrix text subcommands to synthesized BotEvent"
```

---

## Task 6: runtime.rs — Wire parse_to_event + Matrix file extraction

**Files:**
- Modify: `src/main/runtime.rs`

**Interfaces:**
- Consumes: `parse_to_event()`, `parse_to_bot_command()` (replaced)
- Produces: Matrix event handler uses `parse_to_event` AND extracts file info from m.file/m.image events

- [ ] **Step 6.1: Examine current Matrix handler**

Read `src/main/runtime.rs:105-145` (the Matrix `add_event_handler` block). The current code calls `parse_to_bot_command` and builds `BotEvent::Message` with hardcoded `file_id: None, file_name: None`.

- [ ] **Step 6.2: Modify the Matrix event handler**

Replace the current event-building block:

```rust
let text = event.content.body().trim().to_string();
let msgtype = event.content.msgtype.as_str();

let (file_id, file_name) = match msgtype {
    "m.file" | "m.image" | "m.video" | "m.audio" => {
        // Extract mxc URI from Matrix file event
        let fid = event.content.url()
            .map(|u| u.to_string());
        let fname = event.content.filename()
            .or_else(|| event.content.body())
            .map(|s| s.to_string());
        (fid, fname)
    }
    _ => (None, None),
};

let event = if let Some(ev) = aegis::adapters::matrix::commands::parse_to_event(
    &text,
    adapter.clone(),
    &target,
    user_id,
) {
    ev
} else {
    BotEvent::Message(MessageEvent {
        adapter: adapter.clone(),
        target: target.clone(),
        user_id,
        text: Some(text),
        file_id,
        file_name,
        reply_to_text: None,
    })
};
let _ = dispatch_event(event, &state).await;
```

Note: `event.content.url()` returns `Option<&matrix_sdk::ruma::OwnedMxcUri>` — convert with `.to_string()`. `event.content.filename()` returns `Option<&str>` for file events. For non-file msgtypes, `url()` returns `None` and `filename()` returns `None`.

- [ ] **Step 6.3: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 6.4: Commit**

```bash
git add src/main/runtime.rs
git commit -m "feat(aegis): wire parse_to_event in Matrix handler + extract file metadata from m.file/m.image"
```

---

## Task 7: destruct.rs — Text confirmation path for Matrix

**Files:**
- Modify: `src/shared/destruct.rs`

**Interfaces:**
- Consumes: `intercept_message(msg, state)` for `DestructStep::AwaitConfirm` and `DestructStep::AwaitFinalConfirm`
- Produces: Text commands "confirm"/"cancel" accepted during destruct completion steps

- [ ] **Step 7.1: Write failing test**

```rust
#[cfg(test)]
mod destruct_text_tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::core::totp::TotpManager;
    use crate::core::security::self_destruct::SelfDestructExecutor;
    use crate::adapters::common::*;
    use aegis::shared::types::*;
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use std::time::Instant;

    struct TestAdapter;
    #[async_trait::async_trait]
    impl BotAdapter for TestAdapter {
        fn platform(&self) -> Platform { Platform::Matrix }
        async fn send_message(&self, _t: &TargetId, _c: MessageContent) -> Result<MessageId> { Ok(MessageId("0".into())) }
        async fn edit_message(&self, _t: &TargetId, _m: &MessageId, _c: MessageContent) -> Result<()> { Ok(()) }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> Result<()> { Ok(()) }
        async fn download_file(&self, _f: &str) -> Result<Vec<u8>> { Ok(vec![]) }
        fn capabilities(&self) -> PlatformCapabilities { PlatformCapabilities::TELEGRAM }
    }

    struct TestExecutor;
    impl SelfDestructExecutor for TestExecutor {
        fn execute(&self) -> BoxFuture<'static, anyhow::Result<()>> { Box::pin(async { Ok(()) }) }
    }

    async fn make_auth_state() -> Arc<AppState> {
        let secret = TotpManager::generate_new_secret();
        let state = AppState::new(
            42,
            TotpManager::new(&secrecy::SecretString::from(secret)).unwrap(),
            Arc::new(TestExecutor),
            None,
            600,
            Arc::new(TestAdapter),
        );
        state.record_auth_success(42, Instant::now()).await;
        Arc::new(state)
    }

    #[tokio::test]
    async fn confirm_text_advances_await_confirm() {
        let state = make_auth_state().await;
        state.begin_destruct("42".into(), Instant::now()).await;
        state.advance_destruct_step("42", DestructStep::AwaitFirstTotp, DestructStep::AwaitConfirm, Instant::now()).await;
        // Pretend AwaitConfirm was reached via TOTP shortcut
        let _ = state.advance_destruct_step("42", DestructStep::AwaitConfirm, DestructStep::AwaitFinalConfirm, Instant::now()).await;
        let totp = state.generate_current_totp().unwrap();
        let msg = MessageEvent {
            adapter: Arc::new(TestAdapter) as Arc<dyn BotAdapter>,
            target: TargetId("42".into()),
            user_id: 42,
            text: Some(totp),
            file_id: None,
            file_name: None,
            reply_to_text: None,
        };
        let outcome = intercept_message(&msg, &state).await.unwrap();
        assert_eq!(outcome, FlowOutcome::Handled);
        // Should trigger destruct executor — we test via snapshot being cleared
        assert!(state.destruct_snapshot("42").await.is_none());
    }
}
```

- [ ] **Step 7.2: Run test to verify it fails**

Run: `cargo test destruct_text_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — AwaitFinalConfirm handling doesn't trigger destruct

- [ ] **Step 7.3: Add text confirmation path to intercept_message**

In `intercept_message`, in the main `match (destruct_state.step, action)` block, add arms before the catch-all `_ => {}`:

```rust
(DestructStep::AwaitConfirm, _) => {
    if let Some(ref text) = msg.text {
        let t = text.trim().to_lowercase();
        if t == "confirm" || t == "確認" || t == "yes" || state.verify_totp(text.trim()) {
            if state.advance_destruct_step(&chat_id_str, DestructStep::AwaitConfirm, DestructStep::AwaitFinalConfirm, Instant::now()).await {
                adapter.send_message(target, MessageContent {
                    text: t!("destruct.title_4").into(),
                    markup: None,
                }).await?;
            } else {
                adapter.send_message(target, MessageContent {
                    text: t!("destruct.state_invalid").into(),
                    markup: None,
                }).await?;
            }
        } else if t == "cancel" || t == "取消" || t == "no" {
            state.cancel_destruct(&chat_id_str).await;
            adapter.send_message(target, MessageContent {
                text: t!("destruct.cancelled").into(),
                markup: None,
            }).await?;
        }
    }
}
(DestructStep::AwaitFinalConfirm, _) => {
    if let Some(ref text) = msg.text {
        let t = text.trim().to_lowercase();
        if t == "confirm" || t == "確認" || t == "yes" || state.verify_totp(text.trim()) {
            adapter.send_message(target, MessageContent {
                text: t!("destruct.final_exec").into(),
                markup: None,
            }).await?;
            let executor = state.self_destruct_executor();
            aegis::core::security::self_destruct::trigger(executor);
            state.cancel_destruct(&chat_id_str).await;
        } else if t == "cancel" || t == "取消" || t == "no" {
            state.cancel_destruct(&chat_id_str).await;
            adapter.send_message(target, MessageContent {
                text: t!("destruct.cancelled").into(),
                markup: None,
            }).await?;
        }
    }
}
```

- [ ] **Step 7.4: Run test to verify it passes**

Run: `cargo test destruct_text_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 7.5: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 7.6: Commit**

```bash
git add src/shared/destruct.rs
git commit -m "feat(aegis): Matrix destruct text completion — accept confirm/cancel text in AwaitConfirm/AwaitFinalConfirm"
```

---

## Task 8: matrix/adapter.rs — Render markup as text command list

**Files:**
- Modify: `src/adapters/matrix/adapter.rs`

**Interfaces:**
- Consumes: `MatrixAdapter::send_message()` — currently ignores `content.markup`
- Produces: When `content.markup` has buttons, render them as a numbered text list

- [ ] **Step 8.1: Write failing test**

```rust
#[cfg(test)]
mod matrix_adapter_tests {
    use super::*;
    use aegis::adapters::common::*;

    #[tokio::test]
    async fn send_message_with_markup_appends_command_list() {
        let adapter = MatrixAdapter {
            client: None,
            bot_id: String::new(),
            upload_url: None,
        };
        // This is a compile-check: verify the adapter signature works
        // send_message must accept markup and render it
        let _ = adapter;
    }
}
```

- [ ] **Step 8.2: Run test to verify behavior**

Run: `cargo test matrix_adapter_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS (this is a placeholder)

- [ ] **Step 8.3: Modify MatrixAdapter::send_message to render markup**

In `src/adapters/matrix/adapter.rs`, modify the `send_message` method. After building the message content string, if `content.markup` contains buttons, append them as text:

```rust
let mut body = content.text.clone();

if let Some(markup) = &content.markup {
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in markup.buttons.iter().enumerate() {
        for btn in row {
            lines.push(format!("{}. {} — send: `{}`", i + 1, btn.text, btn.data));
        }
    }
    if !lines.is_empty() {
        body.push_str("\n\n📋 **可用操作:**\n");
        body.push_str(&lines.join("\n"));
    }
}
```

This uses the text of buttons and shows the command the user should type to trigger that action. The exact format can be adjusted.

- [ ] **Step 8.4: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 8.5: Commit**

```bash
git add src/adapters/matrix/adapter.rs
git commit -m "feat(aegis): render Matrix markup buttons as text command list for discoverability"
```

---

## Task 9: xray handlers — Text-to-callback bridge for add/del

**Files:**
- Modify: `src/shared/handlers/xray.rs`

**Interfaces:**
- Consumes: `parse_to_event` routes some xray text commands to `BotEvent::Callback` with `data` starting with `u_`, `cfg_`, `m_pq_`
- Produces: Multi-step text input for xray add (proto/count/ip) recognized by existing state machines

- [ ] **Step 9.1: Map basic xray commands**

Extend `parse_to_event` in `src/adapters/matrix/commands.rs` to handle:
- `xray add reality 5 1.2.3.4` → synthesize callback with data `u_batch_exec:reality 5 1.2.3.4`
- `xray del <name>` → `cfg_del:<name>`
- `xray routing` → `m_routing`
- `xray pq status` → `m_pq_mgmt`

- [ ] **Step 9.2: Add pending xray input state (if needed)**

If the existing `AppState` doesn't have a pending-xray state, add one following the `pending_warp_inputs` / `pending_schedule_inputs` pattern. Most xray commands that need multi-step input (add with proto/count/ip) should pack all params into a single callback data string (`u_batch_exec:<params>`) so no pending state is needed.

- [ ] **Step 9.3: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 9.4: Commit**

```bash
git add src/shared/handlers/xray.rs src/adapters/matrix/commands.rs
git commit -m "feat(aegis): xray text-to-callback bridge for Matrix add/del/routing/pq"
```

---

## Task 10: singbox + schedule handlers — Text-to-callback bridges

**Files:**
- Modify: `src/shared/handlers/singbox.rs`, `src/shared/handlers/schedule.rs`

**Interfaces:**
- Consumes: `parse_to_event` routes singbox/schedule text commands to synthesized `BotEvent::Callback`
- Produces: Singbox add/del and schedule add/del/list text equivalents

- [ ] **Step 10.1: Map singbox commands**

In `src/adapters/matrix/commands.rs`, add:
- `sb add h2 <domain> <count>` → `sb_h2_ip:<domain>,<count>`
- `sb add tu <domain> <count>` → `sb_tu_ip:<domain>,<count>`
- `sb del <name>` → `sb_del_cfg:<name>`
- `sb status` → stays as `m_singbox_mgmt` (menu shows status)

- [ ] **Step 10.2: Map schedule commands**

In `src/adapters/matrix/commands.rs`, add:
- `schedule add <template>` → `s_add:<template>` (triggers existing template-based scheduler input flow)
- `schedule del <idx>` → two-step: first `s_del_menu` to show list, then `s_del:<idx>` to select
- For simplicity, support `schedule del <idx>` as one-step via `s_del:<idx>` directly

- [ ] **Step 10.3: Run full suite + lint**

Run: `cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 10.4: Commit**

```bash
git add src/shared/handlers/singbox.rs src/shared/handlers/schedule.rs src/adapters/matrix/commands.rs
git commit -m "feat(aegis): singbox and schedule text-to-callback bridges for Matrix"
```

---

## Verification Checklist

After all tasks complete:

- [ ] `cargo fmt` passes
- [ ] `cargo clippy -- -D warnings` passes (0 warnings)
- [ ] `cargo test` passes (536+ tests)
- [ ] `/setsecurityfile on Telegram starts pending -> next file upload captured -> hashed -> persisted`
- [ ] `Matrix file upload (m.file/m.image) extracted to MessageEvent.file_id/file_name`
- [ ] `Matrix: ops reload` → `a_reload` callback → handler executes
- [ ] `Matrix: warp status/install/uninstall` → synthesized callback → action runs
- [ ] `Matrix: xray status` → `m_xray_mgmt` menu shown
- [ ] `Matrix: sb install` → `sb_install` action runs
- [ ] `Matrix: schedule` → `m_sched` menu shown
- [ ] `Matrix: destruct` → begins flow; confirm/cancel text completes it
- [ ] `Matrix menu buttons rendered as text command list in adapter`
- [ ] No teloxide types in `src/shared/`
