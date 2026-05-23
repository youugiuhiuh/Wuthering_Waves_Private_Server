# tgbot i18n + Configurable Default Schedule Design

## 1. Overview

Add three core features to `rust/tgbot`:
1. **i18n** — all user-facing text in zh-CN / en / ja via `rust-i18n`
2. **Language switch button** — in settings menu, auto-switches timezone on language change
3. **Fully configurable default update schedule** — replaces hardcoded `0 4 * * 0` UTC Sunday 4:00

## 2. Dependencies

**New in `Cargo.toml`:**
```toml
rust-i18n = "4"
```

## 3. i18n Architecture

### 3.1 Translation Files

```
src/i18n/
├── zh-CN.yml   # Chinese (default/fallback)
├── en.yml      # English
└── ja.yml      # Japanese
```

All user-facing strings (UI labels, menu text, error messages) live in YAML. No hardcoded Chinese in user-visible strings.

### 3.2 Init

In `src/lib.rs`:
```rust
rust_i18n::i18n_cfg!(path = "src/i18n", fallback = "zh-CN");
```

### 3.3 AppState Language Field

```rust
// src/app/state.rs
pub struct AppState {
    // ...
    language: Mutex<String>,  // "zh-CN" | "en" | "ja"
}
```

Methods: `language() -> String`, `set_language(&str)`.

### 3.4 BotSettings Extension

```rust
// src/bootstrap.rs
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct BotSettings {
    pub session_timeout_secs: u64,           // existing
    pub language: String,                    // NEW
    pub default_schedule_hour: u8,           // NEW (default 4)
    pub default_schedule_minute: u8,         // NEW (default 0)
    pub default_schedule_frequency: String,  // NEW "daily"|"weekly" (default "weekly")
    pub default_schedule_day_of_week: Option<String>,  // NEW e.g. "Sun"
    pub default_schedule_timezone: String,   // NEW (default "UTC")
}
```

All new fields use `#[serde(default)]` for backward compatibility with existing JSON configs.

## 4. Language Switch + Timezone Linkage

### 4.1 Settings Menu

Add `"🌐 语言 / Language"` button (`m_language`) to `m_settings` menu.

### 4.2 Language Selection UI

`m_language` shows current language with 3 options (zh-CN/en/ja), each marking current with ✅.

### 4.3 Language → Timezone Auto-Switch

| Language | Timezone |
|----------|----------|
| zh-CN | Asia/Shanghai |
| en | UTC |
| ja | Asia/Tokyo |

Changing language updates `BotSettings.language` and `BotSettings.default_schedule_timezone`, then saves.

## 5. Fully Configurable Default Schedule

### 5.1 BotSettings Schedule Fields

| Field | Type | Default |
|-------|------|---------|
| `default_schedule_hour` | u8 | 4 |
| `default_schedule_minute` | u8 | 0 |
| `default_schedule_frequency` | String | "weekly" |
| `default_schedule_day_of_week` | Option\<String\> | Some("Sun") |
| `default_schedule_timezone` | String | "UTC" |

### 5.2 SchedulerState::from_settings()

Replace `SchedulerState::default()` (hardcoded `0 4 * * 0`) with:

```rust
impl SchedulerState {
    pub fn from_settings(settings: &BotSettings) -> Self {
        let cron = build_default_cron(
            settings.default_schedule_hour,
            settings.default_schedule_minute,
            &settings.default_schedule_frequency,
            settings.default_schedule_day_of_week.as_deref(),
        );
        Self {
            tasks: vec![ScheduledTask::new_with_timezone(
                TaskType::GeoUpdate,
                &cron,
                &settings.default_schedule_timezone,
            )],
        }
    }
}
```

### 5.3 Default Schedule UI

New `m_default_schedule` button in `m_settings` menu leading to:
- **Frequency**: daily / weekly (with weekday picker for weekly)
- **Time**: hour picker (0-23) + minute picker (0, 5, 10, ..., 55)
- **Timezone**: same picker as existing custom schedule builder

## 6. File Changes

| File | Change |
|------|--------|
| `Cargo.toml` | add rust-i18n |
| `src/i18n/zh-CN.yml` | new — Chinese translations |
| `src/i18n/en.yml` | new — English translations |
| `src/i18n/ja.yml` | new — Japanese translations |
| `src/lib.rs` | add i18n init macro |
| `src/app/state.rs` | add language field + methods |
| `src/bootstrap.rs` | extend BotSettings with schedule + language fields |
| `src/handlers/menu.rs` | add language/schedule buttons, migrate all strings to `t!()` |
| `src/handlers/schedule.rs` | migrate all strings to `t!()` |
| `src/handlers/callback.rs` | migrate all strings to `t!()` |
| `src/handlers/mod.rs` | route new callback data |
| `src/handlers/ops.rs` | migrate all strings to `t!()` |
| `src/handlers/log.rs` | migrate all strings to `t!()` |
| `src/handlers/message.rs` | migrate all strings to `t!()` |
| `src/handlers/warp.rs` | migrate all strings to `t!()` |
| `src/handlers/singbox.rs` | migrate all strings to `t!()` |
| `src/handlers/xray.rs` | migrate all strings to `t!()` |
| `src/app/auth.rs` | migrate all strings to `t!()` |
| `src/app/destruct_flow.rs` | migrate all strings to `t!()` |
| `src/logic/system/scheduler/mod.rs` | `from_settings()`, `build_default_cron()` |
| `src/logic/system/scheduler/task_types.rs` | `get_display_name()` → i18n |
| `src/logic/system/monitor.rs` | `get_status_report()` → i18n |
| `src/utils/mod.rs` | `format_duration_human()` → i18n |
| `src/main.rs` | migrate all strings to `t!()` |

## 7. Out of Scope

- Backend logs (`log::info!`, etc.) remain English-only
- `obfstr!()` wrapped security strings unchanged
- Existing custom schedule UI structure unchanged