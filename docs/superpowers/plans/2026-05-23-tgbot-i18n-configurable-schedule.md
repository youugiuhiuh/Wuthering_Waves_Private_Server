# tgbot i18n + Configurable Schedule Implementation Plan

> **For agentic workers:** Use subagent-driven-development or executing-plans to implement task-by-task.

**Goal:** Add i18n (zh-CN/en/ja), language switch menu, auto timezone, and configurable default update schedule to tgbot.

**Architecture:** `rust-i18n` with YAML files, `t!()` macro for runtime lookup. `AppState` holds a Mutex-protected `language` field persisted via `BotSettings`. `SchedulerState::default()` replaced by `from_settings()`.

**Tech Stack:** Rust, teloxide 0.13, rust-i18n 4, tokio-cron-scheduler, chrono-tz, serde_json

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| create | `src/i18n/zh-CN.yml` | Chinese translations (fallback) |
| create | `src/i18n/en.yml` | English translations |
| create | `src/i18n/ja.yml` | Japanese translations |
| modify | `Cargo.toml` | add `rust-i18n = "4"` |
| modify | `src/lib.rs` | add i18n init macro |
| modify | `src/bootstrap.rs` | extend BotSettings |
| modify | `src/app/state.rs` | add language field + methods |
| modify | `src/utils/mod.rs` | i18n-aware format_duration_human |
| modify | `src/logic/system/scheduler/mod.rs` | from_settings(), build_default_cron() |
| modify | `src/logic/system/scheduler/task_types.rs` | get_display_name() i18n |
| modify | `src/handlers/menu.rs` | language/schedule buttons, all t!() |
| modify | `src/handlers/schedule.rs` | all t!() |
| modify | `src/handlers/callback.rs` | all t!() |
| modify | `src/handlers/mod.rs` | route new callbacks |
| modify | `src/handlers/ops.rs` | all t!() |
| modify | `src/handlers/log.rs` | all t!() |
| modify | `src/handlers/message.rs` | all t!() |
| modify | `src/app/auth.rs` | all t!() |
| modify | `src/app/destruct_flow.rs` | all t!() |
| modify | `src/main.rs` | all t!() |

---

## Tasks

### Task 1: Add rust-i18n dependency

**File:** modify `Cargo.toml`

- [ ] Add `rust-i18n = "4"` to dependencies

### Task 2: Create i18n YAML files

**Files:** create `src/i18n/zh-CN.yml`, `src/i18n/en.yml`, `src/i18n/ja.yml`

Each file contains keys for: menu, auth, session, settings, language, default_schedule, schedule, ops, log, destruct, users, danger, monitor, xray, singbox, geo, file, startup, commands, duration, message, misc

### Task 3: Initialize rust-i18n in lib.rs

**File:** modify `src/lib.rs`

Add `rust_i18n::i18n_cfg!(path = "src/i18n", fallback = "zh-CN");`

### Task 4: Extend BotSettings with new fields

**File:** modify `src/bootstrap.rs`

Add to `BotSettings`:
- `language: String` (default "zh-CN")
- `default_schedule_hour: u8` (default 4)
- `default_schedule_minute: u8` (default 0)
- `default_schedule_frequency: String` (default "weekly")
- `default_schedule_day_of_week: Option<String>` (default Some("Sun"))
- `default_schedule_timezone: String` (default "UTC")

All with `#[serde(default)]`. Add `timezone_for_language(lang) -> &'static str` helper.

### Task 5: Add language field to AppState

**File:** modify `src/app/state.rs`

Add `language: Mutex<String>` field. Add `language()` and `set_language()` async methods. Update `new()` signature. Update all test helpers.

### Task 6: Add build_default_cron and from_settings

**File:** modify `src/logic/system/scheduler/mod.rs`

```rust
pub fn build_default_cron(hour: u8, minute: u8, frequency: &str, day_of_week: Option<&str>) -> String

impl SchedulerState {
    pub fn from_settings(settings: &BotSettings) -> Self
}
```

### Task 7: Add language switch handler to menu.rs

**File:** modify `src/handlers/menu.rs`

Add `m_language` button to settings. Add `m_language` handler showing lang picker. Add `set_language:xx` handler that updates AppState + BotSettings + saves. Migrate all hardcoded strings to `t!()`.

### Task 8: Add default schedule config UI

**Files:** modify `src/handlers/menu.rs`, `src/handlers/mod.rs`, `src/logic/system/scheduler/mod.rs`

Add `m_default_schedule` button to settings. Add `ds_freq:daily/weekly`, `ds_hour:N`, `ds_minute:N`, `ds_tz:tz` handlers. Route in mod.rs. Update scheduler to use `from_settings()` when loading.

### Task 9: Migrate schedule.rs to i18n

**File:** modify `src/handlers/schedule.rs`

Replace all hardcoded strings with `t!()` calls. Update label functions to use `t!()` with language parameter.

### Task 10: Migrate callback.rs, auth.rs, destruct_flow.rs to i18n

**Files:** modify `src/handlers/callback.rs`, `src/app/auth.rs`, `src/app/destruct_flow.rs`

### Task 11: Migrate task_types.rs get_display_name to i18n

**File:** modify `src/logic/system/scheduler/task_types.rs`

Change signature to `get_display_name(&self, lang: &str) -> String`. Use `t!()` internally.

### Task 12: Migrate ops.rs, log.rs, message.rs to i18n

**Files:** modify `src/handlers/ops.rs`, `src/handlers/log.rs`, `src/handlers/message.rs`

### Task 13: Migrate main.rs and format_duration_human to i18n

**Files:** modify `src/main.rs`, `src/utils/mod.rs`

Update `format_duration_human(secs, lang)` to use `t!()`. Update all call sites.

### Task 14: Migrate warp.rs, singbox.rs, xray.rs, monitor.rs

**Files:** modify `src/handlers/warp.rs`, `src/handlers/singbox.rs`, `src/handlers/xray.rs`, `src/logic/system/monitor.rs`

### Task 15: Final integration, test, verify

- [ ] cargo check
- [ ] cargo test
- [ ] cargo clippy (fix errors)
- [ ] Verify all 3 YAML files have identical key sets
- [ ] Final commit

---

## Key Signatures (for reference)

```rust
// AppState::new - language param added
pub fn new(admin_id: i64, totp_manager: TotpManager, self_destruct_executor: Arc<dyn SelfDestructExecutor>, self_destruct_key_hash: Option<String>, session_timeout_secs: u64, language: String) -> Self

// get_display_name - now takes lang
pub fn get_display_name(&self, lang: &str) -> String

// format_duration_human - now takes lang
pub fn format_duration_human(secs: u64, lang: &str) -> String

// build_default_cron
pub fn build_default_cron(hour: u8, minute: u8, frequency: &str, day_of_week: Option<&str>) -> String

// SchedulerState::from_settings
pub fn from_settings(settings: &BotSettings) -> Self
```

## Backward Compatibility

- `serde(default)` on all new BotSettings fields
- Existing scheduler_state.json files load normally
- Old JSON configs without new fields get defaults (zh-CN, weekly Sun 04:00 UTC)