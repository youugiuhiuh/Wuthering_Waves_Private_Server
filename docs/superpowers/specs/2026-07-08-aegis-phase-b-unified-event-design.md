# Aegis Phase B: Unified Event Architecture Design

**Date:** 2026-07-08
**Status:** Approved
**Depends on:** Phase A (shared handlers) — PR #161

## Goal

Unify event reception, authorization, command processing, and destruct flow across all platforms (Telegram, Discord, Matrix, future LINE). Each platform only converts native events to `BotEvent` and calls `shared::dispatch_event()`. All business logic lives in the shared layer.

## Background

Phase A migrated callback handlers to `src/shared/handlers/` using `BotAdapter` trait. However, the following remain Telegram-specific:

- **Event reception**: Telegram uses `teloxide::Dispatcher`, Matrix uses `matrix_sdk` event handlers — each has its own callback loop
- **Authorization**: `callback.rs` checks `state.is_authorized()` with Telegram-specific user ID extraction
- **Command system**: `main.rs` defines `Command` enum with `#[derive(BotCommands)]` (teloxide macro), `handle_command()` uses `Bot`, `Message` types
- **Destruct flow**: `destruct_flow.rs` uses `teloxide::Bot`, `ChatId`, `Message`, `InlineKeyboardMarkup` directly
- **Language selection**: `callback.rs` handles `lang:` prefix with Telegram-specific `timedatectl` call
- **State operations**: `set_timeout:`, `a_warp_add_input` intercepted in `callback.rs` with `AppState` access
- **File download**: `SetSecurityFile` command and destruct flow use `bot.get_file()` + `bot.download_file()` directly

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Unification scope | Full (events + auth + commands + destruct) | Eliminate all platform coupling in business logic |
| Dispatch architecture | Direct dispatch (`dispatch_event()`) | Simplest, no channel overhead, matches Phase A pattern |
| Platform capability differences | Auto-degrade | `BotAdapter.capabilities()` already exists; handlers check and fall back to text |
| Command system | Unified `BotCommand` enum | Each platform parses native commands into same enum |
| File upload | Unified `file_id` + `adapter.download_file()` | Already implemented in Phase A; each adapter handles its own download mechanism |

## Architecture

### Unified Event Types

```rust
// src/shared/types.rs (extended)

pub enum BotEvent {
    Message(MessageEvent),
    Callback(CallbackEvent),   // already exists from Phase A
    Command(CommandEvent),
}

pub struct MessageEvent {
    pub adapter: Arc<dyn BotAdapter>,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,       // platform-specific identifier
    pub reply_to_text: Option<String>, // for context-aware responses
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

### Unified Event Dispatch

```rust
// src/shared/dispatch.rs (new)

pub async fn dispatch_event(event: BotEvent, state: &AppState) -> Result<()> {
    // 1. Unified authorization check
    if !check_auth(&event, state).await {
        return Ok(());
    }

    // 2. Unified destruct flow interception
    if destruct::intercept(&event, state).await? == FlowOutcome::Handled {
        return Ok(());
    }

    // 3. Dispatch by event type
    match event {
        BotEvent::Command(cmd) => commands::handle(cmd, state).await,
        BotEvent::Message(msg) => message::handle_message(msg, state).await,
        BotEvent::Callback(cb) => {
            // State operations (set_timeout, a_warp_add_input, lang:)
            state_ops::intercept(&cb, state).await;
            // Then shared handler dispatch (from Phase A)
            handlers::dispatch(&cb).await
        }
    }
}
```

### Authorization Check (Unified)

```rust
// src/shared/dispatch.rs

async fn check_auth(event: &BotEvent, state: &AppState) -> bool {
    let user_id = event.user_id();
    if !state.is_admin_user(user_id) {
        return false;
    }
    // Commands like Help, Start don't require auth
    // Auth command is always allowed (it's the login attempt)
    // Menu, callbacks require is_authorized()
    // Messages: TOTP codes allowed when not authorized
    match event {
        BotEvent::Command(CommandEvent { command: BotCommand::Auth { .. }, .. }) => true,
        BotEvent::Command(CommandEvent { command: BotCommand::Help | BotCommand::Start, .. }) => true,
        BotEvent::Message(msg) if is_totp_code(&msg.text) => true,
        BotEvent::Callback(_) | BotEvent::Command(_) => state.is_authorized(user_id).await,
        _ => state.is_authorized(user_id).await,
    }
}
```

### Destruct Flow Abstraction

Current `destruct_flow.rs` uses `teloxide::Bot`, `ChatId`, `Message`, `InlineKeyboardMarkup`. The pure logic layer (`process_destruct_message`) is already platform-agnostic. Only the UI layer needs migration:

```rust
// src/shared/destruct.rs (new)

// Replaces destruct_flow.rs — uses BotAdapter instead of teloxide types
// bot.send_message → adapter.send_message
// InlineKeyboardMarkup → Markup { buttons: vec![...] }
// bot.get_file + bot.download_file → adapter.download_file(file_id)
// bot.answer_callback_query → adapter.answer_callback

pub async fn intercept_message(
    msg: &MessageEvent,
    state: &AppState,
) -> Result<FlowOutcome> { ... }

pub async fn intercept_callback(
    cb: &CallbackEvent,
    state: &AppState,
) -> Result<FlowOutcome> { ... }
```

### Command System (Unified)

```rust
// src/shared/commands.rs (new)

pub async fn handle(cmd: CommandEvent, state: &AppState) -> Result<()> {
    match cmd.command {
        BotCommand::Help => {
            // Unified help text from i18n
        }
        BotCommand::Start => {
            // Unified welcome message
        }
        BotCommand::Auth { code } => {
            // Unified TOTP verification using app::auth::process_auth_code
        }
        BotCommand::Menu => {
            // Call shared::handlers::menu::send_main_menu
        }
        BotCommand::SetSecurityFile => {
            // Use adapter.download_file() + SHA-256 hash
            // No more bot.get_file() / bot.download_file()
        }
    }
}
```

### Platform Event Normalizers

Each platform converts native events to `BotEvent`:

**Telegram** (`main/runtime.rs`):
```rust
// teloxide dispatcher → BotEvent
// Update::filter_message() → BotEvent::Message
// filter_command::<BotCommand>() → BotEvent::Command
// filter_callback_query() → BotEvent::Callback
```

**Matrix** (`main/runtime.rs`):
```rust
// matrix_sdk event handler → BotEvent
// OriginalSyncRoomMessageEvent → parse command or → BotEvent::Message
```

**Discord** (future):
```rust
// serenity EventHandler::interaction_create → BotEvent::Callback
// EventHandler::message → BotEvent::Message
```

### State Operations (Unified)

```rust
// src/shared/state_ops.rs (new)

// Operations that need AppState access but are called from shared dispatch
pub async fn intercept(cb: &CallbackEvent, state: &AppState) {
    let data = cb.data.as_str();

    // Language selection
    if data.starts_with("lang:") {
        handle_lang_selection(cb, state).await;
        return;
    }

    // Session timeout
    if data.starts_with("set_timeout:") {
        handle_set_timeout(cb, state).await;
        return;
    }

    // Warp input state
    if data == "a_warp_add_input" {
        state.start_warp_input(cb.target.0.clone(), Instant::now()).await;
        return;
    }
}
```

### File Download (Unified)

`BotAdapter::download_file(file_id)` already implemented in Phase A:
- Telegram: `bot.get_file(file_id)` → `bot.download_file()`
- Matrix: `client.media().get_media_content(mxc://...)`
- Discord: `reqwest::get(attachment_url)`

All callers (`SetSecurityFile`, destruct flow security file verification) use this unified interface.

## File Structure (Post Phase B)

```
src/shared/
├── types.rs          # BotEvent, MessageEvent, CommandEvent, CallbackEvent, BotCommand
├── dispatch.rs       # Unified event entry (auth + destruct intercept + dispatch)
├── commands.rs       # Command handlers (Help/Start/Auth/Menu/SetSecurityFile)
├── destruct.rs       # Destruct flow (using BotAdapter, replaces destruct_flow.rs)
├── state_ops.rs      # State operation intercepts (lang, set_timeout, a_warp_add_input)
├── handlers/         # Callback handlers (from Phase A)
│   ├── mod.rs
│   ├── menu.rs
│   ├── message.rs
│   └── ...
└── types.rs          # Existing shared types
```

## Migration Impact

### Files Created
- `src/shared/dispatch.rs` — unified event dispatch
- `src/shared/commands.rs` — unified command handlers
- `src/shared/destruct.rs` — destruct flow with BotAdapter
- `src/shared/state_ops.rs` — state operation intercepts

### Files Modified
- `src/shared/types.rs` — add `BotEvent`, `MessageEvent`, `CommandEvent`, `BotCommand`
- `src/shared/handlers/mod.rs` — re-export from dispatch
- `src/shared/mod.rs` — declare new modules
- `src/main.rs` — remove `Command` enum, `handle_command`, `looks_like_totp_code`, `process_auth_code`; teloxide dispatcher produces `BotEvent`
- `src/main/runtime.rs` — Telegram/Matrix event handlers produce `BotEvent` and call `shared::dispatch_event()`
- `src/app/destruct_flow.rs` — thin wrapper or deleted (logic moves to `shared/destruct.rs`)
- `src/app/auth.rs` — keep pure logic, called from `shared/commands.rs`

### Files Deleted
- `src/app/destruct_flow.rs` — logic moves to `src/shared/destruct.rs`
- `src/adapters/matrix/handlers.rs` — Matrix command dispatch replaced by unified system

## Platform Capability Auto-Degradation

When a handler needs inline keyboard but `capabilities().has_inline_keyboard == false`:
- Render buttons as numbered text list: `1) Monitor  2) Users  3) Settings`
- User types the number to select

When a platform doesn't support `edit_message`:
- Delete old message + send new message

When a platform doesn't support `answer_callback`:
- No-op (already default implementation)

## Testing Strategy

- Unit tests for `dispatch_event` with mock `BotAdapter` and `AppState`
- Unit tests for `destruct::intercept_message` and `intercept_callback`
- Unit tests for `commands::handle` with each `BotCommand` variant
- Integration tests: Telegram event → `BotEvent` → dispatch → mock adapter receives response
- Existing 504 tests continue passing (no behavior change)

## Non-Goals

- LINE bot adapter implementation (Phase B provides the architecture; LINE is a future task)
- Discord gateway/event handler implementation (same — architecture ready, impl later)
- Removing teloxide dependency entirely (teloxide still used for Telegram event reception)
- Custom schedule creation wizard (removed in Phase A; would be reimplemented as shared handler)
