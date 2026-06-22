# 设计文档：安全更新自动化 — 首次初始化 + systemd 接管

**日期**: 2026-06-22
**状态**: 已设计 / 待实施
**影响文件**:
- `rust/aegis/src/adapters/telegram/handlers/callback.rs`

---

## 1. 问题

PR #117 引入了 SecurityUpdate cron 定时任务（每天 23:00 跑 `unattended-upgrade -v`）。但 `unattended-upgrade` 的配置文件由 Aegis 写入后不会丢失，且 **systemd timer 已经覆盖了整个更新流程**，此 cron 完全多余。

## 2. 最终架构

```
用户操作（一次性触发初始化）
├─ 语言选择
│   ├─ timedatectl set-timezone      → 系统时区
│   ├─ set_apt_daily_timer()         → 00:00-04:00 窗口
│   ├─ perform_maintenance_with_reboot_time("05:00")  → 首次安装+配置
│   └─ GeoUpdate cron (每周一 01:00)
│
└─ 一键部署 [8/8]
    └─ perform_maintenance_with_reboot_time("05:00")  → 首次安装+配置

此后全自动（无需 Aegis 介入）
00:00 ────────────────── 04:00 ── 05:00
│   systemd apt-daily      │       │
│   + apt-daily-upgrade    │    重启 │
│   (随机 4h 窗口)          │  自动  │
└──────────────────────────┴───────┘
```

## 3. 角色职责

| 角色 | 触发时机 | 职责 |
|------|----------|------|
| Aegis `perform_maintenance_with_reboot_time` | 语言选择 / 一键部署 | 安装 unattended-upgrades + needrestart，写 50unattended-upgrades，enable systemd timer |
| Aegis `set_apt_daily_timer` | 语言选择 | 写入 `RandomizedDelaySec=4h` override，锁定更新窗口 |
| systemd `apt-daily.timer` | 每天 00:00-04:00 | 执行 `apt update` |
| systemd `apt-daily-upgrade.timer` | 每天 00:00-04:00 | 执行 `unattended-upgrade` |
| `Automatic-Reboot-Time` | 每天 05:00 | 按需重启 |

## 4. 改动点

### 唯一改动：删除 SecurityUpdate cron 注册

**`callback.rs`** — 语言选择 handler 末尾，删除 7 行：

```rust
// 删除以下代码
let sec_task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
    aegis::core::system::scheduler::TaskType::SecurityUpdate,
    "0 23 * * *",
    tz,
);
let _ = manager.add_new_task(sec_task).await;
```

仅保留 GeoUpdate cron：

```rust
let geo_task = aegis::core::system::scheduler::ScheduledTask::new_with_timezone(
    aegis::core::system::scheduler::TaskType::GeoUpdate,
    "0 1 * * 1",
    tz,
);
let _ = manager.add_new_task(geo_task).await;
```

## 5. 兼容性

| 场景 | 变更前 | 变更后 |
|------|--------|--------|
| 新部署 + 语言选择 | 首次 init + SecurityUpdate cron (02:00) | 首次 init + 无 cron |
| 新部署 + 一键部署 | 7 步 | 8 步（含安全更新 init）|
| 已有服务器更新 | SecurityUpdate cron 继续存在（旧注册） | 不会新建；旧 cron 无副作用 |

> 注：已有服务器的 SecurityUpdate cron 是在 Aegis scheduler 内存中注册的，更新代码后不会自动删除旧注册。旧 cron 会继续调用 `perform_security_update_task()`，该函数仅检查配置存在性（无实际 upgrade 操作），无不良影响。

## 6. 验证

- `cargo fmt && cargo clippy -- -D warnings` 通过
- `cargo test` 全绿
