# 设计文档：语言选择联动系统时区与安全更新定时

**日期**: 2026-06-22
**状态**: 已设计 / 待实施
**影响文件**:
- `rust/aegis/src/core/system/operations.rs`
- `rust/aegis/src/adapters/telegram/handlers/callback.rs`
- `rust/aegis/src/main.rs`

---

## 1. 问题

语言选择后 `lang_to_timezone()` 仅用于 Aegis scheduler 的 cron 任务时区，从未调用 `timedatectl set-timezone` 设置系统时区。导致：

1. `apt-daily.timer` 使用安装时的系统时区（随机，不受控）
2. `Automatic-Reboot-Time "02:00"` 不在可控时区
3. 三台服务器时区不一致（PST / IST），更新节奏差 12 小时
4. `RandomizedDelaySec=12h`（Debian 默认）使更新时间在 00:00-12:00 随机分布

## 2. 目标

- 语言选择（中文/英文/日文）时自动设置系统时区
- apt-daily timer 窗口锁定为 00:00-04:00（用户时区）
- 重启时间改为 05:00（用户时区），与业务高峰错开
- 启动时从持久化配置同步时区和 timer 设置

## 3. 方案

### 3.1 语言 ↔ 时区映射（已有）

| 语言 | 时区 |
|------|------|
| 中文 | `Asia/Shanghai` |
| 英文 | `America/New_York` |
| 日文 | `Asia/Tokyo` |

### 3.2 语言选择时执行 4 项操作

| # | 操作 | 文件 |
|---|------|------|
| 1 | `timedatectl set-timezone <tz>` 设系统时区 | `callback.rs`, `main.rs` |
| 2 | 重建 Aegis 定时任务（随 tz 重建，已有） | `callback.rs` |
| 3 | override `apt-daily.timer` → `RandomizedDelaySec=4h` | `operations.rs` → 由 callback/main 调用 |
| 4 | override `apt-daily-upgrade.timer` → `RandomizedDelaySec=4h` + `After=apt-daily.service` | `operations.rs` → 由 callback/main 调用 |

### 3.3 最终时间线

```
00:00 ───────────── 04:00 ── 05:00 ───────────── 24:00
│                     │       │
│  apt update/upgrade │       │  系统重启
│  (4h 随机窗口)       │       │  (安全更新)
│  安全更新安装完成     │       │
└────── 不重启 ───────→ 等 05:00 统一重启
```

### 3.4 systemd timer override 内容

**`/etc/systemd/system/apt-daily.timer.d/aegis-timezone.conf`**:

```ini
[Timer]
OnCalendar=daily
RandomizedDelaySec=4h
```

**`/etc/systemd/system/apt-daily-upgrade.timer.d/aegis-timezone.conf`**:

```ini
[Timer]
OnCalendar=daily
RandomizedDelaySec=4h

[Unit]
After=apt-daily.service
```

### 3.5 改动点

**`operations.rs`**:
- 新增 `apt_daily_timer_override()` 纯函数返回 timer config 内容
- 新增 `apt_daily_upgrade_timer_override()` 纯函数
- 新增 `set_apt_daily_timer()` 写入 override 文件 + `systemctl daemon-reload`
- `DEFAULT_REBOOT_TIME` → `"05:00"`

**`callback.rs:50`**:
- `let tz = i18n::lang_to_timezone(lang);` 后追加:
  - `timedatectl set-timezone $tz`
  - `Operations::set_apt_daily_timer().await`

**`main.rs:504`**:
- `mark_lang_configured()` 后追加同上

### 3.6 兼容性

| 系统 | `timedatectl` | systemd timer override |
|------|:--:|:--:|
| Debian 12+ | ✅ | ✅ |
| Ubuntu 20.04+ | ✅ | ✅ |
| RHEL 9+ | ✅ | ✅ |

---

## 4. 验证

- `cargo fmt && cargo clippy -- -D warnings` 通过
- `cargo test` 全绿（含新增的 apt_daily_timer_override 内容测试）
- `timedatectl status` 确认时区已切换
- `systemctl cat apt-daily.timer` 确认 override 已生效
- `systemctl cat apt-daily-upgrade.timer` 确认 override 已生效
