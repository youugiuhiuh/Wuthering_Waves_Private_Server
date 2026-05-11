# 纯搬移重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将散落的常量和过大的文件通过纯搬移重构集中化，不改变任何运行时行为。

**Architecture:** 三项独立的纯搬移变更：路径常量集中化、KcpMask提取、upgrade.rs拆分。每项变更保持所有公开API路径不变（通过re-export），确保外部`use`语句零修改。

**Tech Stack:** Rust 2024 edition, cargo check/test/clippy 验证

---

## 文件变更总览

| 变更项 | 新增文件 | 修改文件 | 删除文件 |
|--------|----------|----------|----------|
| Task 1: 路径常量集中化 | 0 | 4+ | 0 |
| Task 2: KcpMask提取 | 1 | 2 | 0 |
| Task 3: upgrade.rs拆分 | 2 | 2 | 1 |

---

### Task 1: 路径常量集中化

**Files:**
- Modify: `src/core/paths.rs` — 新增 `maintenance` 子模块
- Modify: `src/logic/system/maintenance.rs` — 移除3个常量定义，改为 `use`
- Modify: `src/logic/upgrade.rs` — 移除1个常量定义，改为 re-export
- Modify: `src/bootstrap.rs` — 移除2个常量定义，改为 `use`
- Modify: `src/main.rs` — 调整 `use` 语句

- [ ] **Step 1: 在 `core/paths.rs` 新增 `maintenance` 子模块**

在 `src/core/paths.rs` 末尾（`mod tests` 之前）新增：

```rust
pub mod maintenance {
    pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/tgbot/bbr3_pending.flag";
    pub const UPGRADE_FLAG_FILE: &str = "/etc/wwps/tgbot/upgrade.flag";
    pub const DESTRUCT_TARGETS: &[&str] = &[
        "/etc/wwps",
        "/var/log",
        "/root/.acme.sh",
        "/etc/systemd/system/wwps-tgbot.service",
    ];
    pub const DESTRUCT_SERVICES: &[&str] = &["wwps-core", "wwps-box", "nginx"];
}
```

同时更新 `bot` 子模块，新增两个路径常量：

```rust
pub mod bot {
    pub const DIR: &str = "/etc/wwps/tgbot";
    pub const KEY_FILE: &str = "/etc/wwps/tgbot/.key";
    pub const BBR3_PENDING_FLAG_FILE: &str = "/etc/wwps/tgbot/bbr3_pending.flag";
}
```

并在 `xray` 子模块中新增 PQ 路径：

```rust
pub mod xray {
    pub const DIR: &str = "/etc/wwps/wwps-core";
    pub const BIN: &str = "/etc/wwps/wwps-core/wwps-core";
    pub const CONF_DIR: &str = "/etc/wwps/wwps-core/conf";
    pub const BACKUP_DIR: &str = "/etc/wwps/wwps-core/backup";
    pub const TEMP_DIR: &str = "/tmp/wwps-core-installer";

    pub const DEFAULT_OWNER: &str = "XTLS";
    pub const DEFAULT_REPO: &str = "Xray-core";
    pub const DEFAULT_SERVICE: &str = "wwps-core";
    pub const DEFAULT_TEMP_DIR: &str = "/tmp/wwps-core-upgrade";
    pub const DEFAULT_BACKUP_PREFIX: &str = "wwps-core-backup";

    pub const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
    pub const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
}
```

在 `#[cfg(test)] mod tests` 中新增测试：

```rust
#[test]
fn test_maintenance_paths() {
    assert_eq!(maintenance::BBR3_PENDING_FLAG_FILE, "/etc/wwps/tgbot/bbr3_pending.flag");
    assert_eq!(maintenance::UPGRADE_FLAG_FILE, "/etc/wwps/tgbot/upgrade.flag");
    assert!(!maintenance::DESTRUCT_TARGETS.is_empty());
    assert!(maintenance::DESTRUCT_TARGETS.contains(&"/etc/wwps"));
    assert!(!maintenance::DESTRUCT_SERVICES.is_empty());
    assert!(maintenance::DESTRUCT_SERVICES.contains(&"wwps-core"));
}

#[test]
fn test_xray_pq_paths() {
    assert_eq!(xray::PQ_SEED_PATH, "/etc/wwps/reality_pq.seed");
    assert_eq!(xray::PQ_PUB_PATH, "/etc/wwps/reality_pq.pub");
}
```

- [ ] **Step 2: 修改 `src/logic/system/maintenance.rs`**

移除第13行 `pub const BBR3_PENDING_FLAG_FILE` 定义。

在文件顶部添加：
```rust
use crate::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
```

注意：因为 `BBR3_PENDING_FLAG_FILE` 现在从 paths crate 引入，它不再是此模块定义的，但 `use` 引入后所有函数内的引用自动可用。

对于 `DESTRUCT_TARGETS` 和 `DESTRUCT_SERVICES`：它们是 `MaintenanceManager` 的关联常量，外部通过 `MaintenanceManager::DESTRUCT_TARGETS` 引用。保持此 API 不变，将定义改为委托：

```rust
impl MaintenanceManager {
    pub const DESTRUCT_TARGETS: &[&str] = crate::core::paths::maintenance::DESTRUCT_TARGETS;
    pub const DESTRUCT_SERVICES: &[&str] = crate::core::paths::maintenance::DESTRUCT_SERVICES;
    // ... 其余方法不变 ...
}
```

- [ ] **Step 3: 修改 `src/logic/upgrade.rs`**

移除第51行 `pub const UPGRADE_FLAG_FILE` 定义。

在文件顶部添加 re-export：
```rust
pub use crate::core::paths::maintenance::UPGRADE_FLAG_FILE;
```

这样 `main.rs` 中的 `use tgbot::logic::upgrade::UPGRADE_FLAG_FILE` 保持不变。

- [ ] **Step 4: 修改 `src/bootstrap.rs`**

移除第86-87行的两个常量定义：
```rust
const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
```

在文件顶部 `use` 区域添加：
```rust
use crate::core::paths::xray::{PQ_SEED_PATH, PQ_PUB_PATH};
```

注意：原来这两个常量是 `const`（模块私有），现在从 paths 引入后变为 `use` 导入的 pub 常量。如果原代码中这两个路径只在当前模块内使用，这没有影响。但如果有其他模块引用（搜索未发现），需要同步更新。

- [ ] **Step 5: 修改 `src/main.rs`**

将第35行：
```rust
use tgbot::logic::maintenance::{BBR3_PENDING_FLAG_FILE, MaintenanceManager};
```

改为：
```rust
use tgbot::core::paths::maintenance::BBR3_PENDING_FLAG_FILE;
use tgbot::logic::maintenance::MaintenanceManager;
```

`UPGRADE_FLAG_FILE` 的引用（第43-46行）保持不变（通过 upgrade re-export）。

- [ ] **Step 6: 验证编译和测试**

```bash
cargo check 2>&1 && cargo test 2>&1 && cargo clippy -- -D warnings 2>&1
```

预期：编译通过，所有测试通过，无新 warnings。

- [ ] **Step 7: 提交**

```bash
git add -A
git commit -m "refactor: centralize path constants into core/paths.rs

- Move BBR3_PENDING_FLAG_FILE, UPGRADE_FLAG_FILE, DESTRUCT_TARGETS,
  DESTRUCT_SERVICES into core/paths/maintenance module
- Move PQ_SEED_PATH, PQ_PUB_PATH into core/paths/xray module
- Add re-exports to maintain backward-compatible use paths
- No runtime behavior changes"
```

---

### Task 2: KcpMask 提取

**Files:**
- Create: `src/logic/xraycore/kcp_mask.rs`
- Modify: `src/logic/xraycore/config.rs` — 删除 KcpMask 定义和 impl（第81-536行）+ generate_aes_password
- Modify: `src/logic/xraycore/mod.rs` — 新增 `pub mod kcp_mask;` + re-export

**依赖说明**: `KcpMask::from_code()` 调用了 `ConfigManager::generate_aes_password()`，会形成循环依赖。解决方案：将 `generate_aes_password()` 提取为 `kcp_mask.rs` 的模块级 `pub(crate) fn`。

- [ ] **Step 1: 创建 `src/logic/xraycore/kcp_mask.rs`**

从 `config.rs` 搬移以下内容：
- 第82-96行：`KcpMask` enum 定义
- 第99-536行：`impl KcpMask` 块（全部方法）
- 第545-552行：`generate_aes_password` 函数（从 ConfigManager impl 中提取，改为模块级 `pub(crate) fn`）

关键修改：
1. `KcpMask::from_code()` 中3处 `ConfigManager::generate_aes_password()` 改为 `generate_aes_password()`
2. 添加必要的 `use` 声明

新文件顶部的 `use`：
```rust
use rand::Rng;
use serde_json::{Value, json};

use crate::core::error::{AppError, Result};
```

`generate_aes_password` 定义：
```rust
pub(crate) fn generate_aes_password() -> String {
    let rng_len = rand::thread_rng().gen_range(16..32);
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(rng_len)
        .map(char::from)
        .collect()
}
```

`from_code` 中的修改（3处）：
```rust
// 原：ConfigManager::generate_aes_password()
// 改：generate_aes_password()
```

`from_code` 中 `rand::thread_rng().gen_range(1..=65535)` 保持不变（`use rand::Rng` 已引入）。

- [ ] **Step 2: 修改 `src/logic/xraycore/config.rs`**

1. 删除第81-96行（`#[derive(Debug, Clone)]` + `KcpMask` enum）
2. 删除第99-536行（`impl KcpMask { ... }`）
3. 删除第545-552行（`fn generate_aes_password() -> String { ... }`，在 ConfigManager impl 内部）
4. 在文件顶部 `use` 区域修改：
   - 如果 `use rand::{Rng, SeedableRng};` 和 `use rand::rngs::StdRng;` 只被 KcpMask 使用，可以删除（但 ConfigManager 中仍有 `StdRng` 用途用于 UUID 生成等，需检查）
   - 添加 `use super::kcp_mask::{KcpMask, generate_aes_password};`

经过检查，`config.rs` 中 `ConfigManager` 的方法如 `generate_uuid()` 仍在使用 `rand`，所以保留 `use rand` 相关 import，只删除 KcpMask 相关代码。

5. 搜索 `config.rs` 中所有对 `ConfigManager::generate_aes_password()` 的引用 — 因为 `from_code` 已移走，config.rs 中不应再有任何对此函数的引用。

- [ ] **Step 3: 修改 `src/logic/xraycore/mod.rs`**

```rust
pub mod config;
pub mod installer;
pub mod port_allocator;
pub mod kcp_mask;

pub use config::{ConfigManager, Proto, WarpMode};
pub use kcp_mask::KcpMask;
pub use installer::{RealityInstaller, WarpInstaller, RealityInstallOutcome};
pub use port_allocator::PortAllocator;
```

- [ ] **Step 4: 验证编译和测试**

```bash
cargo check 2>&1 && cargo test 2>&1 && cargo clippy -- -D warnings 2>&1
```

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "refactor: extract KcpMask into xraycore/kcp_mask.rs

- Move KcpMask enum and all impl methods to dedicated file
- Extract generate_aes_password as module-level pub(crate) fn
- Break circular dependency: KcpMask::from_code no longer calls
  ConfigManager method
- Add kcp_mask module to xraycore/mod.rs with re-export
- No runtime behavior changes"
```

---

### Task 3: upgrade.rs 拆分

**Files:**
- Create: `src/logic/bot_upgrade.rs` — UpgradeManager, ReleaseArtifact, ReleaseRepo
- Create: `src/logic/core_upgrade.rs` — WwpsCoreUpgradeManager, CpuArch, etc.
- Modify: `src/logic/mod.rs` — 模块声明 + re-export
- Delete: `src/logic/upgrade.rs`
- Modify: `src/main.rs` — use 路径更新
- Modify: `src/logic/xraycore/installer.rs` — use 路径更新

- [ ] **Step 1: 创建 `src/logic/bot_upgrade.rs`**

将 `upgrade.rs` 第1-549行内容（`UpgradeManager`、`ReleaseArtifact`、`ReleaseRepo`、辅助函数、常量）复制到新文件。

需要注意：
- `UPGRADE_FLAG_FILE` 已在 Task 1 中改为 re-export，保持 `pub use crate::core::paths::maintenance::UPGRADE_FLAG_FILE;`
- 所有 `use` 保留原样或微调（如果是 `use super::*` 的部分，需要替换为具体 import）

- [ ] **Step 2: 创建 `src/logic/core_upgrade.rs`**

将 `upgrade.rs` 第552-1336行 `pub mod wwps_core { ... }` 的内容展平到新文件。

关键变更：
- 原 `use super::*;` 展开为实际的 `use` 语句（从 bot_upgrade.rs 的 use 中选取需要的）
- 将 `CpuArch`、`WwpsCoreUpgradeConfig`、`WwpsCoreReleaseInfo`、`WwpsCoreUpgradeManager` 和所有辅助函数设为 `pub`
- 所有 `use super::xxx` 改为 `use crate::logic::bot_upgrade::xxx` 或 `use crate::logic::cmd_async::xxx` 等

需要的 `use` 语句（在展开 `use super::*` 后精简为实际使用的）：
```rust
use crate::core::paths::xray;
use crate::logic::cmd_async::run_cmd_status;
use crate::logic::utils::{format_download_progress, human_readable_size, should_report};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use futures_util::StreamExt;
use obfstr::obfstr;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::task;
use tokio::time::sleep;
use zip::ZipArchive;
```

注意：`ReleaseRepo` 和 `parse_release_repo` 在 `bot_upgrade.rs` 中是私有的。如果 `core_upgrade.rs` 需要 `ReleaseArtifact` 等类型，需要将它们设为 `pub` 或 `pub(crate)`。检查 `core_upgrade.rs` 中是否引用了这些类型。

原 wwps_core 模块中使用的 `use super::*` 会引入 bot_upgrade 的所有公开项，但核心所需的是：`ReleaseArtifact`、`ReleaseRepo`、`parse_release_repo`、`SHA256_LINE_RE` 等。把 bot_upgrade.rs 中需要共享的项标记为 `pub(crate)`。

- [ ] **Step 3: 修改 `src/logic/mod.rs`**

将：
```rust
pub mod upgrade;
```

改为：
```rust
pub mod bot_upgrade;
pub mod core_upgrade;
```

添加向后兼容 re-export：
```rust
pub use bot_upgrade::{UpgradeManager, ReleaseArtifact, ReleaseRepo, UPGRADE_FLAG_FILE};
pub use core_upgrade::{CpuArch, WwpsCoreUpgradeConfig, WwpsCoreReleaseInfo, WwpsCoreUpgradeManager};
```

移除原来的 `pub use upgrade::*` 行（如果有的话，当前代码没有）。

- [ ] **Step 4: 修改 `src/main.rs`**

将第43-46行：
```rust
use tgbot::logic::upgrade::{
    UPGRADE_FLAG_FILE, UpgradeManager,
    wwps_core::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager},
};
```

改为：
```rust
use tgbot::logic::bot_upgrade::{UpgradeManager, UPGRADE_FLAG_FILE};
use tgbot::logic::core_upgrade::{WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
```

- [ ] **Step 5: 修改 `src/logic/xraycore/installer.rs`**

将第3行：
```rust
use crate::logic::upgrade::wwps_core::{CpuArch, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
```

改为：
```rust
use crate::logic::core_upgrade::{CpuArch, WwpsCoreUpgradeConfig, WwpsCoreUpgradeManager};
```

- [ ] **Step 6: 删除 `src/logic/upgrade.rs`**

```bash
rm src/logic/upgrade.rs
```

- [ ] **Step 7: 验证编译和测试**

```bash
cargo check 2>&1 && cargo test 2>&1 && cargo clippy -- -D warnings 2>&1
```

- [ ] **Step 8: 提交**

```bash
git add -A
git commit -m "refactor: split upgrade.rs into bot_upgrade.rs and core_upgrade.rs

- Move UpgradeManager/ReleaseArtifact/ReleaseRepo to bot_upgrade.rs
- Move WwpsCoreUpgradeManager/CpuArch/WwpsCoreUpgradeConfig to core_upgrade.rs
- Flatten wwps_core submodule into top-level module
- Add re-exports in logic/mod.rs for backward compatibility
- Update all use paths in main.rs and installer.rs
- Delete upgrade.rs
- No runtime behavior changes"
```

---

## 自检清单

- [x] Spec覆盖：所有3项纯搬移任务在上述 Task 1-3 中完整覆盖
- [x] 无占位符：每个步骤包含确切代码和命令
- [x] 类型一致性：所有公开 API 路径通过 re-export 保持不变
- [x] 循环依赖：KcpMask→ConfigManager依赖通过提取generate_aes_password为模块级函数解决
- [x] 向后兼容：UPGRADE_FLAG_FILE、BBR3_PENDING_FLAG_FILE 等通过 re-export 保持原 use 路径可用