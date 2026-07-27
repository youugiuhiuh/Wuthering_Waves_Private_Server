# 删除 Release API 根地址列表 — 修改边界

## 概述

删除 `rust/aegis` 中两个 `*_release_api_bases()` 函数及完整调用链：
1. `aegis_release_api_bases()` — bot 自升级镜像源列表
2. `wwps_core_release_api_bases()` — Xray-core 升级镜像源列表

同时删除 `NicholasDewar/Wuthering_Waves_Private_Server` 等默认仓库引用。

---

## 修改清单

### A. `rust/aegis/src/core/system/upgrade.rs` — Bot 自升级

| 行号 | 内容 | 类型 |
|------|------|------|
| 23–26 | `DEFAULT_RELEASE_REPOSITORIES` 常量 (含 `NicholasDewar`) | 删除 |
| 30–47 | `aegis_release_api_bases()` 函数 | **直接目标** |
| 52–68 | `ReleaseRepo` 结构体 + `new()` / `display_name()` | 删除 (仅用于 DEFAULT_REPOSITORIES) |
| 70–79 | `parse_release_repo()` 辅助函数 | 删除 |
| 81–109 | `configured_release_repositories()` 函数 | 删除 |
| 111–116 | `UpgradeManager` 结构体字段 `repositories: Vec<ReleaseRepo>` | 需修改 |
| 130–131 | `UpgradeManager::new()` 中 `configured_release_repositories()` 调用 | 需修改 |
| 246–266 | `fetch_latest_release()` — 调用 `aegis_release_api_bases()`，遍历 repositories | 需修改 |
| 268–318 | `fetch_latest_release_from_repo()` — 接收 `bases: &[String]` 参数 | 需修改 |

**环境变量：** `AEGIS_RELEASE_MIRRORS`, `AEGIS_RELEASE_REPOSITORIES`, `AEGIS_RELEASE_REPOSITORY`, `AEGIS_RELEASE_OWNER`, `AEGIS_RELEASE_REPO`

### B. `rust/aegis/src/core/system/core_upgrade.rs` — Xray-core 升级

| 行号 | 内容 | 类型 |
|------|------|------|
| 33–50 | `wwps_core_release_api_bases()` 函数 | **直接目标** |
| 211–232 | `fetch_recent_tags()` — 调用 `wwps_core_release_api_bases()` | 需修改 |
| 234–283 | `fetch_release()` — 调用 `wwps_core_release_api_bases()` | 需修改 |
| 810–819 | `test_wwps_core_release_api_bases_default` | 删除测试 |
| 823–849 | `test_wwps_core_release_api_bases_env_override` | 删除测试 |
| 852–864 | `test_wwps_core_release_api_bases_trailing_slash_stripped` | 删除测试 |
| 721–734 | `with_clear_env()` 测试辅助（仅被上述 3 个测试使用） | 待确认后再删 |

**环境变量：** `WWPS_CORE_RELEASE_MIRRORS`

### C. 调用 `UpgradeManager` 的外部文件

| 文件 | 行号 | 用途 |
|------|------|------|
| `rust/aegis/src/core/system/mod.rs` | 16 | `pub use upgrade::{..., UpgradeManager}` |
| `rust/aegis/src/shared/handlers/ops.rs` | 8 | `use crate::core::system::upgrade::UpgradeManager` |
| `rust/aegis/src/shared/handlers/ops.rs` | 140 | `UpgradeManager::new()` → `run()` |

### D. 共用网络层 — `release_api.rs`（需保留下游）

| 行号 | 内容 | 类型 |
|------|------|------|
| 44–90 | `fetch_json_from_mirrors()` | **保留**（仍被 `fetch_release`/`fetch_latest_release_from_repo` 调用，需改为直接传硬编码地址） |

### E. `NicholasDewar` 特有引用

| 文件 | 行号 | 内容 |
|------|------|------|
| `upgrade.rs` | 24 | `("NicholasDewar", "Wuthering_Waves_Private_Server")` |
| `go/installer/main.go` | 22 | `import "github.com/NicholasDewar/..."` |
| `go/installer/main.go` | 58 | `{Owner: "NicholasDewar", Name: "..."}` |
| `.build.yml` | 20 | `SOURCE_OWNER: NicholasDewar` |

### F. 环境变量清理（i18n 文档引用）

| 文件 | 行号 | 内容 |
|------|------|------|
| `go/installer/i18n/en.json` | 4 | `AEGIS_RELEASE_MIRRORS` |
| `go/installer/i18n/zh.json` | 4 | `AEGIS_RELEASE_MIRRORS` |
| `go/installer/i18n/ja.json` | 4 | `AEGIS_RELEASE_MIRRORS` |

### G. Go installer `releaseAPIBases` / `releaseRepo`（go/installer/main.go）

| 行号 | 内容 | 类型 |
|------|------|------|
| 52–55 | `releaseRepo` 结构体 | 关联 |
| 57–60 | `defaultReleaseRepositories` (含 `NicholasDewar`) | 关联 |
| 62–80 | `releaseAPIBases` + `init()` | 关联 |
| 89–101 | `parseReleaseRepo()` | 关联 |
| 103–130 | `configuredReleaseRepositories()` | 关联 |

---

## 调用链图

```
┌──────────────────────────────────────────────────────┐
│ upgrade.rs                                            │
│                                                       │
│  UpgradeManager::new()                                │
│  ├── configured_release_repositories()                │
│  │   ├── env AEGIS_RELEASE_REPOSITORIES               │
│  │   ├── env AEGIS_RELEASE_REPOSITORY                 │
│  │   ├── env AEGIS_RELEASE_OWNER / AEGIS_RELEASE_REPO │
│  │   └── DEFAULT_RELEASE_REPOSITORIES [DELETE]         │
│  │       ├── ("NicholasDewar", "...")                  │
│  │       └── ("youugiuhiuh", "...")                   │
│  └── ...                                              │
│                                                        │
│  UpgradeManager::run()                                 │
│  └── fetch_latest_release()                            │
│      ├── aegis_release_api_bases() [DELETE]            │
│      │   ├── env AEGIS_RELEASE_MIRRORS                 │
│      │   └── [GitHub, Codeberg, Gitea]                 │
│      └── for each repository:                          │
│          └── fetch_latest_release_from_repo()          │
│              └── fetch_json_from_mirrors() [KEEP]      │
│                  └── {base}/{api_path}                 │
└──────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────┐
│ core_upgrade.rs                                       │
│                                                       │
│  WwpsCoreUpgradeManager::fetch_release()              │
│  ├── wwps_core_release_api_bases() [DELETE]           │
│  │   ├── env WWPS_CORE_RELEASE_MIRRORS                │
│  │   └── [GitHub, Codeberg, Gitea]/repos              │
│  └── fetch_json_from_mirrors() [KEEP]                 │
│                                                        │
│  WwpsCoreUpgradeManager::fetch_recent_tags()           │
│  ├── wwps_core_release_api_bases() [DELETE]           │
│  └── fetch_json_from_mirrors() [KEEP]                 │
│                                                        │
│  调用方:                                               │
│  ├── xray/installer.rs:538 — fetch_release()          │
│  └── handlers/menu.rs:554,579,689 — UpgradeManager    │
└──────────────────────────────────────────────────────┘
```

---

## 后续问题

1. 删除 `*_release_api_bases()` 后，`fetch_json_from_mirrors()` 的调用方如何获取 `bases`？选项：
   - A) 硬编码单个 URL（如 `https://api.github.com/repos`）
   - B) 由调用方传入 `Vec<String>`
   - C) 完全移除 `fetch_json_from_mirrors`，改回简单 `reqwest::get(url)`

2. Go installer (`go/installer/main.go`) 中的对等代码是否一并处理？
