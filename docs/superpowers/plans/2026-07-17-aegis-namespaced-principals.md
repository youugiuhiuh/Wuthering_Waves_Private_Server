# Aegis Namespaced Principals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace numeric cross-platform identities with validated, namespaced principals and perform one fail-closed startup migration.

**Architecture:** `Principal` is created at adapter boundaries and carried unchanged by events. `AppState` uses principals for authorization and authentication state. Startup migrates legacy encrypted administrator fields atomically before constructing `AppState`.

**Tech Stack:** Rust 2024, serde, Tokio, existing AES-GCM configuration storage.

## Global Constraints

- Telegram and Discord subjects are canonical unsigned decimal strings.
- Matrix subjects are complete MXIDs including homeserver.
- Invalid identity never maps to zero or another fallback identity.
- Ambiguous legacy migration stops startup and preserves the old file.
- Sessions and failed-attempt state are not migrated.
- No dual-format runtime compatibility layer and no new dependency.

---

### Task 1: Validated Principal Type

**Files:**
- Modify: `rust/aegis/src/adapters/common/trait.rs`
- Modify: `rust/aegis/src/shared/types.rs`

**Interfaces:**
- Produces: `Principal::new(Platform, impl Into<String>) -> Result<Principal>`
- Produces: `Principal::telegram(u64)`, `Principal::discord(u64)`, `Principal::matrix(&str)`
- Produces: `BotEvent::principal() -> &Principal`

- [ ] **Step 1: Add failing validation and isolation tests**

```rust
#[test]
fn principals_include_platform_namespace() {
    assert_ne!(Principal::telegram(42), Principal::discord(42));
    assert_ne!(Principal::matrix("@admin:a.example").unwrap(), Principal::matrix("@admin:b.example").unwrap());
}

#[test]
fn principal_rejects_noncanonical_subjects() {
    assert!(Principal::new(Platform::Telegram, "0042").is_err());
    assert!(Principal::new(Platform::Discord, " 42").is_err());
    assert!(Principal::matrix("admin").is_err());
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test adapters::common:: --all-features`
Expected: FAIL because `Principal` does not exist.

- [ ] **Step 3: Implement the minimal validated type**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform { Telegram, Discord, Matrix }

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Principal { pub platform: Platform, pub subject: String }

impl Principal {
    pub fn new(platform: Platform, subject: impl Into<String>) -> Result<Self> {
        let subject = subject.into();
        let valid = match platform {
            Platform::Telegram | Platform::Discord => subject.parse::<u64>().map(|n| n.to_string() == subject).unwrap_or(false),
            Platform::Matrix => {
                !subject.chars().any(char::is_whitespace)
                    && subject.starts_with('@')
                    && subject.split_once(':').is_some_and(|(u, h)| {
                        u.len() > 1 && !h.is_empty() && !h.contains(':')
                    })
            }
        };
        if !valid { anyhow::bail!("invalid principal subject for platform"); }
        Ok(Self { platform, subject })
    }
    pub fn telegram(id: u64) -> Self { Self { platform: Platform::Telegram, subject: id.to_string() } }
    pub fn discord(id: u64) -> Self { Self { platform: Platform::Discord, subject: id.to_string() } }
    pub fn matrix(id: &str) -> Result<Self> { Self::new(Platform::Matrix, id) }
}
```

- [ ] **Step 4: Replace event `user_id` fields with `principal` and remove parsing fallback**

```rust
pub struct CallbackEvent { pub principal: Principal, /* existing fields unchanged */ }
pub struct MessageEvent { pub principal: Principal, /* existing fields unchanged */ }
pub struct CommandEvent { pub principal: Principal, /* existing fields unchanged */ }

impl BotEvent {
    pub fn principal(&self) -> &Principal {
        match self { Self::Message(e) => &e.principal, Self::Callback(e) => &e.principal, Self::Command(e) => &e.principal }
    }
}
```

- [ ] **Step 5: Verify GREEN and commit**

Run: `cargo test adapters::common:: shared::types:: --all-features`
Expected: PASS.

```bash
git add rust/aegis/src/adapters/common/trait.rs rust/aegis/src/shared/types.rs
git commit -m "security: add namespaced principals"
```

### Task 2: Adapter-Boundary Principal Construction

**Files:**
- Modify: `rust/aegis/src/main/runtime.rs` (Telegram events)
- Modify: `rust/aegis/src/main/discord.rs` (Discord events)
- Modify: `rust/aegis/src/adapters/matrix/adapter.rs`
- Modify: `rust/aegis/src/adapters/matrix/commands.rs` (Matrix events)

**Interfaces:**
- Consumes: `Principal` from Task 1.
- Produces: every inbound event with an already validated `principal`.

- [ ] **Step 1: Add adapter tests for Telegram, Discord, and full Matrix MXID conversion**
- [ ] **Step 2: Verify RED with `cargo test adapters:: --all-features`**
- [ ] **Step 3: Replace each event construction with exact platform conversion**

```rust
let principal = Principal::telegram(message.from.id.0);
let principal = Principal::discord(user_id.get());
let principal = Principal::matrix(sender.as_str()).context("invalid Matrix sender MXID")?;
```

- [ ] **Step 4: Remove all event-boundary `parse::<i64>()`, `unwrap_or(0)`, and Matrix localpart extraction**
- [ ] **Step 5: Run `cargo test adapters:: shared:: --all-features` and commit**

```bash
git add rust/aegis/src/adapters rust/aegis/src/main rust/aegis/src/shared/types.rs
git commit -m "security: bind events to platform principals"
```

### Task 3: Principal-Keyed App State

**Files:**
- Modify: `rust/aegis/src/app/state.rs`
- Modify: `rust/aegis/src/app/auth.rs`
- Modify: `rust/aegis/src/shared/commands.rs`
- Modify: `rust/aegis/src/shared/dispatch.rs`
- Modify: `rust/aegis/src/shared/boundary.rs`
- Modify: `rust/aegis/src/shared/state_ops.rs`
- Modify: `rust/aegis/src/shared/destruct.rs`

**Interfaces:**
- `AppState::new(administrators: HashSet<Principal>, ...)`
- `is_admin_user(&Principal)`, `is_authorized(&Principal)`, `record_auth_success(Principal, Instant)`

- [ ] **Step 1: Add tests proving same numeric subject has isolated sessions and failed attempts**
- [ ] **Step 2: Verify RED with `cargo test app::state:: --all-features`**
- [ ] **Step 3: Change administrator and authentication maps**

```rust
administrators: HashSet<Principal>,
sessions: Mutex<HashMap<Principal, Instant>>,
failed_attempts: Mutex<HashMap<Principal, FailedRecord>>,
```

- [ ] **Step 4: Pass `event.principal()` through every auth and dispatch call; clone only when inserting map keys**
- [ ] **Step 5: Run `cargo test app:: shared:: --all-features` and commit**

```bash
git add rust/aegis/src/app rust/aegis/src/shared
git commit -m "security: key authentication state by principal"
```

### Task 4: Fail-Closed Startup Migration

**Files:**
- Modify: `rust/aegis/src/bootstrap.rs`
- Modify: `rust/aegis/src/main/config.rs`
- Modify: `rust/aegis/src/main.rs`
- Modify: `rust/aegis/src/main/discord.rs`
- Modify: `rust/aegis/src/main/matrix.rs`
- Test: tests in the same modules

**Interfaces:**
- Produces encrypted `administrators: Vec<Principal>` as the only normal runtime source.
- Produces `migrate_legacy_administrators(path, security, config) -> Result<EncryptedConfig>`.

- [ ] **Step 1: Add fixtures for unique Telegram, explicit Discord, full Matrix, ambiguous, and invalid migration**
- [ ] **Step 2: Assert failed migration leaves original bytes unchanged; verify RED**
- [ ] **Step 3: Add optional legacy fields only to the deserialization migration shape and a required new administrator field**
- [ ] **Step 4: Encrypt and atomically persist migrated configuration before returning it; then construct `AppState` from principals**
- [ ] **Step 5: Delete normal-runtime reads of legacy administrator fields and run full gates**

Run: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`
Expected: PASS.

- [ ] **Step 6: Update only the principal-collision audit finding and commit**

```bash
git add rust/aegis/src/bootstrap.rs rust/aegis/src/main rust/aegis/src/app rust/aegis/src/shared rust/aegis/src/adapters docs/audits/2026-07-17-aegis-rust-stability-performance-security-audit.md
git commit -m "security: migrate administrators to namespaced principals"
```
