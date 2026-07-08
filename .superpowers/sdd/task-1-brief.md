## Task 1: Extend types.rs with BotEvent, MessageEvent, CommandEvent, BotCommand

**Files:**
- Modify: `src/shared/types.rs`
- Modify: `src/shared/mod.rs`

**Interfaces:**
- Produces: `BotEvent`, `MessageEvent`, `CommandEvent`, `BotCommand` in `aegis::shared::types`
- Consumes: existing `CallbackEvent`, `TargetId`, `MessageId`, `BotAdapter`

- [ ] **Step 1.1: Write failing test for BotEvent construction**

```rust
// In src/shared/types.rs, add test at bottom:
#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::adapters::common::Markup;

    #[test]
    fn message_event_constructs() {
        // MessageEvent is a plain struct — verify fields compile
        let _ = MessageEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            user_id: 42,
            text: Some("hello".into()),
            file_id: None,
            reply_to_text: None,
        };
    }

    #[test]
    fn command_event_constructs() {
        let _ = CommandEvent {
            adapter: std::sync::Arc::new(crate::adapters::common::MockBotAdapter::new()),
            target: TargetId("123".into()),
            user_id: 42,
            command: BotCommand::Help,
        };
    }

    #[test]
    fn bot_command_auth_carries_code() {
        let cmd = BotCommand::Auth { code: "123456".into() };
        assert!(matches!(cmd, BotCommand::Auth { ref code } if code == "123456"));
    }
}
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cd rust/aegis && cargo test shared::types::event_tests -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `MessageEvent`, `CommandEvent`, `BotCommand` not defined

- [ ] **Step 1.3: Add types to types.rs**

```rust
// src/shared/types.rs — add after existing types

pub enum BotEvent {
    Message(MessageEvent),
    Callback(CallbackEvent),
    Command(CommandEvent),
}

impl BotEvent {
    pub fn user_id(&self) -> i64 {
        match self {
            BotEvent::Message(m) => m.user_id,
            BotEvent::Callback(c) => c.user_id.parse().unwrap_or(0),
            BotEvent::Command(c) => c.user_id,
        }
    }

    pub fn adapter(&self) -> &Arc<dyn BotAdapter> {
        match self {
            BotEvent::Message(m) => &m.adapter,
            BotEvent::Callback(c) => &c.adapter,
            BotEvent::Command(c) => &c.adapter,
        }
    }

    pub fn target(&self) -> &TargetId {
        match self {
            BotEvent::Message(m) => &m.target,
            BotEvent::Callback(c) => &c.target,
            BotEvent::Command(c) => &c.target,
        }
    }
}

pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub reply_to_text: Option<String>,
}

pub struct CommandEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub command: BotCommand,
}

pub enum BotCommand {
    Help,
    Start,
    Menu,
    Auth { code: String },
    SetSecurityFile,
}
```

- [ ] **Step 1.4: Run test to verify it passes**

Run: `cd rust/aegis && cargo test shared::types::event_tests -- --nocapture 2>&1 | tail -10`
Expected: PASS — 3 tests

- [ ] **Step 1.5: Run full suite + lint**

Run: `cd rust/aegis && cargo fmt && cargo clippy -- -D warnings && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failures

- [ ] **Step 1.6: Commit**

```bash
git add -A && git commit -m "feat(aegis): add BotEvent, MessageEvent, CommandEvent, BotCommand types"
```

---
