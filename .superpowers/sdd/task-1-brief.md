### Task 1: Create shared infrastructure + extend BotAdapter

**Files:**
- Create: `src/shared/mod.rs`
- Create: `src/shared/types.rs`
- Modify: `src/adapters/common/trait.rs`
- Modify: `src/adapters/telegram/adapter.rs`
- Modify: `src/adapters/discord/adapter.rs`
- Modify: `src/adapters/matrix/adapter.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `shared::types::CallbackEvent`, `shared::types::HandlerAction`, `shared::types::HandlerResult`
- Produces: `BotAdapter::answer_callback()`, `BotAdapter::download_file()`, `BotAdapter::capabilities()`
- Produces: `PlatformCapabilities` struct + `TELEGRAM/DISCORD/MATRIX` constants

- [ ] **Step 1.1: Add PlatformCapabilities to trait.rs**

```rust
// src/adapters/common/trait.rs — add after InlineButton

#[derive(Debug, Clone, Copy)]
pub struct PlatformCapabilities {
    pub can_edit_message: bool,
    pub can_delete_message: bool,
    pub has_inline_keyboard: bool,
    pub has_slash_commands: bool,
    pub has_file_transfer: bool,
}

impl PlatformCapabilities {
    pub const TELEGRAM: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: true,
    };
}
```

- [ ] **Step 1.2: Add new methods to BotAdapter trait**

In `trait.rs`, add these methods to the trait with default implementations:

```rust
#[async_trait]
pub trait BotAdapter: Send + Sync {
    // ... existing methods ...

    async fn answer_callback(&self, _target: &TargetId, _callback_id: &str, _text: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
        anyhow::bail!("platform does not support file download")
    }

    fn capabilities(&self) -> PlatformCapabilities;
}
```

- [ ] **Step 1.3: Add mock methods to mockall**

In `trait.rs`, the `#[mockall::automock]` attribute is on the trait. Add to the mock config:

```rust
#[mockall::automock]
#[async_trait]
pub trait BotAdapter: Send + Sync {
    // ...
    async fn answer_callback(&self, target: &TargetId, callback_id: &str, text: Option<&str>) -> Result<()>;
    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>>;
    fn capabilities(&self) -> PlatformCapabilities;
}
```

Note: mockall derives `Expectation` for the new methods automatically.

- [ ] **Step 1.4: Implement capabilities in TelegramAdapter**

```rust
// src/adapters/telegram/adapter.rs — add method to impl BotAdapter for TelegramAdapter

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities::TELEGRAM
}
```

Also add `answer_callback` implementation for Telegram:

```rust
async fn answer_callback(&self, _target: &TargetId, callback_id: &str, text: Option<&str>) -> Result<()> {
    let mut answer = self.bot.answer_callback_query(callback_id);
    if let Some(t) = text {
        answer = answer.text(t);
    }
    answer.await?;
    Ok(())
}
```

- [ ] **Step 1.5: Implement capabilities in DiscordAdapter**

```rust
// src/adapters/discord/adapter.rs

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: false,
    }
}
```

- [ ] **Step 1.6: Implement capabilities in MatrixAdapter**

```rust
// src/adapters/matrix/adapter.rs

fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: false,
        has_slash_commands: false,
        has_file_transfer: true,
    }
}
```

- [ ] **Step 1.7: Create src/shared/mod.rs**

```rust
pub(crate) mod types;
pub(crate) mod handlers;
```

- [ ] **Step 1.8: Create src/shared/types.rs**

```rust
use std::sync::Arc;
use crate::adapters::common::{BotAdapter, MessageId, TargetId};
use anyhow::Result;

pub struct CallbackEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: String,
    pub msg_id: MessageId,
    pub data: String,
    pub callback_id: String,
}

pub enum HandlerAction {
    Done,
    Redirect(String),
}

pub type HandlerResult = Result<HandlerAction>;
```

- [ ] **Step 1.9: Update lib.rs**

```rust
pub mod adapters;
pub mod core;
pub(crate) mod shared;  // match visibility in existing codebase
```

- [ ] **Step 1.10: Run tests**

```bash
cd rust/aegis && cargo test 2>&1 | tail -20
```

Expected: all existing tests pass.

- [ ] **Step 1.11: Commit**

```bash
git add -A && git commit -m "feat(aegis): add shared infrastructure and extend BotAdapter trait"
```

---
