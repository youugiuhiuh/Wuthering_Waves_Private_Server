# Tgbot Handler Registry Refactor Design

**Date**: 2026-05-05
**Status**: Approved
**Approach**: Handler Registry pattern (Option A)
**Scope**: Pure internal refactor — no external behavior changes
**Style**: Incremental — each step compiles and passes tests
**Risk**: Low — test-first methodology

---

## Problem Statement

`rust/tgbot` has accumulated significant architectural debt:

1. **5193-line main.rs** — All Telegram bot interaction logic in a single `handle_callback` function with ~70+ string-match branches (`d.starts_with("u_kcp_more:")`, etc.)
2. **No separation of concerns** — UI rendering, business logic orchestration, and inline keyboard construction intermixed
3. **Stringly-typed callback routing** — Fragile, no compile-time safety, easy to introduce typos
4. **ConfigManager god module** (2874 lines) — Config generation, WARP routing, PQ keys, batch creation, KCP masks all in one file
5. **Bidirectional coupling** between `config` ↔ `maintenance`
6. **Layer violation** — `core/utils` depends on `logic/maintenance`
7. **No dependency injection** — Managers are unit structs with associated functions + `Lazy` statics = global mutable state

---

## Design

### Section 1: Handler Registry & Typed Callback Routing

#### 1.1 CallbackAction Enum

Replace all string-matched callback data with a type-safe enum:

```rust
// src/router/callback_action.rs
#[derive(Debug, Clone, PartialEq)]
pub enum CallbackAction {
    // Main menu
    MainMenu,
    Monitor,
    // User management
    BatchInit,
    BatchCreate(IpVersion),
    KcpMore(KcpMask),
    KcpSelect(KcpMask),
    // Warp
    WarpSwitchMode(WarpMode),
    WarpStatus,
    // Singbox
    SbH2Exec(String),
    SbTuicExec(String),
    SbH2Status,
    SbTuicStatus,
    // Auth
    AuthVerify,
    // Self-destruct
    DestructStart,
    DestructConfirm,
    DestructCancel,
    // Upgrade
    UpgradeCheck,
    UpgradeApply,
    // Scheduler
    ScheduleList,
    ScheduleAdd(ScheduleTaskType),
    // ... all existing callback data strings mapped here
}
```

Implement `FromStr` for parsing Telegram callback data strings into `CallbackAction`, and `Display` for generating callback data strings from `CallbackAction`. This ensures:
- Type-safe routing at the dispatch layer
- Backward compatibility with existing callback data format
- Single source of truth for all callback actions

#### 1.2 Handler Trait & Registry

```rust
// src/router/mod.rs
#[async_trait]
pub trait Handler: Send + Sync {
    async fn handle(&self, ctx: &AppContext, msg: &Message, action: &CallbackAction) -> Result<()>;
}

pub struct HandlerRegistry {
    handlers: HashMap<CallbackAction, Box<dyn Handler>>,
}

impl HandlerRegistry {
    pub fn register(&mut self, action: CallbackAction, handler: Box<dyn Handler>) { ... }
    pub async fn dispatch(&self, ctx: &AppContext, msg: &Message, action: &CallbackAction) -> Result<()> { ... }
}
```

#### 1.3 Directory Structure

```
src/
  router/               # CallbackAction, Handler trait, Registry
    mod.rs
    callback_action.rs
  handlers/              # Each handler in its own file
    main_menu.rs
    monitor.rs
    batch.rs
    kcp.rs
    warp.rs
    singbox_h2.rs
    singbox_tuic.rs
    upgrade.rs
    auth.rs
    destruct.rs
    scheduler.rs
    ...
```

#### 1.4 Migration Strategy

Extract handlers one at a time from main.rs. After each extraction:
1. Move the match arm logic into a handler file
2. Register it in the registry
3. Replace the original match arm with a registry dispatch call
4. Verify: `cargo check` + tests pass + bot behaves identically

---

### Section 2: Core Layer Fix & Module Decoupling

#### 2a. core/utils Layer Violation Fix

Move `select_available_port()` from `core/utils.rs` to `logic/port_allocator.rs`. This function logically belongs with port allocation, and its dependency on `MaintenanceManager::is_port_available()` creates an illegal downward dependency from core to logic.

After this move, `core/utils.rs` contains only pure utility functions with no logic-layer dependencies:
- `generate_random_suffix()`
- `generate_timestamp_filename()`
- `parse_ip_version()`

#### 2b. config <-> maintenance Decoupling via Traits

Introduce two trait abstractions:

```rust
// src/logic/service_lifecycle.rs
#[async_trait]
pub trait ServiceLifecycle: Send + Sync {
    async fn reload_core(&self) -> Result<()>;
    async fn restart_service(&self, name: &str) -> Result<()>;
    async fn stop_service(&self, name: &str) -> Result<()>;
}

// src/logic/config_provider.rs
#[async_trait]
pub trait ConfigProvider: Send + Sync {
    async fn ensure_base_config(&self) -> Result<()>;
}
```

- `ConfigManager` depends on `Arc<dyn ServiceLifecycle>` instead of `MaintenanceManager` directly
- `MaintenanceManager` depends on `Arc<dyn ConfigProvider>` instead of `ConfigManager` directly
- Concrete implementations injected in `AppContext` assembly

This eliminates the bidirectional compile-time dependency while preserving all runtime behavior.

#### 2c. ConfigManager Split

Split `logic/config.rs` (2874 lines) into focused sub-modules:

```
src/logic/config/
  mod.rs           -- ConfigContext (shared state), ConfigProvider impl, re-exports
  vision.rs        -- Vision protocol config generation
  xhttp.rs         -- XHTTP protocol config generation
  kcp.rs           -- KCP protocol config generation + KcpMask enum
  warp.rs           -- WARP routing rule management
  batch.rs          -- Batch account creation logic
  pq_keys.rs        -- ML-DSA-65 post-quantum Reality key management
  link_gen.rs       -- Client link generation (vless:// URLs)
```

`ConfigContext` holds shared runtime state:
```rust
pub struct ConfigContext {
    pub pq_seed: Vec<u8>,
    pub pq_verify: Vec<u8>,
    pub sni_selector: Arc<SNISelector>,
    pub port_allocator: Arc<PortAllocator>,
    pub tls_probe: Arc<TlsProbe>,
    // ... other shared dependencies
}
```

Each sub-module's public functions accept `&ConfigContext` as a parameter rather than accessing global `Lazy` statics.

#### 2d. Lazy Statics -> AppContext Injection

Replace global `Lazy<Mutex<...>>` state with explicit injection through an `AppContext` container:

```rust
pub struct AppContext {
    pub config: Arc<dyn ConfigProvider>,
    pub service: Arc<dyn ServiceLifecycle>,
    pub system: Arc<SystemMonitor>,
    pub scheduler: Arc<SchedulerManager>,
    pub upgrade: Arc<UpgradeManager>,
    pub security: Arc<SecurityManager>,
    pub firewall: Arc<FirewallManager>,
    pub geoip: Arc<GeoIPService>,
    // ...
    pub bot: Arc<Bot>,
    pub state: Arc<AppState>,
}
```

Migration strategy:
1. Convert each Manager from unit struct with associated functions to an instantiated struct with state
2. Keep `Lazy` statics as temporary backward compatibility during transition
3. Replace call sites one at a time to use `AppContext` references
4. Remove `Lazy` statics once all call sites migrated
5. Verify at each step: `cargo check` + tests pass

---

### Section 3: Migration Phases & Verification

#### Phase 0: Test Infrastructure (Prerequisite)

- Add integration tests for main.rs critical callback branches (capture callback data strings, verify message/keyboard output)
- Add unit tests for ConfigManager core functions (config generation, link generation)
- Add mock-based tests for MaintenanceManager core methods
- Goal: establish safety net before any refactoring begins

#### Phase 1: Typed Callback Routing Layer

- 1.1 Define `CallbackAction` enum + `FromStr`/`Display` impl
- 1.2 Define `Handler` trait + `HandlerRegistry`
- 1.3 Extract first handler from main.rs (e.g., `MainMenu`)
- 1.4 Replace corresponding match arm with registry dispatch
- 1.5 Verify: bot works identically, tests pass
- 1.6 Repeat for each handler until main.rs is cleaned up

#### Phase 2: Core Layer Fix

- 2.1 Move `select_available_port()` from `core/utils` to `logic/port_allocator`
- 2.2 Clean remaining logic-layer dependencies from `core/utils`
- 2.3 Verify: compiles, existing tests pass

#### Phase 3: Trait Decoupling

- 3.1 Define `ServiceLifecycle` trait and `ConfigProvider` trait
- 3.2 Implement traits for `MaintenanceManager` and `ConfigManager`
- 3.3 Replace `ConfigManager`'s direct calls to `MaintenanceManager` with trait calls
- 3.4 Replace `MaintenanceManager`'s direct calls to `ConfigManager` with trait calls
- 3.5 Verify: compiles, integration tests pass

#### Phase 4: ConfigManager Split

- 4.1 Create `logic/config/` directory structure + `ConfigContext` struct
- 4.2 Extract `VisionConfig` generation to `vision.rs`
- 4.3 Extract `XhttpConfig` generation to `xhttp.rs`
- 4.4 Extract `KcpConfig + KcpMask` to `kcp.rs`
- 4.5 Extract WARP routing to `warp.rs`
- 4.6 Extract batch creation to `batch.rs`
- 4.7 Extract PQ keys to `pq_keys.rs`
- 4.8 Extract link generation to `link_gen.rs`
- 4.9 Update all call sites
- 4.10 Verify: each extraction confirmed by passing tests

#### Phase 5: AppContext Dependency Injection

- 5.1 Define `AppContext` struct
- 5.2 Convert Managers from unit structs + Lazy statics to instantiated structs
- 5.3 Assemble `AppContext` in `main()`
- 5.4 Inject into Handler's `handle()` method parameter
- 5.5 Remove remaining Lazy statics
- 5.6 Verify: compiles, all tests pass

#### Verification Strategy

- Run full test suite after each phase
- Ensure `cargo check` passes after each sub-step
- Phase 0 must complete before other phases begin
- Each phase can be committed independently, keeping mainline always runnable
- Integration tests verify bot behavior remains unchanged

---

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Refactoring approach | Handler Registry pattern | Best fit for incremental, test-safe migration |
| Callback routing | Typed enum + FromStr/Display | Type safety while preserving wire format |
| Config-maintenance coupling | Trait abstraction | Breaks compile-time cycle, preserves runtime behavior |
| ConfigManager split | By proto & function domain | Natural domain boundaries, each sub-module is cohesive |
| DI approach | AppContext struct | Simple, explicit, no framework overhead |
| Migration style | Incremental per-handler | Each extraction independently verifiable |
| Test strategy | Phase 0 tests first | Safety net before any structural changes |
| Bot framework | Keep teloxide | No reason to change; architecture improvements are framework-agnostic |

## Out of Scope

- i18n / localization of Chinese UI strings (separate effort)
- New features or behavior changes (pure internal refactor)
- Database or persistence layer changes
- Telegram Bot API version upgrade
- Performance optimization (not a goal of this refactor)