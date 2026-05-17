# tgbot Handler Trait Dispatcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor main.rs (5184 lines) into a modular Handler Trait Dispatcher architecture, enabling incremental extraction of callback logic into separate handler modules.

**Architecture:** Introduce `CallbackContext`, `HandlerResult`, `CallbackHandler` trait, and `HandlerRegistry`. Phase 1 creates infrastructure with zero behavior change — the entire match block moves into a single `CatchAllHandler`. Phases 2-5 extract handlers one by one, progressively shrinking the catch-all.

**Tech Stack:** Rust 2024 edition, teloxide 0.13, tokio, async-trait

**Working directory:** `rust/tgbot/`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `Cargo.toml` | Modify | Add `async-trait` dependency |
| `src/utils/mod.rs` | Create | `format_duration_human`, `escape_html`, `validate_hash_prefix`, `validate_idx` + unit tests |
| `src/handlers/mod.rs` | Create | `CallbackHandler` trait, `HandlerRegistry`, module declarations |
| `src/handlers/context.rs` | Create | `CallbackContext`, `HandlerResult` types |
| `src/handlers/catch_all.rs` | Create | Phase 1: CatchAllHandler containing entire match block |
| `src/main.rs` | Modify | Replace match block with registry dispatch, add `mod handlers; mod utils;` |

---

## Phase 1: Infrastructure (Zero Behavior Change)

### Task 1: Add async-trait dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add async-trait to Cargo.toml**

Add `async-trait = "0.1"` to the `[dependencies]` section:

```toml
async-trait = "0.1"
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check 2>&1 | tail -5`
Expected: Compilation succeeds (no error about async-trait yet — just added as dependency)

---

### Task 2: Extract utility functions to src/utils/mod.rs

**Files:**
- Create: `src/utils/mod.rs`
- Modify: `src/main.rs` (replace functions with imports)

- [ ] **Step 1: Create src/utils/mod.rs with extracted functions**

Move these 4 functions from `main.rs` to `src/utils/mod.rs`:

```rust
pub fn format_duration_human(secs: u64) -> String {
    if secs < 60 {
        format!("{}秒", secs)
    } else if secs < 3600 {
        format!("{}分钟", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{}小时", h)
        } else {
            format!("{}小时{}分", h, m)
        }
    } else {
        format!("{}天", secs / 86400)
    }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn validate_hash_prefix(prefix: &str) -> Result<&str> {
    if prefix.is_empty() {
        anyhow::bail!("hash 前缀不能为空");
    }
    if prefix.len() > 8 {
        anyhow::bail!("hash 前缀过长: {} (最大 8)", prefix.len());
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("hash 前缀包含无效字符");
    }
    Ok(prefix)
}

pub fn validate_idx(idx: usize, max: usize, field_name: &str) -> Result<()> {
    if idx >= max {
        anyhow::bail!(
            "{} 索引 {} 超出范围 (最大 {})",
            field_name,
            idx,
            max.saturating_sub(1)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration_human(0), "0秒");
        assert_eq!(format_duration_human(45), "45秒");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration_human(60), "1分钟");
        assert_eq!(format_duration_human(90), "1分钟");
        assert_eq!(format_duration_human(120), "2分钟");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration_human(3600), "1小时");
        assert_eq!(format_duration_human(3661), "1小时1分");
    }

    #[test]
    fn format_duration_days() {
        assert!(format_duration_human(86400).starts_with("1天"));
    }

    #[test]
    fn escape_html_entities() {
        assert_eq!(escape_html("<script>alert('1')&\"2\"</script>"),
                   "&lt;script&gt;alert(&apos;1&apos;)&amp;&quot;2&quot;&lt;/script&gt;");
    }

    #[test]
    fn validate_hash_prefix_valid() {
        assert!(validate_hash_prefix("a1b2").is_ok());
        assert!(validate_hash_prefix("abcdef12").is_ok());
    }

    #[test]
    fn validate_hash_prefix_invalid() {
        assert!(validate_hash_prefix("").is_err());
        assert!(validate_hash_prefix("abcdefgh9").is_err()); // too long
        assert!(validate_hash_prefix("xyz").is_err()); // non-hex
    }

    #[test]
    fn validate_idx_bounds() {
        assert!(validate_idx(0, 10, "test").is_ok());
        assert!(validate_idx(9, 10, "test").is_ok());
        assert!(validate_idx(10, 10, "test").is_err());
        assert!(validate_idx(0, 0, "test").is_err());
    }
}
```

- [ ] **Step 2: Remove functions from main.rs and add imports**

In `src/main.rs`:
1. Remove the function bodies of `format_duration_human`, `escape_html`, `validate_hash_prefix`, `validate_idx`
2. Add `mod utils;` alongside `mod app;` and `mod bootstrap;`
3. Add `use crate::utils::{format_duration_human, escape_html, validate_hash_prefix, validate_idx};`
4. Remove the existing test functions in main.rs `mod tests` that test these functions (format_duration_human_seconds, format_duration_human_minutes, format_duration_human_hours_and_days)

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1 | tail -5`
Expected: Clean compilation

- [ ] **Step 4: Run all tests**

Run: `cargo test 2>&1 | grep -E "^test result:"`
Expected: All tests pass (same count as baseline, minus the moved tests which are now in utils module)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: extract utility functions to src/utils/mod.rs

Move format_duration_human, escape_html, validate_hash_prefix, and
validate_idx from main.rs to a dedicated utils module with unit tests."
```

---

### Task 3: Create src/handlers/context.rs

**Files:**
- Create: `src/handlers/context.rs`

- [ ] **Step 1: Create the context module**

Create `src/handlers/context.rs`:

```rust
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId};

use crate::app::state::AppState;

pub struct CallbackContext {
    pub bot: Bot,
    pub chat_id: ChatId,
    pub msg_id: MessageId,
    pub query_id: String,
    pub data: String,
    pub state: Arc<AppState>,
    pub user_id: i64,
}

impl CallbackContext {
    pub fn with_data(&self, data: &str) -> Self {
        Self {
            bot: self.bot.clone(),
            chat_id: self.chat_id,
            msg_id: self.msg_id,
            query_id: self.query_id.clone(),
            data: data.to_string(),
            state: self.state.clone(),
            user_id: self.user_id,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandlerResult {
    Handled,
    NotHandled,
    Redirect(String),
}
```

- [ ] **Step 2: Verify compilation (will fail until Task 4)**

This file won't compile standalone yet — it needs `handlers/mod.rs` to exist. We'll verify in Task 4.

---

### Task 4: Create src/handlers/mod.rs with trait and registry

**Files:**
- Create: `src/handlers/mod.rs`

- [ ] **Step 1: Create the handlers module with trait and registry**

Create `src/handlers/mod.rs`:

```rust
pub mod catch_all;
pub mod context;

use teloxide::prelude::*;

use context::{CallbackContext, HandlerResult};

#[async_trait::async_trait]
pub trait CallbackHandler: Send + Sync {
    fn patterns(&self) -> &[&str];
    async fn handle(&self, ctx: &CallbackContext) -> ResponseResult<HandlerResult>;
}

pub struct HandlerRegistry {
    handlers: Vec<Box<dyn CallbackHandler>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn register(mut self, handler: Box<dyn CallbackHandler>) -> Self {
        self.handlers.push(handler);
        self
    }

    pub async fn dispatch(&self, ctx: &CallbackContext) -> ResponseResult<HandlerResult> {
        for handler in &self.handlers {
            let patterns = handler.patterns();
            let matched = patterns.iter().any(|p| {
                if p.is_empty() {
                    true
                } else {
                    ctx.data == *p || ctx.data.starts_with(p)
                }
            });
            if matched {
                match handler.handle(ctx).await? {
                    result @ (HandlerResult::Handled | HandlerResult::Redirect(_)) => {
                        return Ok(result);
                    }
                    HandlerResult::NotHandled => continue,
                }
            }
        }
        Ok(HandlerResult::NotHandled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_result_equality() {
        assert_eq!(HandlerResult::Handled, HandlerResult::Handled);
        assert_eq!(HandlerResult::NotHandled, HandlerResult::NotHandled);
        assert_eq!(HandlerResult::Redirect("m_main".into()), HandlerResult::Redirect("m_main".into()));
    }

    #[test]
    fn handler_result_debug_format() {
        assert!(format!("{:?}", HandlerResult::Handled).contains("Handled"));
        assert!(format!("{:?}", HandlerResult::NotHandled).contains("NotHandled"));
        assert!(format!("{:?}", HandlerResult::Redirect("test".into())).contains("test"));
    }
}
```

- [ ] **Step 2: Create placeholder catch_all.rs**

Create `src/handlers/catch_all.rs`:

```rust
use teloxide::prelude::*;

use crate::handlers::context::{CallbackContext, HandlerResult};

pub struct CatchAllHandler;

#[async_trait::async_trait]
impl CallbackHandler for CatchAllHandler {
    fn patterns(&self) -> &[&str] {
        &[""]
    }

    async fn handle(&self, _ctx: &CallbackContext) -> ResponseResult<HandlerResult> {
        Ok(HandlerResult::NotHandled)
    }
}
```

- [ ] **Step 3: Add `mod handlers;` to main.rs**

In `src/main.rs`, add at the top (near `mod app;` and `mod bootstrap;`):

```rust
mod handlers;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1 | tail -10`
Expected: Clean compilation. The handlers module compiles but the CatchAllHandler is just a placeholder.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: add Handler Trait Dispatcher infrastructure

Create CallbackContext, HandlerResult, CallbackHandler trait, and
HandlerRegistry in src/handlers/. Add async-trait dependency.
CatchAllHandler is a placeholder for now."
```

---

### Task 5: Populate CatchAllHandler with the full match block

**Files:**
- Modify: `src/handlers/catch_all.rs`

**Transformation rules (apply to every match arm in the current `handle_callback`):**

1. **Replace local variables with `ctx` fields:**
   - `bot` → `ctx.bot` (but clone before spawning: `let bot = ctx.bot.clone();`)
   - `chat_id` → `ctx.chat_id`
   - `msg_id` → `ctx.msg_id`
   - `q.id` or `q.id.clone()` → `ctx.query_id.clone()` (for `answer_callback_query`)
   - `state` → `ctx.state.clone()` or `&ctx.state` (Arc is cheap to clone)
   - `user_id` → `ctx.user_id`
   - `data.as_str()` → `ctx.data.as_str()`

2. **Replace control flow:**
   - `break Ok(());` at end of match arm → `Ok(HandlerResult::Handled)`
   - `return Ok(());` inside a match arm → `return Ok(HandlerResult::Handled);`
   - `continue;` with redirect pattern: `q = CallbackQuery { data: Some("target".to_string()), ..new_q }; continue;` → `return Ok(HandlerResult::Redirect("target".to_string()));`

3. **Handle `q.clone()` patterns:** These become `HandlerResult::Redirect(String)`

4. **Handle spawned tasks:** Use `ctx.bot.clone()` and `ctx.chat_id` instead of the local variables.

5. **The `save_config` function:** Keep in main.rs as `pub(crate) async fn save_config`.

- [ ] **Step 1: Create the complete CatchAllHandler**

Write `src/handlers/catch_all.rs` containing the full match block transformed per the rules above. The file structure is:

```rust
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageId, ParseMode};
use tempfile::NamedTempFile;
use tokio::time;

use tgbot::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use tgbot::core::paths::{singbox, xray};
use tgbot::core::types::IpVersion;
use tgbot::logic::bot_upgrade::UPGRADE_FLAG_FILE;
use tgbot::logic::config::{ConfigManager, KcpMask, Proto, WarpMode};
use tgbot::logic::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
use tgbot::logic::installer::{RealityInstallOutcome, RealityInstaller, WarpInstaller};
use tgbot::logic::log_audit::{LogAudit, SERVICE_SING_BOX, SERVICE_WWPS_CORE};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::operations::Operations;
use tgbot::logic::scheduler::task_types::TaskType;
use tgbot::logic::scheduler::ScheduledTask;
use tgbot::logic::singbox::{SingBoxConfigManager, SingBoxInstaller};
use tgbot::logic::system::SystemMonitor;

use crate::app::batch_handler::send_singbox_batch_result;
use crate::app::state::{AppState, ScheduleFrequency, ScheduleInputState, TimeoutStatus};
use crate::bootstrap::{BotSettings, BOT_VERSION, CONFIG_FILE, DEFAULT_SESSION_TIMEOUT_SECS};
use crate::handlers::context::{CallbackContext, HandlerResult};
use crate::utils::{escape_html, format_duration_human, validate_hash_prefix, validate_idx};

// Helper functions moved here from main.rs
// show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init,
// build_custom_schedule_text, build_custom_schedule_keyboard, etc.

pub struct CatchAllHandler;

#[async_trait::async_trait]
impl super::CallbackHandler for CatchAllHandler {
    fn patterns(&self) -> &[&str] {
        &[""]
    }

    async fn handle(&self, ctx: &CallbackContext) -> ResponseResult<HandlerResult> {
        let data = &ctx.data;
        match data.as_str() {
            "m_main" => { /* ... */ Ok(HandlerResult::Handled) }
            // ... ALL other match arms ...
            _ => {
                ctx.bot.answer_callback_query(ctx.query_id.clone()).await?;
                Ok(HandlerResult::Handled)
            }
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check 2>&1 | tail -20`
Expected: Clean compilation. This may require several iterations to fix import issues.

- [ ] **Step 3: Run all tests**

Run: `cargo test 2>&1 | grep -E "^test result:"`
Expected: All tests pass (same count as baseline)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor: populate CatchAllHandler with full match block

Move all callback handling logic from main.rs into CatchAllHandler.
Redirect patterns converted from loop+continue to HandlerResult::Redirect.
Helper functions moved from main.rs to catch_all.rs module."
```

---

### Task 6: Refactor handle_callback to use registry dispatch

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace handle_callback body with registry dispatch**

Replace the entire `handle_callback` function with the registry-based version that:
1. Does auth check
2. Does destruct_flow_timeout check  
3. Does schedule_timeout check
4. Does destruct_flow_action check
5. Creates CallbackContext and calls REGISTRY.dispatch()
6. Handles HandlerResult::Redirect by re-dispatching (continue loop)
7. Answers callback query for NotHandled

Use `once_cell::sync::Lazy` to create the static REGISTRY:

```rust
use once_cell::sync::Lazy;
use handlers::HandlerRegistry;
use handlers::catch_all::CatchAllHandler;

static REGISTRY: Lazy<HandlerRegistry> = Lazy::new(|| {
    HandlerRegistry::new()
        .register(Box::new(CatchAllHandler))
});
```

- [ ] **Step 2: Remove moved code from main.rs**

Remove the entire match block from handle_callback. Remove the helper functions that moved to catch_all.rs (show_reality_batch_prompt, show_reality_qty_prompt, trigger_reality_auto_init, build_custom_schedule_text, build_custom_schedule_keyboard, build_custom_day_keyboard, build_custom_hour_keyboard, build_custom_minute_keyboard, build_custom_timezone_keyboard, build_cron_from_custom_state, schedule_task_name, schedule_frequency_name, weekday_label, timezone_label, send_main_menu).

- [ ] **Step 3: Add imports**

Add:
```rust
mod handlers;
mod utils;
use handlers::context::{CallbackContext, HandlerResult};
use handlers::HandlerRegistry;
use handlers::catch_all::CatchAllHandler;
use once_cell::sync::Lazy;
```

- [ ] **Step 4: Make save_config pub(crate)**

Change `async fn save_config(state: &Arc<AppState>) -> Result<()>` to `pub(crate) async fn save_config(...)`

- [ ] **Step 5: Verify compilation**

Run: `cargo check 2>&1 | tail -20`
Expected: Clean compilation

- [ ] **Step 6: Run all tests**

Run: `cargo test 2>&1 | grep -E "^test result:"`
Expected: All tests pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: wire up HandlerRegistry in handle_callback

Replace the monolithic match block with registry.dispatch(). 
Pre-dispatch logic (auth, destruct flow, schedule timeout) remains 
in handle_callback. Redirect pattern uses HandlerResult::Redirect."
```

---

### Task 7: Verify Phase 1 complete — no behavior change

- [ ] **Step 1: Run full test suite**

Run: `cargo test 2>&1 | grep -E "^test result:"`
Expected: All tests pass. Total count should match baseline.

- [ ] **Step 2: Run cargo clippy**

Run: `cargo clippy 2>&1 | grep -E "^(error|warning)" | head -20`
Expected: No errors.

- [ ] **Step 3: Verify handle_callback is now a thin wrapper**

The function should be ~50-60 lines containing only pre-dispatch logic and registry dispatch.

- [ ] **Step 4: Commit Phase 1 completion**

```bash
git add -A
git commit -m "refactor: Phase 1 complete — Handler Trait Dispatcher infrastructure

main.rs reduced from 5184 to ~200 lines of handler code.
All callback logic moved to CatchAllHandler.
Behavior is identical to before. All tests pass."
```

---

## Phases 2-5: Extract Independent Modules (Outline)

Each extraction follows the same pattern:
1. Create new handler file
2. Define handler struct with patterns()
3. Move match arms from catch_all.rs
4. Register handler BEFORE CatchAllHandler
5. Remove moved arms from CatchAllHandler
6. Run cargo test

### Task 8: Extract SingboxHandler
### Task 9: Extract WarpHandler  
### Task 10: Extract MonitorHandler
### Task 11: Extract LogHandler
### Task 12: Extract SessionHandler
### Task 13: Extract GeoHandler
### Task 14: Extract ScheduleHandler
### Task 15: Extract NetworkHandler
### Task 16: Extract SecurityHandler
### Task 17: Extract CoreUpgradeHandler
### Task 18: Extract ConfigDeleteHandler
### Task 19: Extract XrayBatchHandler
### Task 20: Extract UserMgmtHandler
### Task 21: Extract MenuHandler
### Task 22: Introduce CallbackAction enum
### Task 23: Remove CatchAllHandler