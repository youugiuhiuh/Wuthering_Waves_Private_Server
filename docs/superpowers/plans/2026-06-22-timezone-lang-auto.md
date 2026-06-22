# 语言选择联动系统时区与安全更新定时 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 语言选择时自动设置系统时区，覆盖 apt-daily timer 为 00:00-04:00 窗口，重启时间改为 05:00。

**Architecture:** 在 `i18n::lang_to_timezone()` 后追加 `timedatectl set-timezone` + `systemctl daemon-reload` 使 timer override 生效。新增 `set_apt_daily_timer()` 函数写入 systemd override 配置。

**Tech Stack:** Rust, tokio::fs, tokio::process::Command, systemd timer override

---

### 文件结构

| 文件 | 职责 |
|------|------|
| `rust/aegis/src/core/system/operations.rs` | 新增 `set_apt_daily_timer()`, 改 `DEFAULT_REBOOT_TIME` |
| `rust/aegis/src/adapters/telegram/handlers/callback.rs` | 语言选择后加 `timedatectl` + timer override |
| `rust/aegis/src/main.rs` | 启动后加 `timedatectl` + timer override |

---

### Task 1: 新增 `set_apt_daily_timer()` 纯函数测试

**Files:**
- Modify: `rust/aegis/src/core/system/operations.rs` — test module 末尾

- [ ] **Step 1: 写失败测试**

在 `#[cfg(test)] mod tests` 末尾添加：

```rust
#[test]
fn test_apt_daily_timer_override_content() {
    let content = AutoUpdateConfigurator::apt_daily_timer_override();
    assert!(content.contains("OnCalendar=daily"));
    assert!(content.contains("RandomizedDelaySec=4h"));
}

#[test]
fn test_apt_daily_upgrade_timer_override_content() {
    let content = AutoUpdateConfigurator::apt_daily_upgrade_timer_override();
    assert!(content.contains("OnCalendar=daily"));
    assert!(content.contains("RandomizedDelaySec=4h"));
    assert!(content.contains("After=apt-daily.service"));
}
```

Run: `cargo test test_apt_daily_timer_override_content` → FAIL (function not found)

- [ ] **Step 2: 写最小实现**

在 `impl AutoUpdateConfigurator` 中 `rhel_config()` 之后、结束 `}` 之前添加：

```rust
pub fn apt_daily_timer_override() -> String {
    r#"[Timer]
OnCalendar=daily
RandomizedDelaySec=4h
"#
    .to_string()
}

pub fn apt_daily_upgrade_timer_override() -> String {
    r#"[Timer]
OnCalendar=daily
RandomizedDelaySec=4h

[Unit]
After=apt-daily.service
"#
    .to_string()
}
```

Run: `cargo test test_apt_daily_timer_override_content` → PASS

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/core/system/operations.rs
git commit -m "test: add apt-daily timer override content tests"
```

---

### Task 2: 实现 `set_apt_daily_timer()` + 改 `DEFAULT_REBOOT_TIME`

**Files:**
- Modify: `rust/aegis/src/core/system/operations.rs`

- [ ] **Step 1: 修改 `DEFAULT_REBOOT_TIME`**

在 `perform_security_update_task()` 上方（约 line 370）：

将 `"02:00"` 改为 `"05:00"`

- [ ] **Step 2: 新增 `set_apt_daily_timer()` 函数**

在 `perform_security_update_task()` 之后、`reboot_system()` 之前插入：

```rust
pub async fn set_apt_daily_timer() -> Result<()> {
    let upgrade_override_dir = "/etc/systemd/system/apt-daily-upgrade.timer.d";
    tokio::fs::create_dir_all(upgrade_override_dir)
        .await
        .context("创建 apt-daily-upgrade.timer.d 目录失败")?;

    let upgrade_content = AutoUpdateConfigurator::apt_daily_upgrade_timer_override();
    tokio::fs::write(
        format!("{}/aegis-timezone.conf", upgrade_override_dir),
        upgrade_content,
    )
    .await
    .context("写入 apt-daily-upgrade.timer override 失败")?;

    let daily_override_dir = "/etc/systemd/system/apt-daily.timer.d";
    tokio::fs::create_dir_all(daily_override_dir)
        .await
        .context("创建 apt-daily.timer.d 目录失败")?;

    let daily_content = AutoUpdateConfigurator::apt_daily_timer_override();
    tokio::fs::write(
        format!("{}/aegis-timezone.conf", daily_override_dir),
        daily_content,
    )
    .await
    .context("写入 apt-daily.timer override 失败")?;

    run_cmd_checked("systemctl", &["daemon-reload"], Duration::from_secs(10))
        .await
        .context("systemctl daemon-reload 失败")?;

    Ok(())
}
```

- [ ] **Step 3: 验证**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/system/operations.rs && git commit -m "feat: add set_apt_daily_timer() and change reboot to 05:00"
```

---

### Task 3: 语言选择时联动（callback.rs）

**Files:**
- Modify: `rust/aegis/src/adapters/telegram/handlers/callback.rs:44-60`

- [ ] **Step 1: 读文件确认上下文**

读 `callback.rs` line 44-60 确认 `let tz = i18n::lang_to_timezone(lang);` 的位置。

- [ ] **Step 2: 在 `let tz = i18n::lang_to_timezone(lang);` 后插入**

```rust
let tz = i18n::lang_to_timezone(lang);

if let Err(e) = tokio::process::Command::new("timedatectl")
    .args(["set-timezone", tz])
    .output()
    .await
{
    log::warn!("设置系统时区 {} 失败: {}", tz, e);
}

if let Err(e) = aegis::core::system::operations::Operations::set_apt_daily_timer().await {
    log::warn!("覆盖 apt-daily timer 失败: {}", e);
}
```

- [ ] **Step 3: 验证**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/adapters/telegram/handlers/callback.rs && git commit -m "feat: set system timezone and apt-daily timer on language selection"
```

---

### Task 4: 启动时联动（main.rs）

**Files:**
- Modify: `rust/aegis/src/main.rs:498-505`

- [ ] **Step 1: 读文件确认上下文**

读 `main.rs` line 498-505 确认语言初始化位置。

- [ ] **Step 2: 在 `mark_lang_configured()` 后插入**

```rust
    if let Some(ref lang_str) = encrypted_config.lang {
        let lang = lang_str.parse().unwrap_or(i18n::Lang::Zh);
        i18n::set_lang(lang);
        state.set_lang(lang).await;
        state.mark_lang_configured().await;
        i18n::mark_lang_configured();

        let tz = i18n::lang_to_timezone(lang);
        if let Err(e) = tokio::process::Command::new("timedatectl")
            .args(["set-timezone", tz])
            .output()
            .await
        {
            log::warn!("设置系统时区 {} 失败: {}", tz, e);
        }

        if let Err(e) = aegis::core::system::operations::Operations::set_apt_daily_timer()
            .await
        {
            log::warn!("覆盖 apt-daily timer 失败: {}", e);
        }
    }
```

- [ ] **Step 3: 验证**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Expected: 全绿

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/main.rs && git commit -m "feat: set system timezone and apt-daily timer on startup"
```

---

### Task 5: 最终验证

**Files:** N/A

- [ ] **Step 1: 全量验证**

```bash
cargo fmt && cargo clippy -- -D warnings && cargo test
```

Expected: 全绿，0 警告，434+ 测试通过

- [ ] **Step 2: 确认文件改动清单**

```bash
git diff --stat
```

Expected: 3 个文件改动（operations.rs, callback.rs, main.rs）
