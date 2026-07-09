# Aegis × Discord (serenity) 完整平台实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Promote Discord from send-only stub to first-class event-receiving platform via `serenity::Client` + `EventHandler`.

**Architecture:** `serenity::Client` (gateway) → EventHandler → `BotEvent` → `dispatch_event()`. Discord runs standalone (`--discord`, no RoutingAdapter). Config fields stored encrypted (`discord_token`, `discord_admin_id`). AppState gets `discord_admin_id: Option<i64>`. 5 slash commands registered at startup.

**Tech Stack:** Rust, serenity 0.12, async_trait, tokio

## Global Constraints

- Use `cargo add` / `cargo remove` for dependency changes (never edit Cargo.toml directly)
- `serenity` is ALREADY in dependencies (non-optional) — no Cargo.toml changes needed
- `user_id` for Discord = `u64 as i64` cast (snowflakes fit positive i64)
- `interaction.defer()` MUST be called within 3 seconds before dispatching
- Discord standalone: `--discord` disables Telegram (like `--matrix`)
- Each task: RED (write failing test) → GREEN (implement) → `cargo test` passes → commit

---

### Task 1: `PlatformCapabilities::DISCORD` const + `DiscordAdapter::capabilities()`

**Files:**
- Modify: `src/adapters/common/trait.rs` — add const
- Modify: `src/adapters/discord/adapter.rs` — use const in `capabilities()`
- Test: inline `#[cfg(test)]` in adapter.rs

**Interfaces:**
- Consumes: `PlatformCapabilities::TELEGRAM` (existing pattern, line ~46)
- Produces: `PlatformCapabilities::DISCORD` const, used by `DiscordAdapter::capabilities()`

- [ ] **Step 1: Write failing test in adapter.rs**

```rust
#[cfg(test)]
mod tests {
    use crate::adapters::common::PlatformCapabilities;

    #[test]
    fn discord_capabilities_matches_expected() {
        let caps = PlatformCapabilities::DISCORD;
        assert!(caps.can_edit_message);
        assert!(caps.can_delete_message);
        assert!(!caps.has_file_transfer);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test discord_capabilities -- --ignored
```
Expected: compile error — `DISCORD` not defined on `PlatformCapabilities`

- [ ] **Step 3: Add const to `src/adapters/common/trait.rs` after the existing `TELEGRAM` const (line ~52)**

```rust
pub const DISCORD: Self = Self {
    can_edit_message: true,
    can_delete_message: true,
    has_inline_keyboard: true,
    has_slash_commands: true,
    has_file_transfer: false,
};
```

- [ ] **Step 4: Replace `DiscordAdapter::capabilities()` (lines 89-97 of adapter.rs)**

Change from manual struct to:
```rust
fn capabilities(&self) -> PlatformCapabilities {
    PlatformCapabilities::DISCORD
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test discord_capabilities -v
```
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/adapters/common/trait.rs src/adapters/discord/adapter.rs
git commit -m "feat(aegis): add PlatformCapabilities::DISCORD const"
```

---

### Task 2: bootstrap.rs — add Discord config fields

**Files:**
- Modify: `src/bootstrap.rs` — EncryptedConfig, SetupInput, Drop impl, run_setup, run_setup_from_stdin
- Modify: `src/main/cli.rs` — add `None, None` param to `run_setup()` call
- Test: inline `#[cfg(test)]` in bootstrap.rs

**Interfaces:**
- Consumes: existing `EncryptedConfig` (line 29), `SetupInput` (line 50), `impl Drop` (line 66), `run_setup` (line 176)
- Produces: expanded `EncryptedConfig` + `SetupInput` with `discord_token`, `discord_admin_id`

- [ ] **Step 1: Add Discord fields to `EncryptedConfig` (after `matrix_store_passphrase`, before `lang`)**

```rust
#[serde(default)]
pub discord_token: Option<Vec<u8>>,
#[serde(default)]
pub discord_admin_id: Option<Vec<u8>>,
```

- [ ] **Step 2: Add Zeroize in `impl Drop for EncryptedConfig` (before the closing `}`)**

```rust
if let Some(v) = &mut self.discord_token {
    v.zeroize();
}
if let Some(v) = &mut self.discord_admin_id {
    v.zeroize();
}
```

- [ ] **Step 3: Add fields to `SetupInput` (after `matrix_store_passphrase`)**

```rust
#[serde(default)]
discord_token: Option<String>,
#[serde(default)]
discord_admin_id: Option<String>,
```

- [ ] **Step 4: Update `run_setup` signature + encrypt Discord fields**

Change signature to:
```rust
pub async fn run_setup(
    token: &str,
    admin_id: &str,
    totp_secret: &str,
    matrix: Option<MatrixSetupConfig>,
    discord_token: Option<&str>,
    discord_admin_id: Option<&str>,
) -> Result<()>
```

In `encrypted_config` construction, add:
```rust
discord_token: discord_token.map(|t| security.encrypt(t.as_bytes()).unwrap()),
discord_admin_id: discord_admin_id.map(|id| security.encrypt(id.as_bytes()).unwrap()),
```

- [ ] **Step 5: Update `run_setup_from_stdin` — extract Discord fields, pass to `run_setup`**

```rust
// After matrix extraction (line ~267)
let discord_token = input.discord_token.take();
let discord_admin_id = input.discord_admin_id.take();
// In the run_setup call:
run_setup(&input.token, &input.admin_id, &input.totp_secret, matrix, discord_token.as_deref(), discord_admin_id.as_deref()).await
```

- [ ] **Step 6: Fix caller in `src/main/cli.rs` line 54 — add `None, None`**

```rust
} => run_setup(&token, &admin_id, &totp_secret, None, None, None).await,
```

- [ ] **Step 7: Add test for serde round-trip**

```rust
#[test]
fn encrypted_config_roundtrip_includes_discord_fields() {
    let config = EncryptedConfig {
        token: vec![1, 2, 3], admin_id: vec![4, 5, 6], totp_secret: vec![7, 8, 9],
        self_destruct_key_hash: None,
        matrix_homeserver: None, matrix_username: None, matrix_password: None,
        matrix_room_id: None, matrix_store_passphrase: None,
        discord_token: Some(vec![10, 11]), discord_admin_id: Some(vec![12, 13, 14]),
        lang: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    let restored: EncryptedConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.discord_token, Some(vec![10, 11]));
    assert_eq!(restored.discord_admin_id, Some(vec![12, 13, 14]));
}
```

- [ ] **Step 8: Run tests to verify**

```bash
cargo test encrypted_config_roundtrip_includes_discord_fields -v
```
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add src/bootstrap.rs src/main/cli.rs
git commit -m "feat(aegis): add discord_token/discord_admin_id to EncryptedConfig"
```

---

### Task 3: config.rs + state.rs — decrypt + AppState

**Files:**
- Modify: `src/main/config.rs` — DecryptedConfig + decrypt fields
- Modify: `src/app/state.rs` — discord_admin_id field + is_admin_user extension
- Modify: `src/main.rs` — pass `None` to new `AppState::new` param
- Test: inline `#[cfg(test)]` in state.rs

**Interfaces:**
- Consumes: `EncryptedConfig.discord_token` / `.discord_admin_id` (from Task 2)
- Produces: `DecryptedConfig.discord_admin_id: Option<i64>`, `AppState::new(..., discord_admin_id: Option<i64>)`

- [ ] **Step 1: Write failing test in state.rs**

```rust
#[cfg(test)]
mod is_admin_user_discord_tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn is_admin_user_accepts_discord_admin() {
        let state = AppState::new(
            42,
            TotpManager::new(&SecretString::from("JBSWY3DPEHPK3PXP")).unwrap(),
            Arc::new(NoopExecutor),
            None, 600,
            Arc::new(crate::adapters::common::MockBotAdapter::new()),
            Some(100), // discord_admin_id
        );
        assert!(state.is_admin_user(42));
        assert!(state.is_admin_user(100));
        assert!(!state.is_admin_user(200));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test is_admin_user_accepts_discord_admin -- --ignored
```
Expected: compile error — wrong arg count for `AppState::new`

- [ ] **Step 3: Update `AppState` in `src/app/state.rs`**

Add field after `admin_id`:
```rust
discord_admin_id: Option<i64>,
```

Extend `new()`:
```rust
pub fn new(
    admin_id: i64,
    totp_manager: TotpManager,
    self_destruct_executor: Arc<dyn SelfDestructExecutor>,
    self_destruct_key_hash: Option<String>,
    session_timeout_secs: u64,
    adapter: Arc<dyn BotAdapter>,
    discord_admin_id: Option<i64>,
) -> Self
```

In the struct literal:
```rust
Self {
    adapter,
    admin_id,
    discord_admin_id,
    // rest unchanged
}
```

Update `is_admin_user`:
```rust
pub fn is_admin_user(&self, user_id: i64) -> bool {
    user_id == self.admin_id
        || self.discord_admin_id.map_or(false, |d| user_id == d)
}
```

- [ ] **Step 4: Add Discord fields to `DecryptedConfig` in `src/main/config.rs`**

```rust
pub struct DecryptedConfig {
    pub token: String,
    pub admin_id: i64,
    pub totp_secret: String,
    pub discord_token: Option<String>,
    pub discord_admin_id: Option<i64>,
    pub encrypted_config: EncryptedConfig,
}
```

In `load_and_validate`, after existing decrypt (after line 65), add decrypt for Discord:

```rust
let discord_token = match &encrypted_config.discord_token {
    Some(v) => {
        let vec = security.decrypt(v).context("解密 discord_token 失败")?;
        Some(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("discord_token 包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string())
    }
    None => None,
};
let discord_admin_id = match &encrypted_config.discord_admin_id {
    Some(v) => {
        let vec = security.decrypt(v).context("解密 discord_admin_id 失败")?;
        let s = String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("discord_admin_id 包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string();
        Some(s.parse::<i64>().context("discord_admin_id 应为整数")?)
    }
    None => None,
};
```

Include in DecryptedConfig:
```rust
DecryptedConfig {
    token, admin_id, totp_secret,
    discord_token, discord_admin_id,
    encrypted_config,
}
```

- [ ] **Step 5: Update callers of `AppState::new`**

In `src/main.rs` (line ~86), add the new param — either `None` (Telegram/Matrix only) or `app_config.decrypted.discord_admin_id`.

For Task 3, pass `None`:
```rust
let state = Arc::new(AppState::new(
    app_config.decrypted.admin_id,
    app_config.totp_manager,
    production_executor(),
    app_config.decrypted.encrypted_config.self_destruct_key_hash.clone(),
    app_config.bot_settings.session_timeout_secs,
    adapter,
    None,  // discord_admin_id — will be wired in Task 5
));
```

Also fix any test callers (the `make_state()` helper and test code constructing AppState directly).

- [ ] **Step 6: Run tests**

```bash
cargo test is_admin_user_accepts_discord_admin -v
```
Expected: PASS
```bash
cargo test
```
Expected: all green (fix any test compilation errors from the AppState::new signature change)

- [ ] **Step 7: Commit**

```bash
git add src/main/config.rs src/app/state.rs src/main.rs
git commit -m "feat(aegis): AppState discord_admin_id + config decrypt"
```

---

### Task 4: `main/discord.rs` — new file

**Files:**
- Create: `src/main/discord.rs`
- Modify: `src/main/mod.rs` — add `pub mod discord;`

**Interfaces:**
- Consumes: `DiscordAdapter::new(http: Arc<Http>)`, `serenity` types
- Produces:
  - `DiscordRawHandle { pub token: String, pub http: Arc<Http>, pub admin_channel: ChannelId, pub adapter: Arc<dyn BotAdapter> }`
  - `pub fn has_discord_config(enc: &EncryptedConfig, args: &[String]) -> bool`
  - `pub fn register_slash_commands(http: &Http) -> Result<()>`
  - `pub async fn connect_discord(security, enc, config_dir) -> Result<DiscordRawHandle>`
  - `pub struct DiscordHandler { state: Arc<AppState>, adapter: Arc<dyn BotAdapter>, admin_channel: ChannelId }`
  - `impl EventHandler for DiscordHandler`
  - `fn parse_slash(name, code) -> Option<BotCommand>` (pure fn, testable)

Note: `connect_discord` does NOT build `serenity::Client` (that happens in `runtime.rs` Task 5, after `AppState` exists). It returns `DiscordRawHandle` with token, http, channel, adapter.

- [ ] **Step 1: Write test for `parse_slash`**

```rust
#[cfg(test)]
mod tests {
    use aegis::shared::types::BotCommand;
    use super::parse_slash;

    #[test]
    fn parse_slash_known_commands() {
        assert_eq!(parse_slash("help", None), Some(BotCommand::Help));
        assert_eq!(parse_slash("start", None), Some(BotCommand::Start));
        assert_eq!(parse_slash("menu", None), Some(BotCommand::Menu));
        assert_eq!(parse_slash("auth", Some("123456")), Some(BotCommand::Auth { code: "123456".into() }));
        assert_eq!(parse_slash("setsecurityfile", None), Some(BotCommand::SetSecurityFile));
    }

    #[test]
    fn parse_slash_unknown_returns_none() {
        assert_eq!(parse_slash("unknown", None), None);
        assert_eq!(parse_slash("auth", None), None);
    }
}
```

- [ ] **Step 2: Write test for `has_discord_config`**

```rust
fn make_enc() -> crate::bootstrap::EncryptedConfig {
    crate::bootstrap::EncryptedConfig {
        token: vec![], admin_id: vec![], totp_secret: vec![],
        self_destruct_key_hash: None,
        matrix_homeserver: None, matrix_username: None,
        matrix_password: None, matrix_room_id: None,
        matrix_store_passphrase: None,
        discord_token: None, discord_admin_id: None,
        lang: None,
    }
}

#[test]
fn has_discord_config_by_flag() {
    let enc = make_enc();
    assert!(super::has_discord_config(&enc, &["--discord".to_string()]));
    assert!(super::has_discord_config(&enc, &["--all".to_string()]));
}

#[test]
fn has_discord_config_by_fields() {
    let mut enc = make_enc();
    enc.discord_token = Some(vec![1]);
    enc.discord_admin_id = Some(vec![2]);
    assert!(super::has_discord_config(&enc, &[]));
}

#[test]
fn has_discord_config_false_when_missing() {
    let enc = make_enc();
    assert!(!super::has_discord_config(&enc, &[]));
}
```

- [ ] **Step 3: Run to verify it fails**

```bash
cargo test parse_slash has_discord_config -- --ignored
```
Expected: compile errors (file doesn't exist)

- [ ] **Step 4: Add `pub mod discord;` to `src/main/mod.rs`**

- [ ] **Step 5: Implement `src/main/discord.rs`**

```rust
use std::path::Path;
use std::sync::Arc;

use aegis::adapters::common::BotAdapter;
use aegis::adapters::discord::DiscordAdapter;
use aegis::app::state::AppState;
use aegis::shared::dispatch_event;
use aegis::shared::types::*;
use anyhow::{Context, Result};
use serenity::all::{
    ChannelId, CommandDataOptionValue, CreateCommand, CreateInteractionResponse,
    GatewayIntents, Interaction, Message, UserId,
};
use serenity::async_trait;
use serenity::client::{Client, Context as SerenityCtx, EventHandler};
use serenity::http::Http;
use secrecy::ExposeSecret;

use crate::bootstrap::EncryptedConfig;

/// Raw materials for Discord runtime wiring.
/// Client is built in runtime.rs AFTER AppState exists.
pub struct DiscordRawHandle {
    pub token: String,
    pub http: Arc<Http>,
    pub admin_channel: ChannelId,
    pub adapter: Arc<dyn BotAdapter>,
}

pub type DiscordHandle = (Client, ChannelId, Arc<dyn BotAdapter>);

fn parse_slash(name: &str, code: Option<&str>) -> Option<BotCommand> {
    match name {
        "help" => Some(BotCommand::Help),
        "start" => Some(BotCommand::Start),
        "menu" => Some(BotCommand::Menu),
        "auth" => code.map(|c| BotCommand::Auth {
            code: c.to_string(),
        }),
        "setsecurityfile" => Some(BotCommand::SetSecurityFile),
        _ => None,
    }
}

pub fn has_discord_config(enc: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--discord" || a == "--all");
    explicit
        || (enc.discord_token.is_some() && enc.discord_admin_id.is_some())
}

pub async fn register_slash_commands(http: &Http) -> Result<()> {
    http.set_global_commands(&[
        CreateCommand::new("help").description("Show help"),
        CreateCommand::new("start").description("Start bot"),
        CreateCommand::new("menu").description("Show admin menu"),
        CreateCommand::new("auth")
            .description("Verify TOTP code")
            .add_option(
                serenity::all::CommandOption::new(
                    serenity::all::CommandOptionType::String,
                    "code",
                    "6-digit TOTP code",
                )
                .required(true),
            ),
        CreateCommand::new("setsecurityfile")
            .description("Set destruct verification file"),
    ])
    .await?;
    Ok(())
}

pub async fn connect_discord(
    security: &aegis::core::security::SecurityManager,
    enc: &EncryptedConfig,
    config_dir: &Path,
) -> Result<DiscordRawHandle> {
    let _ = config_dir; // reserved for future use (e.g. session persistence)

    let decrypt = |field: &Option<Vec<u8>>| -> Result<String> {
        let vec = security
            .decrypt(field.as_ref().with_context(|| "缺少 Discord 配置项")?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("Discord 字段包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string())
    };

    let discord_token = decrypt(&enc.discord_token)?;
    let discord_admin_id_str = decrypt(&enc.discord_admin_id)?;
    let discord_admin_id: u64 = discord_admin_id_str
        .parse()
        .context("discord_admin_id 应为用户 ID(数字)")?;

    let http = Arc::new(Http::new(&discord_token));

    let admin_user = UserId::new(discord_admin_id);
    let admin_channel = admin_user.create_dm_channel(&http).await?.id;

    let adapter: Arc<dyn BotAdapter> = Arc::new(DiscordAdapter::new(http.clone()));

    register_slash_commands(&http).await?;

    Ok(DiscordRawHandle {
        token: discord_token,
        http,
        admin_channel,
        adapter,
    })
}

/// Build a `(Client, ChannelId, Adapter)` tuple from a RawHandle + AppState.
/// Called in runtime.rs where AppState exists.
pub async fn build_handle(
    raw: DiscordRawHandle,
    state: Arc<AppState>,
) -> Result<DiscordHandle> {
    let intents = GatewayIntents::DIRECT_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let handler = DiscordHandler {
        state,
        adapter: raw.adapter.clone(),
        admin_channel: raw.admin_channel,
    };
    let client = Client::builder(&raw.token, intents)
        .event_handler(handler)
        .await
        .context("构建 Discord Client 失败")?;
    Ok((client, raw.admin_channel, raw.adapter))
}

struct DiscordHandler {
    state: Arc<AppState>,
    adapter: Arc<dyn BotAdapter>,
    admin_channel: ChannelId,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, _ctx: SerenityCtx, msg: Message) {
        if msg.channel_id != self.admin_channel {
            return;
        }
        let user_id = msg.author.id.0 as i64;
        if !self.state.is_admin_user(user_id) {
            return;
        }
        let text = Some(msg.content).filter(|s| !s.is_empty());
        let (file_id, file_name) = match msg.attachments.first() {
            Some(a) => (Some(a.url.to_string()), Some(a.filename.clone())),
            None => (None, None),
        };
        let event = BotEvent::Message(MessageEvent {
            adapter: self.adapter.clone(),
            target: aegis::adapters::common::TargetId(self.admin_channel.to_string()),
            user_id,
            text,
            file_id,
            file_name,
            reply_to_text: None,
        });
        let _ = dispatch_event(event, &self.state).await;
    }

    async fn interaction_create(&self, ctx: SerenityCtx, interaction: Interaction) {
        let _ = match interaction {
            Interaction::Command(ref cmd) => {
                let _ = cmd
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::DeferredChannelMessageWithSource,
                    )
                    .await;
                let name = cmd.data.name.as_str();
                let code = cmd.data.options.first().and_then(|opt| match &opt.value {
                    CommandDataOptionValue::String(s) => Some(s.as_str()),
                    _ => None,
                });
                if let Some(command) = parse_slash(name, code) {
                    let event = BotEvent::Command(CommandEvent {
                        adapter: self.adapter.clone(),
                        target: aegis::adapters::common::TargetId(
                            cmd.channel_id.to_string(),
                        ),
                        user_id: cmd.user.id.0 as i64,
                        command,
                    });
                    dispatch_event(event, &self.state).await
                } else {
                    Ok(())
                }
            }
            Interaction::Component(ref comp) => {
                let _ = comp
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::DeferredChannelMessageWithSource,
                    )
                    .await;
                if let Some(ref msg) = comp.message {
                    let event = BotEvent::Callback(CallbackEvent {
                        adapter: self.adapter.clone(),
                        target: aegis::adapters::common::TargetId(
                            msg.channel_id.to_string(),
                        ),
                        user_id: comp.user.id.0.to_string(),
                        msg_id: aegis::adapters::common::MessageId(msg.id.to_string()),
                        data: comp.data.custom_id.clone().unwrap_or_default(),
                        callback_id: String::new(),
                        session_timeout_secs: self.state.session_timeout_secs().await,
                    });
                    dispatch_event(event, &self.state).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis::shared::types::BotCommand;

    fn make_enc() -> EncryptedConfig {
        EncryptedConfig {
            token: vec![], admin_id: vec![], totp_secret: vec![],
            self_destruct_key_hash: None,
            matrix_homeserver: None, matrix_username: None,
            matrix_password: None, matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None, discord_admin_id: None,
            lang: None,
        }
    }

    #[test]
    fn parse_slash_known_commands() {
        assert_eq!(parse_slash("help", None), Some(BotCommand::Help));
        assert_eq!(parse_slash("start", None), Some(BotCommand::Start));
        assert_eq!(parse_slash("menu", None), Some(BotCommand::Menu));
        assert_eq!(
            parse_slash("auth", Some("123456")),
            Some(BotCommand::Auth { code: "123456".into() })
        );
        assert_eq!(
            parse_slash("setsecurityfile", None),
            Some(BotCommand::SetSecurityFile)
        );
    }

    #[test]
    fn parse_slash_unknown_returns_none() {
        assert_eq!(parse_slash("unknown", None), None);
        assert_eq!(parse_slash("auth", None), None);
    }

    #[test]
    fn has_discord_config_by_flag() {
        let enc = make_enc();
        assert!(has_discord_config(&enc, &["--discord".to_string()]));
        assert!(has_discord_config(&enc, &["--all".to_string()]));
    }

    #[test]
    fn has_discord_config_by_fields() {
        let mut enc = make_enc();
        enc.discord_token = Some(vec![1]);
        enc.discord_admin_id = Some(vec![2]);
        assert!(has_discord_config(&enc, &[]));
    }

    #[test]
    fn has_discord_config_false_when_missing() {
        let enc = make_enc();
        assert!(!has_discord_config(&enc, &[]));
    }
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test parse_slash has_discord_config -v
```
Expected: PASS (tests run, don't need network)

- [ ] **Step 7: Commit**

```bash
git add src/main/discord.rs src/main/mod.rs
git commit -m "feat(aegis): add main/discord.rs — DiscordHandler, connect, slash commands"
```

---

### Task 5: Wire main.rs + runtime.rs

**Files:**
- Modify: `src/main.rs` — CLI `--discord`, connect, adapter selection
- Modify: `src/main/runtime.rs` — Discord branch: build_handle → spawn client.start()

**Interfaces:**
- Consumes: `DiscordRawHandle`, `DiscordHandle`, `has_discord_config()`, `connect_discord()`, `build_handle()` from Task 4
- Produces: running Discord gateway (via `client.start()` in a spawned task)

- [ ] **Step 1: Update `src/main.rs`**

Goal: add `--discord` flag (no auto-detect; Discord standalone, not part of `--all`), `connect_discord()`, adapter selection, pass to runtime.

After existing flag detection (line 41-44):
```rust
let use_discord = args.iter().any(|a| a == "--discord");
let enable_discord = use_discord;
// Discord is standalone: --discord flag is required, no auto-detect.
// --discord disables telegram AND matrix; --all does NOT include discord.
let mut enable_matrix = use_matrix || use_all;
let enable_telegram = (!use_matrix && !use_discord) || use_all;
if enable_discord {
    enable_matrix = false;
}
```

After matrix_handle block (line 54-65), add discord_raw (--discord only, no auto-detect):
```rust
let discord_raw = if enable_discord {
    Some(
        main::discord::connect_discord(
            &security,
            &app_config.decrypted.encrypted_config,
            &config_dir(),
        )
        .await?,
    )
} else {
    None
};
```

Replace the adapter construction block (line 67-73). Discord adapter comes from the raw handle instead of `build_adapter`:
```rust
let adapter = if let Some(ref raw) = discord_raw {
    raw.adapter.clone()
} else {
    main::adapter::build_adapter(
        &app_config.decrypted.token,
        enable_telegram,
        enable_matrix,
        &matrix_handle,
    )
    .await?
};
```

State construction (line 75-86): pass the Discord admin_id:
```rust
let state = Arc::new(AppState::new(
    app_config.decrypted.admin_id,
    app_config.totp_manager,
    production_executor(),
    app_config.decrypted.encrypted_config.self_destruct_key_hash.clone(),
    app_config.bot_settings.session_timeout_secs,
    adapter,
    discord_raw.as_ref().and_then(|r| {
        // We don't persist raw admin_id; lookup from encrypted config later
        None
    }),
));
```

Wait — we need the discord_admin_id as i64 here. The `connect_discord` already decrypts it but doesn't return the parsed admin_id. Let me add it to DiscordRawHandle.

Let me update Task 4's `DiscordRawHandle` to include `admin_id: u64`:

```rust
pub struct DiscordRawHandle {
    pub token: String,
    pub http: Arc<Http>,
    pub admin_channel: ChannelId,
    pub adapter: Arc<dyn BotAdapter>,
    pub admin_id: u64,
}
```

And in `connect_discord`:
```rust
Ok(DiscordRawHandle {
    token: discord_token,
    http,
    admin_channel,
    adapter,
    admin_id: discord_admin_id,
})
```

Now in main.rs state construction:
```rust
let state = Arc::new(AppState::new(
    app_config.decrypted.admin_id,
    app_config.totp_manager,
    production_executor(),
    app_config.decrypted.encrypted_config.self_destruct_key_hash.clone(),
    app_config.bot_settings.session_timeout_secs,
    adapter,
    discord_raw.as_ref().map(|r| r.admin_id as i64),
));
```

Then pass to runtime:
```rust
main::runtime::run(
    state,
    matrix_handle,
    enable_telegram,
    enable_matrix,
    discord_raw,
    app_config.decrypted.token,
    app_config.decrypted.admin_id,
)
.await
```

- [ ] **Step 2: Update `run()` signature in `runtime.rs`**

```rust
pub async fn run(
    state: Arc<AppState>,
    matrix_handle: Option<super::matrix::MatrixHandle>,
    enable_telegram: bool,
    enable_matrix: bool,
    discord_raw: Option<super::discord::DiscordRawHandle>,
    token: String,
    admin_id: i64,
) -> Result<(), anyhow::Error>
```

- [ ] **Step 3: Add Discord branch in `runtime.rs` (after Matrix sync, before Telegram Dispatcher)**

```rust
// ── Discord gateway ──
if let Some(raw) = discord_raw {
    let adapter_for_init = raw.adapter.clone();
    let target_for_init = TargetId(raw.admin_channel.to_string());

    tokio::spawn(async move {
        if let Err(e) = aegis::core::system::scheduler::start_scheduler(
            adapter_for_init.clone(),
            target_for_init.clone(),
        )
        .await
        {
            log::error!("❌ 初始化调度器失败: {}", e);
        }
        tokio::join!(
            async { let _ = crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await; },
            async { let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init).await; },
            async { let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await; },
        );
    });

    let (mut client, _, _) =
        super::discord::build_handle(raw, state.clone())
            .await
            .context("构建 Discord 客户端失败")?;

    // Discord-only: keep process alive via CancellationToken
    if !enable_telegram && !enable_matrix {
        let token = tokio_util::sync::CancellationToken::new();
        let token_clone = token.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            log::info!("收到关闭信号，正在优雅关闭...");
            token.cancel();
        });
        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                log::error!("Discord 网关错误: {}", e);
            }
        });
        token_clone.cancelled().await;
    } else {
        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                log::error!("Discord 网关错误: {}", e);
            }
        });
    }
}
```

Discord-only: spawn scheduler + notify + CancellationToken (same pattern as Matrix-only lines 300-339). Combined (Telegram + Discord): just spawn client.start() in background, Telegram's `Dispatcher::dispatch()` keeps the process alive.

- [ ] **Step 4: Fix callers of `run()` in main.rs**

Already handled in Step 1 — pass `discord_raw` and the new params.

- [ ] **Step 5: Build and run tests**

```bash
cargo build 2>&1 | grep -E "error|warning"
```
Fix any compilation errors. Discord-related code doesn't require a real Discord token to compile.

```bash
cargo test 2>&1 | tail -20
```
Expected: all green (existing tests + new discord tests)

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/main/runtime.rs
git commit -m "feat(aegis): wire Discord platform — CLI, runtime, gateway"
```

---

## Verification Gate

After all tasks:

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Must all pass before finishing-a-development-branch.
