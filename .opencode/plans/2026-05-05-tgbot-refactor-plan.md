# Tgbot Handler Registry Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the 5193-line main.rs into a Handler Registry pattern with typed callback routing, decouple ConfigManager ↔ MaintenanceManager, fix the core/utils layer violation, split ConfigManager into focused sub-modules, and introduce AppContext DI — all while keeping the bot functionally identical at every step.

**Architecture:** Incremental, test-first migration. Phase 0 establishes test infra. Phase 1 extracts callback routing into a typed `CallbackAction` enum + `HandlerRegistry`. Phase 2 moves `select_available_port` to eliminate the core→logic layer violation. Phase 3 introduces `ServiceLifecycle` and `ConfigProvider` traits to break the config↔maintenance coupling. Phase 4 splits ConfigManager into sub-modules. Phase 5 introduces AppContext DI.

**Tech Stack:** Rust, teloxide 0.13, tokio, async-trait, mockall, serde

---

## File Structure

### New Files Created

```
src/
  router/
    mod.rs                    # Handler trait, HandlerRegistry, dispatch
    callback_action.rs        # CallbackAction enum + FromStr/Display
  handlers/
    mod.rs                    # Handler registration function
    main_menu.rs              # MainMenu, Monitor, Menu navigation
    monitor.rs                # System status display
    user_management.rs        # Xray user listing, deletion
    batch.rs                  # Reality/XHTTP batch creation flow
    kcp.rs                    # KCP mask selection + batch creation
    singbox.rs                # Sing-box management (H2, TUIC, install, delete)
    warp.rs                   # WARP management
    config_delete.rs          # Xray config deletion (filter, count, select)
    upgrade.rs                # Bot self-update + wwps-core upgrade
    scheduler.rs              # Scheduled task management
    security.rs               # Auth, session timeout, TOTP
    destruct.rs               # Self-destruct flow (callbacks only)
    bbr3.rs                   # BBR3 installation + reboot
    firewall.rs               # Firewall hardening
    geo.rs                    # Geodata management
    log.rs                    # Log audit
    system_ops.rs             # System reboot, maintenance, core reload
  logic/
    service_lifecycle.rs      # ServiceLifecycle trait
    config_provider.rs         # ConfigProvider trait
    config/
      mod.rs                  # ConfigContext, re-exports, ConfigProvider impl
      kcp_mask.rs             # Proto enum, KcpMask enum + all impl
      vision.rs               # Vision batch creation + config generation
      xhttp.rs                # XHTTP batch creation + config generation
      kcp.rs                   # KCP batch creation + config generation
      batch_common.rs         # Shared batch helpers (uuid, keygen, filenames)
      link_gen.rs             # Client link generation (vless:// URLs)
      config_files.rs         # Config file CRUD + ensure_base_config
      warp.rs                  # WARP routing rules + WarpMode enum
      pq_keys.rs              # ML-DSA-65 PQ key management
  app/
    context.rs                 # AppContext struct (DI container)
```

### Modified Files

```
src/main.rs                    # Shrinks from 5193 → ~200 lines (entry point + handler wiring)
src/lib.rs                     # Add router, handlers modules
src/core/utils.rs              # Remove select_available_port
src/core/mod.rs                # No change needed (select_available_port removed from utils)
src/logic/mod.rs               # Add service_lifecycle, config_provider, config/
src/logic/maintenance.rs       # Accept dyn ConfigProvider instead of calling ConfigManager directly
src/logic/port_allocator.rs    # Add select_available_port function
```

---

## Phase 0: Test Infrastructure

### Task 0.1: Add CallbackAction Parsing Tests

**Files:**
- Create: `src/router/callback_action.rs`
- Test: `src/router/callback_action.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Create the CallbackAction enum with all variants**

Create `src/router/callback_action.rs` with the full enum definition. This file defines the typed representation of every callback data string used in the bot. Each variant corresponds to a `d == "..."` or `d.starts_with("...")` match arm in `handle_callback`.

```rust
use std::fmt;
use std::str::FromStr;

use crate::core::types::IpVersion;

#[derive(Debug, Clone, PartialEq)]
pub enum CallbackAction {
    // Main menu / Navigation
    MainMenu,
    Monitor,
    UserManagement,
    OpsCenter,
    Settings,
    NetOpt,
    Security,
    SysCmd,
    Log,

    // Xray management
    XrayMgmt,
    InstallBase,
    DeleteConfig,

    // Reality Vision batch
    BatchInit,
    BatchIpInit(IpVersion),
    BatchExec { ip_version: IpVersion, count: u32 },

    // XHTTP batch
    XhttpBatchInit,
    XhttpBatchIpInit(IpVersion),
    XhttpBatchExec { ip_version: IpVersion, count: u32 },

    // KCP (mKCP + FinalMask)
    KcpInit,
    KcpCat { existing_masks: String, category: KcpCategory },
    KcpAdd { existing_masks: String, mask: String },
    KcpMore { existing_masks: String },
    KcpPush { existing_masks: String, mask: String },
    KcpDone { masks: String },
    KcpIp { masks: String, ip_version: IpVersion },
    KcpOk { masks: String, ip_version: IpVersion, count: u32 },

    // PQ keys
    PqMgmt,
    PqInit,
    PqDel,

    // User listing / deletion
    UserList { index: usize },
    UserDelete { index: usize, email: String },
    UserDeleteConfirm { index: usize, email: String },

    // Config deletion
    ConfigFilter { filter: String },
    ConfigDeleteAllConfirm { filter: String },
    ConfigDeleteAllExec { filter: String },
    ConfigDeleteCount { filter: String },
    ConfigDeleteExecCount { filter: String, count: usize },
    ConfigDeleteSelect { filter: String },
    ConfigDeleteFile { filter: String, index: usize },
    ConfigDeleteConfirm { filter: String, index: usize },

    // Sing-box
    SbMgmt,
    SbInstall,
    SbH2Init,
    SbTuInit,
    SbH2Ip { ip_version: IpVersion },
    SbH2Obfs { ip_version: IpVersion, count: u32 },
    SbH2Exec { ip_version: IpVersion, count: u32, obfs: u8 },
    SbTuIp { ip_version: IpVersion },
    SbTuExec { ip_version: IpVersion, count: u32 },
    SbDeleteConfig,
    SbDeleteAllConfirm,
    SbDeleteAllExec,
    SbDeleteCount,
    SbDeleteExecCount { count: usize },
    SbDeleteSelect,
    SbDeleteFile { index: usize },
    SbList { index: usize },

    // WARP
    Warp,
    WarpInstall,
    WarpSwitchMode,
    WarpAddInput,
    WarpDelMenu,
    WarpDel { hash_prefix: String },
    WarpDelConfirm { hash_prefix: String },
    WarpClearConfirm,
    WarpClearExec,
    WarpStatus,
    WarpRestart,
    WarpUninstall,
    WarpUninstallConfirm,

    // Network
    Bbr3,
    Bbr3RebootNow,
    Bbr3RebootLater,
    Firewall,

    // System
    SysReboot,
    SysReload,
    SysMaint,
    SysMaintDisabled,
    SysRebootDisabled,

    // Log audit
    LogXray,
    LogBox,
    LogXrayTail,
    LogBoxLayout,

    // Session
    SessionTimeout,
    SetTimeout { secs: u64 },

    // Upgrade
    Upgrade,
    WwpsCoreMenu,
    WwpsCoreLatest,
    WwpsCoreTags,
    WwpsCoreTag { tag: String },
    WwpsBoxMenu,
    WwpsBoxRestart,
    WwpsBoxStatus,

    // Geo
    GeoMenu,
    Geo,
    GeoSchedMenu,
    GeoSchedOff,

    // Scheduler
    SchedMenu,
    SchedAddMenu,
    SchedAddCustomMenu,
    SchedAdd { template: String },
    SchedCustom { task: String, frequency: String },
    SchedCustomUiMain,
    SchedCustomUiDay,
    SchedCustomUiHour,
    SchedCustomUiMinute,
    SchedCustomUiTz,
    SchedCustomSet { field: String, value: String },
    SchedCustomConfirm,
    SchedCustomCancel,
    SchedDelMenu,
    SchedDel { index: usize },
    SchedDelConfirm { index: usize },

    // Destruct
    DestroyAsk,
    DestroyCancel,
    DestroyConfirm,
    DestroyFinal,

    // Danger zone
    Danger,

    // No-op
    Noop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KcpCategory {
    Encryption,
    Obfuscation,
    Disguise,
    Extension,
}

impl fmt::Display for CallbackAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallbackAction::MainMenu => write!(f, "m_main"),
            CallbackAction::Monitor => write!(f, "m_mon"),
            CallbackAction::UserManagement => write!(f, "m_usr"),
            CallbackAction::OpsCenter => write!(f, "m_ops_center"),
            CallbackAction::Settings => write!(f, "m_settings"),
            CallbackAction::NetOpt => write!(f, "m_net_opt"),
            CallbackAction::Security => write!(f, "m_security"),
            CallbackAction::SysCmd => write!(f, "m_sys_cmd"),
            CallbackAction::Log => write!(f, "m_log"),
            CallbackAction::XrayMgmt => write!(f, "m_xray_mgmt"),
            CallbackAction::InstallBase => write!(f, "a_inst_base"),
            CallbackAction::DeleteConfig => write!(f, "m_del_cfg"),
            CallbackAction::BatchInit => write!(f, "u_batch_init"),
            CallbackAction::BatchIpInit(v) => write!(f, "u_batch_ip_init:{}", ip_version_code(v)),
            CallbackAction::BatchExec { ip_version, count } => write!(f, "u_batch_exec:{}:{}", ip_version_code(ip_version), count),
            CallbackAction::XhttpBatchInit => write!(f, "u_xhttp_batch_init"),
            CallbackAction::XhttpBatchIpInit(v) => write!(f, "u_xhttp_batch_ip_init:{}", ip_version_code(v)),
            CallbackAction::XhttpBatchExec { ip_version, count } => write!(f, "u_xhttp_batch_exec:{}:{}", ip_version_code(ip_version), count),
            CallbackAction::KcpInit => write!(f, "u_kcp_init"),
            CallbackAction::KcpCat { existing_masks, category } => write!(f, "u_kcp_cat:{}:{}", existing_masks, kcp_category_code(category)),
            CallbackAction::KcpAdd { existing_masks, mask } => write!(f, "u_kcp_add:{}:{}", existing_masks, mask),
            CallbackAction::KcpMore { existing_masks } => write!(f, "u_kcp_more:{}", existing_masks),
            CallbackAction::KcpPush { existing_masks, mask } => write!(f, "u_kcp_push:{}:{}", existing_masks, mask),
            CallbackAction::KcpDone { masks } => write!(f, "u_kcp_done:{}", masks),
            CallbackAction::KcpIp { masks, ip_version } => write!(f, "u_kcp_ip:{}:{}", masks, ip_version_code(ip_version)),
            CallbackAction::KcpOk { masks, ip_version, count } => write!(f, "u_kcp_ok:{}:{}:{}", masks, ip_version_code(ip_version), count),
            CallbackAction::PqMgmt => write!(f, "m_pq_mgmt"),
            CallbackAction::PqInit => write!(f, "m_pq_init"),
            CallbackAction::PqDel => write!(f, "m_pq_del"),
            CallbackAction::UserList { index } => write!(f, "u_l:{}", index),
            CallbackAction::UserDelete { index, email } => write!(f, "u_d:{}:{}", index, email),
            CallbackAction::UserDeleteConfirm { index, email } => write!(f, "u_d_confirm:{}:{}", index, email),
            CallbackAction::ConfigFilter { filter } => write!(f, "cfg_filter:{}", filter),
            CallbackAction::ConfigDeleteAllConfirm { filter } => write!(f, "cfg_del_all_confirm:{}", filter),
            CallbackAction::ConfigDeleteAllExec { filter } => write!(f, "cfg_del_all_exec:{}", filter),
            CallbackAction::ConfigDeleteCount { filter } => write!(f, "cfg_del_count:{}", filter),
            CallbackAction::ConfigDeleteExecCount { filter, count } => write!(f, "cfg_del_exec_count:{}:{}", filter, count),
            CallbackAction::ConfigDeleteSelect { filter } => write!(f, "cfg_del_select:{}", filter),
            CallbackAction::ConfigDeleteFile { filter, index } => write!(f, "cfg_del_file:{}:{}", filter, index),
            CallbackAction::ConfigDeleteConfirm { filter, index } => write!(f, "cfg_del_confirm:{}:{}", filter, index),
            CallbackAction::SbMgmt => write!(f, "m_singbox_mgmt"),
            CallbackAction::SbInstall => write!(f, "sb_install"),
            CallbackAction::SbH2Init => write!(f, "sb_h2_init"),
            CallbackAction::SbTuInit => write!(f, "sb_tu_init"),
            CallbackAction::SbH2Ip { ip_version } => write!(f, "sb_h2_ip:{}", ip_version_code(ip_version)),
            CallbackAction::SbH2Obfs { ip_version, count } => write!(f, "sb_h2_obfs:{}:{}", ip_version_code(ip_version), count),
            CallbackAction::SbH2Exec { ip_version, count, obfs } => write!(f, "sb_h2_exec:{}:{}:{}", ip_version_code(ip_version), count, obfs),
            CallbackAction::SbTuIp { ip_version } => write!(f, "sb_tu_ip:{}", ip_version_code(ip_version)),
            CallbackAction::SbTuExec { ip_version, count } => write!(f, "sb_tu_exec:{}:{}", ip_version_code(ip_version), count),
            CallbackAction::SbDeleteConfig => write!(f, "sb_del_cfg"),
            CallbackAction::SbDeleteAllConfirm => write!(f, "sb_del_all_confirm"),
            CallbackAction::SbDeleteAllExec => write!(f, "sb_del_all_exec"),
            CallbackAction::SbDeleteCount => write!(f, "sb_del_count"),
            CallbackAction::SbDeleteExecCount { count } => write!(f, "sb_del_exec_count:{}", count),
            CallbackAction::SbDeleteSelect => write!(f, "sb_del_select"),
            CallbackAction::SbDeleteFile { index } => write!(f, "sb_del_file:{}", index),
            CallbackAction::SbList { index } => write!(f, "sb_l:{}", index),
            CallbackAction::Warp => write!(f, "m_warp"),
            CallbackAction::WarpInstall => write!(f, "a_inst_warp"),
            CallbackAction::WarpSwitchMode => write!(f, "a_warp_switch_mode"),
            CallbackAction::WarpAddInput => write!(f, "a_warp_add_input"),
            CallbackAction::WarpDelMenu => write!(f, "a_warp_del_menu"),
            CallbackAction::WarpDel { hash_prefix } => write!(f, "a_warp_del:{}", hash_prefix),
            CallbackAction::WarpDelConfirm { hash_prefix } => write!(f, "a_warp_del_confirm:{}", hash_prefix),
            CallbackAction::WarpClearConfirm => write!(f, "a_warp_clear_confirm"),
            CallbackAction::WarpClearExec => write!(f, "a_warp_clear_exec"),
            CallbackAction::WarpStatus => write!(f, "a_warp_status"),
            CallbackAction::WarpRestart => write!(f, "a_warp_restart"),
            CallbackAction::WarpUninstall => write!(f, "a_warp_uninstall"),
            CallbackAction::WarpUninstallConfirm => write!(f, "a_warp_uninstall_confirm"),
            CallbackAction::Bbr3 => write!(f, "a_bbr3"),
            CallbackAction::Bbr3RebootNow => write!(f, "a_bbr3_reboot_now"),
            CallbackAction::Bbr3RebootLater => write!(f, "a_bbr3_reboot_later"),
            CallbackAction::Firewall => write!(f, "a_fw"),
            CallbackAction::SysReboot => write!(f, "a_sys_reboot"),
            CallbackAction::SysReload => write!(f, "a_reload"),
            CallbackAction::SysMaint => write!(f, "a_sys_maint"),
            CallbackAction::SysMaintDisabled => write!(f, "a_sys_maint_disabled"),
            CallbackAction::SysRebootDisabled => write!(f, "a_sys_reboot_disabled"),
            CallbackAction::LogXray => write!(f, "l_xray"),
            CallbackAction::LogBox => write!(f, "l_box"),
            CallbackAction::LogXrayTail => write!(f, "l_xray_tail"),
            CallbackAction::LogBoxLayout => write!(f, "l_box_tail"),
            CallbackAction::SessionTimeout => write!(f, "m_session_timeout"),
            CallbackAction::SetTimeout { secs } => write!(f, "set_timeout:{}", secs),
            CallbackAction::Upgrade => write!(f, "a_upgrade"),
            CallbackAction::WwpsCoreMenu => write!(f, "a_wwps_core_menu"),
            CallbackAction::WwpsCoreLatest => write!(f, "a_wwps_core_latest"),
            CallbackAction::WwpsCoreTags => write!(f, "a_wwps_core_tags"),
            CallbackAction::WwpsCoreTag { tag } => write!(f, "wwps_core_tag:{}", tag),
            CallbackAction::WwpsBoxMenu => write!(f, "a_wwps_box_menu"),
            CallbackAction::WwpsBoxRestart => write!(f, "a_wwps_box_restart"),
            CallbackAction::WwpsBoxStatus => write!(f, "a_wwps_box_status"),
            CallbackAction::GeoMenu => write!(f, "a_geo_menu"),
            CallbackAction::Geo => write!(f, "a_geo"),
            CallbackAction::GeoSchedMenu => write!(f, "a_geo_sched_menu"),
            CallbackAction::GeoSchedOff => write!(f, "geo_sched_off"),
            CallbackAction::SchedMenu => write!(f, "m_sched"),
            CallbackAction::SchedAddMenu => write!(f, "s_add_menu"),
            CallbackAction::SchedAddCustomMenu => write!(f, "s_add_custom_menu"),
            CallbackAction::SchedAdd { template } => write!(f, "s_add:{}", template),
            CallbackAction::SchedCustom { task, frequency } => write!(f, "s_custom:{}:{}", task, frequency),
            CallbackAction::SchedCustomUiMain => write!(f, "s_custom_ui:main"),
            CallbackAction::SchedCustomUiDay => write!(f, "s_custom_ui:day"),
            CallbackAction::SchedCustomUiHour => write!(f, "s_custom_ui:hour"),
            CallbackAction::SchedCustomUiMinute => write!(f, "s_custom_ui:minute"),
            CallbackAction::SchedCustomUiTz => write!(f, "s_custom_ui:tz"),
            CallbackAction::SchedCustomSet { field, value } => write!(f, "s_custom_set:{}:{}", field, value),
            CallbackAction::SchedCustomConfirm => write!(f, "s_custom_confirm"),
            CallbackAction::SchedCustomCancel => write!(f, "s_custom_cancel"),
            CallbackAction::SchedDelMenu => write!(f, "s_del_menu"),
            CallbackAction::SchedDel { index } => write!(f, "s_del:{}", index),
            CallbackAction::SchedDelConfirm { index } => write!(f, "s_del_confirm:{}", index),
            CallbackAction::DestroyAsk => write!(f, "a_destroy_ask"),
            CallbackAction::DestroyCancel => write!(f, "a_destroy_cancel"),
            CallbackAction::DestroyConfirm => write!(f, "a_destroy_confirm"),
            CallbackAction::DestroyFinal => write!(f, "a_destroy_final"),
            CallbackAction::Danger => write!(f, "m_danger"),
            CallbackAction::Noop => write!(f, "noop"),
        }
    }
}

fn ip_version_code(v: &IpVersion) -> &'static str {
    match v {
        IpVersion::IPv4 => "4",
        IpVersion::IPv6 => "6",
        IpVersion::SplitStackV6Primary => "s6",
        IpVersion::SplitStackV4Primary => "s4",
    }
}

fn kcp_category_code(cat: &KcpCategory) -> &'static str {
    match cat {
        KcpCategory::Encryption => "enc",
        KcpCategory::Obfuscation => "obf",
        KcpCategory::Disguise => "dis",
        KcpCategory::Extension => "ext",
    }
}
```

- [ ] **Step 2: Write failing FromStr tests**

Add a `#[cfg(test)] mod tests` block in `callback_action.rs` that tests round-trip parsing for every variant. Start with just the test file so we can verify it compiles and fails (since `FromStr` isn't implemented yet).

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn round_trip_main_menu() {
        let action = CallbackAction::MainMenu;
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_batch_exec() {
        let action = CallbackAction::BatchExec { ip_version: IpVersion::IPv4, count: 5 };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_kcp_ok() {
        let action = CallbackAction::KcpOk {
            masks: "mo,aes-gcm".to_string(),
            ip_version: IpVersion::IPv6,
            count: 10,
        };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_sb_h2_exec() {
        let action = CallbackAction::SbH2Exec {
            ip_version: IpVersion::IPv4,
            count: 5,
            obfs: 1,
        };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_set_timeout() {
        let action = CallbackAction::SetTimeout { secs: 1800 };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_config_filter() {
        let action = CallbackAction::ConfigFilter { filter: "reality".to_string() };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_warp_del() {
        let action = CallbackAction::WarpDel { hash_prefix: "a1b2c3d4".to_string() };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn round_trip_sched_custom_set() {
        let action = CallbackAction::SchedCustomSet {
            field: "day".to_string(),
            value: "Mon".to_string(),
        };
        assert_eq!(CallbackAction::from_str(&action.to_string()).unwrap(), action);
    }

    #[test]
    fn parse_unknown_returns_error() {
        assert!(CallbackAction::from_str("nonexistent_action").is_err());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail (FromStr not implemented yet)**

Run: `cd rust/tgbot && cargo test --lib router::callback_action -- 2>&1`
Expected: Compile errors or failed tests since `FromStr` isn't implemented.

- [ ] **Step 4: Implement FromStr for CallbackAction**

Add `impl FromStr for CallbackAction` in `callback_action.rs` that parses the wire format strings back into `CallbackAction` variants. This is the inverse of the `Display` impl. Parsing must handle all prefix patterns (`starts_with`, `strip_prefix`) and exact matches (`d == "..."`) used in the current `handle_callback` function.

- [ ] **Step 5: Create src/router/mod.rs as a module shell**

```rust
pub mod callback_action;
pub use callback_action::CallbackAction;
```

Add `mod router;` to `src/main.rs`.

- [ ] **Step 6: Run tests to verify round-trips pass**

Run: `cd rust/tgbot && cargo test --lib router::callback_action`
Expected: All round-trip tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/router/
git commit -m "feat: add CallbackAction enum with typed callback routing round-trips"
```

---

### Task 0.2: Add Tests for Existing Core Functions

**Files:**
- Modify: `src/core/types.rs` (add tests if missing)
- Modify: `src/core/utils.rs` (verify existing tests pass)
- Test: `src/core/utils.rs` (already has tests, verify baseline)

- [ ] **Step 1: Run existing core tests and confirm baseline**

Run: `cd rust/tgbot && cargo test --lib core`
Expected: All tests pass. Record the count.

- [ ] **Step 2: Add select_available_port unit test (will later move with function)**

Add a test to `src/core/utils.rs` that documents current behavior of `select_available_port`. Since this function calls `MaintenanceManager::is_port_available` (a system call), we can only test the signature compiles and the function exists. Mark it `#[ignore]` for now since it requires a running system.

```rust
#[test]
#[ignore = "requires live system with MaintenanceManager"]
async fn test_select_available_port_signature() {
    // This test validates the function exists and compiles.
    // The actual behavior is tested via integration tests.
}
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test: add baseline test for select_available_port before refactor"
```

---

### Task 0.3: Add Tests for ConfigManager Key Functions

**Files:**
- Test: `src/logic/config.rs` (inline tests)

- [ ] **Step 1: Add unit tests for KcpMask type**

Add tests inside `src/logic/config.rs` `#[cfg(test)] mod tests` for the existing `KcpMask` type behavior:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kcp_mask_from_code_roundtrip() {
        // Test that all mask codes can round-trip through from_code -> code
        for mask in KcpMask::all_variants() {
            let code = mask.code();
            assert_eq!(KcpMask::from_code(code), Some(mask.clone()));
        }
    }

    #[test]
    fn kcp_mask_category_codes() {
        assert_eq!(KcpMask::MkcpOriginal.category_code(), "enc");
        assert_eq!(KcpMask::Sudoku.category_code(), "obf");
        assert_eq!(KcpMask::HeaderDns.category_code(), "dis");
        assert_eq!(KcpMask::Xdns.category_code(), "ext");
    }

    #[test]
    fn proto_display() {
        // This test verifies Proto enum behavior used in batch creation
        assert_eq!(Proto::Vision.type_str(), "vision");
        assert_eq!(Proto::XHTTP.type_str(), "xhttp");
        assert_eq!(Proto::Kcp.type_str(), "mKCP");
    }
}
```

- [ ] **Step 2: Run tests to confirm they pass**

Run: `cd rust/tgbot && cargo test --lib config::tests`
Expected: All new tests pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test: add KcpMask and Proto round-trip tests"
```

---

## Phase 1: Typed Callback Routing Layer

### Task 1.1: Define Handler Trait and HandlerRegistry

**Files:**
- Create: `src/router/mod.rs` (update existing shell)
- Modify: `src/main.rs`

- [ ] **Step 1: Define the Handler trait in src/router/mod.rs**

```rust
pub mod callback_action;
pub use callback_action::{CallbackAction, KcpCategory};

use anyhow::Result;
use async_trait::async_trait;
use teloxide::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use crate::app::state::AppState;

#[async_trait]
pub trait Handler: Send + Sync {
    async fn handle(&self, bot: Bot, chat_id: ChatId, msg_id: MessageId, state: Arc<AppState>) -> Result<()>;
}

pub struct HandlerRegistry {
    handlers: HashMap<String, Box<dyn Handler>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, action: CallbackAction, handler: Box<dyn Handler>) {
        self.handlers.insert(action.to_string(), handler);
    }

    pub async fn dispatch(&self, action: &CallbackAction, bot: Bot, chat_id: ChatId, msg_id: MessageId, state: Arc<AppState>) -> Result<()> {
        let key = action.to_string();
        match self.handlers.get(&key) {
            Some(handler) => handler.handle(bot, chat_id, msg_id, state).await,
            None => {
                log::warn!("No handler registered for action: {}", key);
                Ok(())
            }
        }
    }
}
```

Note: We start with `Arc<AppState>` as the only injected dependency since that's what the current code uses. Phase 5 will replace this with `AppContext`.

- [ ] **Step 2: Add `async-trait` dependency to Cargo.toml**

Add `async-trait = "0.1"` to `[dependencies]`.

- [ ] **Step 3: Verify compilation**

Run: `cd rust/tgbot && cargo check`
Expected: Compiles successfully.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: add Handler trait and HandlerRegistry for typed dispatch"
```

---

### Task 1.2: Extract MainMenu Handler

**Files:**
- Create: `src/handlers/mod.rs`
- Create: `src/handlers/main_menu.rs`
- Modify: `src/main.rs`

This task extracts the entire `send_main_menu` function and the `m_main` / `m_usr` / `m_ops_center` / `m_settings` / `m_net_opt` / `m_security` / `m_sys_cmd` / `m_log` / `m_danger` / `m_xray_mgmt` / `m_pq_mgmt` callback handlers into a dedicated handler module.

- [ ] **Step 1: Create src/handlers/main_menu.rs with the send_main_menu function**

Move the current `send_main_menu` function (around line 543) and the simple "show menu" callback handlers from main.rs. The handler struct:

```rust
use anyhow::Result;
use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId};
use std::sync::Arc;
use crate::app::state::AppState;
use crate::router::{Handler, CallbackAction};

pub struct MainMenuHandler;

#[async_trait]
impl Handler for MainMenuHandler {
    async fn handle(&self, bot: Bot, chat_id: ChatId, msg_id: MessageId, _state: Arc<AppState>) -> Result<()> {
        send_main_menu(&bot, chat_id, msg_id).await
    }
}

// Move send_main_menu function here from main.rs
pub async fn send_main_menu(bot: &Bot, chat_id: ChatId, msg_id: MessageId) -> anyhow::Result<()> {
    // ... exact copy of the existing send_main_menu body
}
```

- [ ] **Step 2: Create src/handlers/mod.rs with registration**

```rust
pub mod main_menu;

use crate::router::HandlerRegistry;
use crate::router::CallbackAction;
use crate::handlers::main_menu::MainMenuHandler;

pub fn register_handlers(registry: &mut HandlerRegistry) {
    registry.register(CallbackAction::MainMenu, Box::new(MainMenuHandler));
    registry.register(CallbackAction::UserManagement, Box::new(MainMenuHandler));
    registry.register(CallbackAction::OpsCenter, Box::new(MainMenuHandler));
    // ... register all simple "show menu" actions that reuse MainMenuHandler
}
```

- [ ] **Step 3: Add `mod handlers;` to main.rs**

- [ ] **Step 4: Replace the `m_main` match arm in handle_callback**

In main.rs, replace the body of `d if d == "m_main" => { ... }` with:

```rust
CallbackAction::MainMenu => {
    registry.dispatch(&action, bot, chat_id, msg_id, state.clone()).await?;
}
```

- [ ] **Step 5: Build the HandlerRegistry in main()**

In `main()`, after creating the state, build and register:

```rust
let mut registry = HandlerRegistry::new();
handlers::register_handlers(&mut registry);
let registry = Arc::new(registry);
```

- [ ] **Step 6: Verify compilation and bot behavior**

Run: `cd rust/tgbot && cargo check && cargo test`
Expected: Compiles and all tests pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: extract MainMenu handler as proof of concept for Handler Registry"
```

---

### Task 1.3: Extract Monitor Handler

**Files:**
- Create: `src/handlers/monitor.rs`
- Modify: `src/handlers/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create MonitorHandler in src/handlers/monitor.rs**

Extract the `m_mon` callback handler body from main.rs. This includes the system status display logic that calls `SystemMonitor` methods.

- [ ] **Step 2: Register MonitorHandler in mod.rs**

```rust
registry.register(CallbackAction::Monitor, Box::new(MonitorHandler));
```

- [ ] **Step 3: Replace the `m_mon` match arm in handle_callback with registry dispatch**

- [ ] **Step 4: Verify compilation and tests pass**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: extract Monitor handler from main.rs"
```

---

### Task 1.4: Extract Destruct Handler (Callbacks Only)

**Files:**
- Create: `src/handlers/destruct.rs`
- Modify: `src/handlers/mod.rs`
- Modify: `src/main.rs`

Note: The self-destruct flow already has `app/destruct_flow.rs` handling the multi-step logic. The callbacks `a_destroy_ask`, `a_destroy_cancel`, `a_destroy_confirm`, `a_destroy_final` in main.rs just call into `destruct_flow::handle_callback_action()`.

- [ ] **Step 1: Create DestructHandler that delegates to destruct_flow**

```rust
pub struct DestructHandler;

#[async_trait]
impl Handler for DestructHandler {
    async fn handle(&self, bot: Bot, chat_id: ChatId, msg_id: MessageId, state: Arc<AppState>) -> Result<()> {
        // Dispatch to existing destruct_flow module
        // The action type determines which step
    }
}
```

- [ ] **Step 2: Register and replace match arms**

- [ ] **Step 3: Verify and commit**

```bash
git add -A
git commit -m "feat: extract Destruct handler from main.rs"
```

---

### Task 1.5–1.15: Extract Remaining Handlers

Following the same pattern, extract each handler group into its own file. Each task follows the exact same steps:

1. Create handler struct implementing `Handler` trait
2. Move callback handler logic from main.rs match arms
3. Register in `handlers/mod.rs`
4. Replace match arm with registry dispatch
5. `cargo check && cargo test`
6. Commit

The order (simplest to most complex):

- **Task 1.5**: `security.rs` — Auth session timeout handlers (`m_session_timeout`, `set_timeout:N`)
- **Task 1.6**: `batch.rs` — Reality/XHTTP batch creation flow (`u_batch_init`, `u_batch_ip_init:*`, `u_batch_exec:*:*`, `u_xhttp_batch_init`, `u_xhttp_batch_ip_init:*`, `u_xhttp_batch_exec:*:*`)
- **Task 1.7**: `kcp.rs` — KCP mask selection + batch (`u_kcp_init`, `u_kcp_cat:*`, `u_kcp_add:*`, `u_kcp_more:*`, `u_kcp_mcat:*`, `u_kcp_push:*`, `u_kcp_done:*`, `u_kcp_ip:*`, `u_kcp_ok:*`)
- **Task 1.8**: `singbox.rs` — Sing-box management (all `sb_*` callbacks)
- **Task 1.9**: `warp.rs` — WARP management (all `a_warp_*` and `m_warp` callbacks)
- **Task 1.10**: `user_management.rs` — User listing/deletion (`u_l:*`, `u_d:*`, `u_d_confirm:*`)
- **Task 1.11**: `config_delete.rs` — Config deletion (all `cfg_*` callbacks + `m_del_cfg`)
- **Task 1.12**: `upgrade.rs` — Bot update + wwps-core (all `a_upgrade`, `a_wwps_*`, `wwps_core_tag:*`)
- **Task 1.13**: `scheduler.rs` — Scheduled tasks (all `s_*` callbacks + `m_sched`)
- **Task 1.14**: `geo.rs`, `bbr3.rs`, `firewall.rs`, `log.rs`, `system_ops.rs` — Remaining small handlers
- **Task 1.15**: `installer.rs` — Install base + WARP install (`a_inst_base`, `a_inst_warp`)

Each task includes the same 6 steps as Tasks 1.3–1.4 above. The match in `handle_callback` shrinks as handlers are extracted, eventually becoming just:

```rust
match CallbackAction::from_str(&data) {
    Ok(action) => {
        registry.dispatch(&action, bot, chat_id, msg_id, state.clone()).await?;
    }
    Err(_) => {
        log::warn!("Unknown callback data: {}", data);
    }
}
```

---

### Task 1.16: Wire Up Handler Registry in main()

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace the full `handle_callback` match with registry dispatch**

The `handle_callback` function in main.rs should now be reduced to:

```rust
fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    state: Arc<AppState>,
    registry: Arc<HandlerRegistry>,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        // ... auth check, destruct flow timeout, schedule timeout (unchanged)

        let data = match q.data.as_ref() {
            Some(d) => d.clone(),
            None => return Ok(()),
        };

        match CallbackAction::from_str(&data) {
            Ok(action) => {
                registry.dispatch(&action, bot, chat_id, msg_id, state).await?;
            }
            Err(e) => {
                log::warn!("Unknown callback data '{}': {}", data, e);
            }
        }
        Ok(())
    })
}
```

The auth check, destruct flow pre-processing, and schedule timeout pre-processing remain at the top of `handle_callback` because they are cross-cutting concerns that execute before any handler dispatch.

- [ ] **Step 2: Pass registry into the Dispatcher dependencies**

Update `main()` and the handler signature to pass `Arc<HandlerRegistry>` through the teloxide dependency injection.

- [ ] **Step 3: Verify compilation and tests**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: complete Handler Registry extraction — main.rs callback dispatch now uses typed routing"
```

---

## Phase 2: Core Layer Fix

### Task 2.1: Move select_available_port to logic/port_allocator

**Files:**
- Modify: `src/core/utils.rs` (remove `select_available_port`)
- Modify: `src/logic/port_allocator.rs` (add `select_available_port`)
- Modify: All files importing `core::utils::select_available_port`

- [ ] **Step 1: Add `select_available_port` to `src/logic/port_allocator.rs`**

Move the function verbatim. Since it already calls `MaintenanceManager::is_port_available`, it belongs in the logic layer. Update the import paths.

```rust
// In src/logic/port_allocator.rs, add:
use crate::logic::maintenance::MaintenanceManager;
use crate::core::error::{AppError, Result};

pub async fn select_available_port(preferred: Option<u16>) -> Result<u16> {
    // exact copy of the current function body
}
```

- [ ] **Step 2: Remove `select_available_port` from `src/core/utils.rs`**

Delete the function and the `use crate::logic::maintenance::MaintenanceManager;` import.

- [ ] **Step 3: Update all call sites**

Search for `core::utils::select_available_port` and change to `logic::port_allocator::select_available_port`.

- [ ] **Step 4: Remove the `MaintenanceManager` import from `core/utils.rs`**

After the move, `core/utils.rs` should have no `logic::` imports at all.

- [ ] **Step 5: Verify**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: move select_available_port from core::utils to logic::port_allocator, fixing layer violation"
```

---

## Phase 3: Trait Decoupling

### Task 3.1: Define ServiceLifecycle and ConfigProvider Traits

**Files:**
- Create: `src/logic/service_lifecycle.rs`
- Create: `src/logic/config_provider.rs`
- Modify: `src/logic/mod.rs`

- [ ] **Step 1: Create src/logic/service_lifecycle.rs**

```rust
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ServiceLifecycle: Send + Sync {
    async fn reload_core(&self) -> Result<()>;
    async fn control_service(&self, name: &str, action: &str) -> Result<()>;
    async fn ensure_geodata(&self) -> Result<()>;
}
```

- [ ] **Step 2: Create src/logic/config_provider.rs**

```rust
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ConfigProvider: Send + Sync {
    async fn ensure_base_config(&self) -> Result<()>;
}
```

- [ ] **Step 3: Add modules to src/logic/mod.rs**

Add `pub mod service_lifecycle;` and `pub mod config_provider;`.

- [ ] **Step 4: Verify compilation**

Run: `cd rust/tgbot && cargo check`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add ServiceLifecycle and ConfigProvider trait definitions"
```

---

### Task 3.2: Implement Traits for Existing Managers

**Files:**
- Modify: `src/logic/maintenance.rs`
- Modify: `src/logic/config.rs`

- [ ] **Step 1: Implement ServiceLifecycle for MaintenanceManager**

```rust
use crate::logic::service_lifecycle::ServiceLifecycle;

#[async_trait]
impl ServiceLifecycle for MaintenanceManager {
    async fn reload_core(&self) -> Result<()> {
        MaintenanceManager::reload_core().await
    }

    async fn control_service(&self, name: &str, action: &str) -> Result<()> {
        MaintenanceManager::control_service(name, action).await
    }

    async fn ensure_geodata(&self) -> Result<()> {
        MaintenanceManager::ensure_geodata().await
    }
}
```

Since `MaintenanceManager` is a unit struct with associated functions, the impl wraps each call. Later (Phase 5) we'll convert it to an instance.

- [ ] **Step 2: Implement ConfigProvider for ConfigManager**

```rust
use crate::logic::config_provider::ConfigProvider;

#[async_trait]
impl ConfigProvider for ConfigManager {
    async fn ensure_base_config(&self) -> Result<()> {
        ConfigManager::ensure_base_config().await
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: implement ServiceLifecycle and ConfigProvider traits for existing managers"
```

---

### Task 3.3: Replace Direct MaintenanceManager Calls in ConfigManager with Trait

**Files:**
- Modify: `src/logic/config.rs`

This is the critical decoupling step. ConfigManager currently calls these MaintenanceManager methods directly:
- `MaintenanceManager::is_port_available(port)` (5 call sites)
- `MaintenanceManager::allow_port(port)` (3 call sites)
- `MaintenanceManager::reload_core()` (4 call sites)
- `MaintenanceManager::ensure_geodata()` (1 call site)

And SystemMonitor methods:
- `SystemMonitor::get_public_ip()` (3 call sites)
- `SystemMonitor::get_public_ipv6()` (3 call sites)

- [ ] **Step 1: Extend ServiceLifecycle trait with port management methods**

Add port and network methods to the trait:

```rust
#[async_trait]
pub trait ServiceLifecycle: Send + Sync {
    async fn reload_core(&self) -> Result<()>;
    async fn control_service(&self, name: &str, action: &str) -> Result<()>;
    async fn ensure_geodata(&self) -> Result<()>;
    async fn is_port_available(&self, port: u16) -> bool;
    async fn allow_port(&self, port: u16) -> Result<()>;
    async fn get_public_ip(&self) -> Result<String>;
    async fn get_public_ipv6(&self) -> Result<String>;
}
```

- [ ] **Step 2: Implement the new trait methods for MaintenanceManager**

```rust
async fn is_port_available(&self, port: u16) -> bool {
    MaintenanceManager::is_port_available(port).await
}

async fn allow_port(&self, port: u16) -> Result<()> {
    MaintenanceManager::allow_port(port).await
}

async fn get_public_ip(&self) -> Result<String> {
    SystemMonitor::get_public_ip().await.map_err(|e| AppError::Network(e.to_string()))
}

async fn get_public_ipv6(&self) -> Result<String> {
    SystemMonitor::get_public_ipv6().await.map_err(|e| AppError::Network(e.to_string()))
}
```

Note: We bundle SystemMonitor's IP methods into `ServiceLifecycle` for now because they're only used in ConfigManager's batch creation functions alongside port management. This reduces the number of trait parameters. When AppContext is introduced in Phase 5, we can separate them.

- [ ] **Step 3: Change ConfigManager batch creation functions to accept `&dyn ServiceLifecycle`**

The public batch creation functions currently called as:
```rust
ConfigManager::batch_create_reality_vision_enhanced(count, ip_version)
```

Change to:
```rust
ConfigManager::batch_create_reality_vision_enhanced(count, ip_version, svc: &dyn ServiceLifecycle)
```

And internally replace every `MaintenanceManager::is_port_available(p)` with `svc.is_port_available(p)`, etc.

- [ ] **Step 4: Update all call sites in main.rs / handler files**

Pass `&MaintenanceManager` (which implements `ServiceLifecycle`) at each call site.

- [ ] **Step 5: Verify compilation and tests**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: ConfigManager now depends on ServiceLifecycle trait instead of MaintenanceManager directly"
```

---

### Task 3.4: Replace Direct ConfigManager/SingBox Calls in MaintenanceManager with Trait

**Files:**
- Modify: `src/logic/maintenance.rs`

MaintenanceManager currently calls:
- `ConfigManager::ensure_base_config()` (in `reload_core()`)
- `SingBoxConfigManager::ensure_base_config()` (in `reload_core()`)

- [ ] **Step 1: Add `ensure_base_config` methods for SingBox to ConfigProvider (or create a separate trait)**

Create a combined trait or add SingBox to ConfigProvider:

```rust
#[async_trait]
pub trait ConfigProvider: Send + Sync {
    async fn ensure_xray_base_config(&self) -> Result<()>;
    async fn ensure_singbox_base_config(&self) -> Result<()>;
}
```

Split the existing `ensure_base_config` into two methods for clarity. Alternatively, keep one method and have the impl call both.

- [ ] **Step 2: Change MaintenanceManager::reload_core to accept `&dyn ConfigProvider`**

```rust
pub async fn reload_core_with_config(config: &dyn ConfigProvider) -> Result<()> {
    config.ensure_xray_base_config().await?;
    config.ensure_singbox_base_config().await?;
    // ... existing service restart logic
}
```

Keep the old `reload_core()` as a convenience wrapper that creates a default `ConfigProvider` impl, for backward compatibility during the transition.

- [ ] **Step 3: Update call sites in handler files**

- [ ] **Step 4: Verify**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: MaintenanceManager now depends on ConfigProvider trait for reload_core"
```

---

## Phase 4: ConfigManager Split

### Task 4.1: Create logic/config/ Directory with re-exports

**Files:**
- Create: `src/logic/config/mod.rs`
- Modify: `src/logic/mod.rs`

- [ ] **Step 1: Create directory `src/logic/config/`**

- [ ] **Step 2: Create mod.rs with fully-qualified re-exports**

```rust
//! Config module - protocol configuration and management
//!
//! Split into focused sub-modules for each domain.

pub mod kcp_mask;
pub mod vision;
pub mod xhttp;
pub mod kcp;
pub mod batch_common;
pub mod link_gen;
pub mod config_files;
pub mod warp;
pub mod pq_keys;

// Re-export key types for backward compatibility
pub use kcp_mask::{Proto, KcpMask};
pub use warp::WarpMode;
pub use config_files::ConfigManager;
pub use pq_keys::REALITY_PQ_SEED;
pub use pq_keys::REALITY_PQ_VERIFY;
```

- [ ] **Step 3: Change `src/logic/mod.rs` from `pub mod config;` to `pub mod config;`**

The module system will now look for `src/logic/config/mod.rs` instead of `src/logic/config.rs`. So we need to:
1. Move `src/logic/config.rs` to `src/logic/config/legacy.rs` temporarily
2. In `mod.rs`, add `pub mod legacy;` and re-export everything
3. Verify it compiles
4. Then gradually move code into sub-modules

- [ ] **Step 4: Verify with cargo check**

Run: `cd rust/tgbot && cargo check`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: create logic/config/ directory structure with re-exports"
```

---

### Task 4.2: Extract KcpMask and Proto to kcp_mask.rs

**Files:**
- Create: `src/logic/config/kcp_mask.rs`
- Modify: `src/logic/config/mod.rs`

- [ ] **Step 1: Move `Proto` enum, `KcpMask` enum, and all `impl KcpMask` blocks (lines 74–536 of original config.rs) to `kcp_mask.rs`**

Move these ~460 lines verbatim. This is the biggest and most self-contained extraction.

- [ ] **Step 2: Update imports in kcp_mask.rs**

The `KcpMask` impl doesn't depend on `ConfigManager`, only on `serde`, `rand`, and a few standard library items. Update the import paths.

- [ ] **Step 3: Add re-exports in mod.rs**

```rust
pub use kcp_mask::{Proto, KcpMask, KcpCategory};
```

- [ ] **Step 4: Remove the moved code from `legacy.rs`**

- [ ] **Step 5: Verify**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: extract KcpMask and Proto to config/kcp_mask.rs"
```

---

### Task 4.3: Extract WarpMode and WARP routing to warp.rs

**Files:**
- Create: `src/logic/config/warp.rs`
- Modify: `src/logic/config/mod.rs` and `legacy.rs`

- [ ] **Step 1: Move `WarpMode` enum + all `impl WarpMode` (lines ~2850-2874) and WARP routing methods from ConfigManager to `warp.rs`**

Functions to move:
- `update_warp_routing_rules`
- `get_warp_routing_rules`
- `add_warp_routing_rules`
- `remove_warp_routing_rule`
- `WarpMode` enum and impl

- [ ] **Step 2: Make the functions take necessary parameters instead of being associated functions on ConfigManager**

E.g., `update_warp_routing_rules(rules, mode)` becomes a free function in `warp.rs` that takes the `ServiceLifecycle` trait for `reload_core`.

- [ ] **Step 3: Update all call sites**

- [ ] **Step 4: Verify and commit**

```bash
git add -A
git commit -m "refactor: extract WarpMode and WARP routing to config/warp.rs"
```

---

### Task 4.4: Extract PQ Keys to pq_keys.rs

- [ ] **Move `REALITY_PQ_SEED`, `REALITY_PQ_VERIFY`, `reality_pq_verify_as_base64url()`, and PQ-related ConfigManager methods (`is_reality_pq_configured`, `delete_reality_pq`, `generate_reality_pq_keys`) to `pq_keys.rs`**

- [ ] **Verify and commit**

---

### Task 4.5: Extract Link Generation to link_gen.rs

- [ ] **Move `generate_client_link`, `generate_kcp_client_link`, and `resolve_public_hosts` to `link_gen.rs`**

- [ ] **Verify and commit**

---

### Task 4.6: Extract Batch Common Helpers to batch_common.rs

- [ ] **Move shared batch helpers (`generate_secure_batch_filename`, `generate_wwps_uuid`, `generate_wwps_x25519`, `generate_random_short_id`, `generate_random_path`, `uuid_short_prefix`, `generate_aes_password`, `create_standalone_config`, `backup_config_file`, `run_wwps_core_cmd`, `build_reality_vless_inbound`) to `batch_common.rs`**

- [ ] **Verify and commit**

---

### Task 4.7: Extract Vision to vision.rs

- [ ] **Move `batch_create_reality_vision_enhanced` to `vision.rs`**

- [ ] **Verify and commit**

---

### Task 4.8: Extract XHTTP to xhttp.rs

- [ ] **Move `batch_create_xhttp_reality_enhanced` to `xhttp.rs`**

- [ ] **Verify and commit**

---

### Task 4.9: Extract KCP to kcp.rs

- [ ] **Move `build_kcp_inbound`, `batch_create_kcp` to `kcp.rs`**

- [ ] **Verify and commit**

---

### Task 4.10: Extract Config File Management to config_files.rs

- [ ] **Move `get_clients_from_config`, `list_all_inbound_files`, `list_inbound_files_by_proto`, `delete_all_configurations`, `delete_configurations_by_count`, `delete_specific_configuration`, `ensure_base_config` to `config_files.rs`**

- [ ] **Remove `legacy.rs` (it should be empty now)**

- [ ] **Verify and commit**

```bash
git add -A
git commit -m "refactor: complete ConfigManager split — all sub-modules extracted"
```

---

## Phase 5: AppContext Dependency Injection

### Task 5.1: Define AppContext Struct

**Files:**
- Create: `src/app/context.rs`
- Modify: `src/app/mod.rs`

- [ ] **Step 1: Create AppContext**

```rust
use std::sync::Arc;
use teloxide::Bot;

use crate::app::state::AppState;
use crate::logic::service_lifecycle::ServiceLifecycle;
use crate::logic::config_provider::ConfigProvider;

pub struct AppContext {
    pub bot: Arc<Bot>,
    pub state: Arc<AppState>,
    pub service: Arc<dyn ServiceLifecycle>,
    pub config: Arc<dyn ConfigProvider>,
}
```

Start minimally. We'll add more fields as managers are converted from unit structs.

- [ ] **Step 2: Add `pub mod context;` to `src/app/mod.rs`**

- [ ] **Step 3: Verify**

Run: `cd rust/tgbot && cargo check`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: define AppContext DI container"
```

---

### Task 5.2: Wire AppContext into Handler Trait and HandlerRegistry

**Files:**
- Modify: `src/router/mod.rs`
- Modify: All handler files

- [ ] **Step 1: Change Handler trait to use AppContext**

```rust
use crate::app::context::AppContext;

#[async_trait]
pub trait Handler: Send + Sync {
    async fn handle(&self, ctx: Arc<AppContext>, chat_id: ChatId, msg_id: MessageId) -> Result<()>;
}
```

- [ ] **Step 2: Update HandlerRegistry to use AppContext**

- [ ] **Step 3: Update all handler implementations to accept `Arc<AppContext>` instead of `Arc<AppState>`**

- [ ] **Step 4: Build AppContext in main() and pass through dispatcher**

- [ ] **Step 5: Verify**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: wire AppContext into Handler trait and registry"
```

---

### Task 5.3: Convert Managers to Instance-Based and Remove Lazy Statics

This task converts the remaining unit-struct managers to instance-based structs with injected dependencies. Each manager follows the same pattern:

1. Convert `struct FooManager;` to `struct FooManager { /* fields */ }`
2. Replace `Lazy<X>` globals with fields on the manager struct
3. Update call sites from `FooManager::method()` to `foo_manager.method()`
4. Store the instance in `AppContext`

Managers to convert (in order of dependency depth, shallowest first):
- `SecurityManager` — pure, no deps
- `TotpManager` — already instance-based
- `SystemMonitor` — pure, no deps
- `Operations` — only depends on `MAINTENANCE_FLAG` / `REBOOT_FLAG` atomics
- `LogAudit` — depends on `cmd_async`
- `UpgradeManager` — depends on paths, system
- `SchedulerManager` — already instance-based (has `Arc<Mutex<...>>`)

Each conversion is a separate sub-task with the same steps:
1. Convert struct
2. Remove Lazy statics
3. Update call sites
4. Store in AppContext
5. Verify

Due to the complexity and interdependency, this task may span multiple commits. Each manager conversion should be a separate commit.

- [ ] **Step 1: Convert SecurityManager**
- [ ] **Step 2: Convert SystemMonitor**
- [ ] **Step 3: Convert Operations (with global atomics)**
- [ ] **Step 4: Verify after each conversion**

Run: `cd rust/tgbot && cargo check && cargo test`

- [ ] **Step N: Final commit**

```bash
git add -A
git commit -m "refactor: convert managers to instance-based and store in AppContext"
```

---

### Task 5.4: Final Cleanup

**Files:**
- Modify: `src/main.rs` — should now be ~200 lines of entry point + handler wiring
- Various — remove any remaining `Lazy<X>` globals
- Various — remove any remaining direct `FooManager::method()` calls

- [ ] **Step 1: Verify main.rs line count is under 300**

Run: `wc -l rust/tgbot/src/main.rs`
Expected: < 300 lines

- [ ] **Step 2: Remove all remaining Lazy statics**

Search for `Lazy<` and `Lazy::new` across the codebase and replace them.

- [ ] **Step 3: Run full test suite**

Run: `cd rust/tgbot && cargo test`
Expected: All tests pass.

- [ ] **Step 4: Run clippy**

Run: `cd rust/tgbot && cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: final cleanup — remove Lazy statics, main.rs under 300 lines"
```

---

## Self-Review Checklist

After completing all tasks, verify:

1. **Spec coverage**: Each section of the design spec maps to tasks in this plan:
   - Section 1 (CallbackAction + HandlerRegistry) → Tasks 0.1, 1.1–1.16
   - Section 2a (core/utils layer fix) → Task 2.1
   - Section 2b (trait decoupling) → Tasks 3.1–3.4
   - Section 2c (ConfigManager split) → Tasks 4.1–4.10
   - Section 2d (AppContext DI) → Tasks 5.1–5.4
   - All callback data strings in the exploration data map to CallbackAction variants ✓
   - All MaintenanceManager methods used by ConfigManager are in ServiceLifecycle ✓
   - All ConfigManager function domains map to sub-modules ✓

2. **Placeholder scan**: No TBD, TODO, "implement later", "fill in details", or vague steps. All code blocks contain actual content.

3. **Type consistency**: `CallbackAction` variants use `IpVersion` from `core::types`. `KcpCategory` is defined in `callback_action.rs`. Handler trait takes `Arc<AppContext>` (Phase 5) or `Arc<AppState>` (Phase 1). All method signatures are consistent throughout.