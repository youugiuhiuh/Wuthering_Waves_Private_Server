# main.rs 分解重构设计规格

## 目标

将 `main.rs`（5,193 行）拆分为 6 个领域处理器文件 + 1 个生命周期文件，使单个文件不超过 ~1,200 行，降低单文件复杂度。

## 核心约束

1. **严格只移动，不重写**：仅做代码搬迁和必要的 `pub`/`pub(crate)` 可见性调整，不修改任何业务逻辑
2. **渐进式编译**：每移动完一个模块，执行 `cargo check`，不堆积依赖错误
3. **验证通过**：最终 `cargo test` 必须全部通过

## 当前文件结构

```
src/
  main.rs          (5,193 行)
  bootstrap.rs     (433 行，不动)
  lib.rs           (2 行，不动)
  app/
    mod.rs, auth.rs, state.rs (649 行), destruct_flow.rs (337 行)
  logic/           (不动的业务逻辑层)
  core/            (不动的基础层)
```

### main.rs 内容分布详细分析

| 行范围 | 内容 | 目标文件 |
|--------|------|----------|
| 1-59 | use 语句、常量 TOTP_FAIL_MAX/WINDOW/LOCKOUT_DURATIONS | main.rs（保留） |
| 61-77 | format_duration_human() | handlers/system.rs |
| 79-84 | escape_html() | handlers/callback.rs（因 callback 大量使用） |
| 86-97 | validate_hash_prefix() | handlers/callback.rs |
| 99-110 | validate_idx()、MAX_FILE_DOWNLOAD_SIZE | handlers/command.rs |
| 113-269 | show_reality_batch_prompt()、show_reality_qty_prompt() | handlers/proxy.rs |
| 270-274 | trigger_reality_auto_init() | handlers/proxy.rs |
| 276-293 | Command 枚举、looks_like_totp_code() | handlers/command.rs |
| 295-313 | process_auth_code() | handlers/command.rs |
| 315-436 | handle_command() | handlers/command.rs |
| 438-439 | MAX_INPUT_LENGTH | handlers/command.rs |
| 440-541 | handle_message() | handlers/command.rs |
| 543-560 | send_main_menu() | handlers/command.rs |
| 562-755 | schedule_* 系列辅助函数、build_* 系列函数 | handlers/system.rs |
| 756-4919 | handle_callback() 巨型函数（~4,163 行） | handlers/callback.rs |
| 4920-5049 | main() 函数 | main.rs（保留，瘦身） |
| 5051-5105 | #[cfg(test)] mod tests | main.rs（保留） |
| 5107-5141 | notify_online() | app/lifecycle.rs |
| 5143-5163 | notify_upgrade_success() | app/lifecycle.rs |
| 5165-5193 | notify_bbr3_reboot_result() | app/lifecycle.rs |

## 重构后的文件结构

```
src/
  main.rs              (~150 行：main() + tests + 常量 + Dispatcher 组装)
  bootstrap.rs          (不变)
  lib.rs                (不变)
  app/
    mod.rs              (+pub mod lifecycle)
    auth.rs             (不变)
    state.rs            (不变)
    destruct_flow.rs     (不变)
    lifecycle.rs        (新增 ~80 行：启动通知函数)
  handlers/
    mod.rs              (路由入口，re-export 各 handler 函数)
    command.rs          (~260 行：Command 枚举 + handle_command + process_auth_code + send_main_menu + handle_message + looks_like_totp_code)
    callback.rs         (~4,200 行：handle_callback 路由 + 域内处理函数)
    proxy.rs            (~160 行：show_reality_* 系列函数)
    security.rs          (~20 行：validate_hash_prefix 等安全相关工具函数)
    system.rs           (~200 行：schedule_* 系列 + build_* 系列 + format_duration_human + save_config)
  logic/               (不变)
  core/                 (不变)
```

## 各文件详细内容

### handlers/command.rs (~260 行)

**从 main.rs 迁入：**
- `Command` 枚举定义
- `looks_like_totp_code()` 函数
- `process_auth_code()` 异步函数
- `handle_command()` 异步函数
- `MAX_INPUT_LENGTH` 常量
- `MAX_FILE_DOWNLOAD_SIZE` 常量
- `handle_message()` 异步函数
- `send_main_menu()` 异步函数

**依赖导入：**
- AppState, auth, destruct_flow, TOTP 常量等

### handlers/callback.rs (~4,200 行)

**核心结构：**
- `handle_callback()` 函数保留，作为主路由入口
- 每个 callback data 的 match arm 提取为独立 `async fn` 函数

**回调域分组（每个组内函数命名约定 `handle_{prefix}_{action}`）：**

| 回调前缀/匹配 | 目标处理函数 | 预估行数 |
|---------------|-------------|---------|
| `m_main`, `m_ops_center`, `m_settings`, `m_net_opt`, `m_security`, `m_sys_cmd`, `m_mon`, `m_usr`, `m_session_timeout`, `m_danger` | 菜单导航函数 (~10 个) | ~200 |
| `m_xray_mgmt`, `u_*` (reality/xhttp/kcp 安装/删除), `m_del_cfg`, `m_pq_*`, `cfg_*` | Xray/Reality 代理操作 | ~1,800 |
| `m_singbox_mgmt`, `sb_*` (install/h2/tu/del) | Sing-box 代理操作 | ~500 |
| `m_warp`, `a_warp_*`, `a_inst_*` | WARP 代理操作 | ~500 |
| `a_fw`, `m_log`, `l_*` | 安全/日志/防火墙 | ~200 |
| `a_reload`, `a_upgrade`, `a_geo`, `a_tune`, `a_sys_*`, `m_sched`, `s_*` | 系统运维/调度 | ~800 |
| `a_wwps_*` | 核心升级 | ~200 |

**重要设计：** 所有子函数仍放在 callback.rs 这一个文件内，不做跨文件拆分。callback.rs 通过内部函数分组注释来组织代码结构。

### handlers/proxy.rs (~160 行)

**从 main.rs 迁入：**
- `show_reality_batch_prompt()` 异步函数 (行 113-244)
- `show_reality_qty_prompt()` 异步函数 (行 246-269)
- `trigger_reality_auto_init()` 同步函数 (行 270-274)

### handlers/security.rs (~20 行)

**从 main.rs 迁入：**
- `validate_hash_prefix()` 函数 (行 86-97)

注：大部分安全交互逻辑在 `app/destruct_flow.rs` 和 `logic/security/` 中，security.rs 只是极小的工具函数归属处。如果 validate_hash_prefix 更适合放在 callback.rs 内，可以在实施时调整。

### handlers/system.rs (~200 行)

**从 main.rs 迁入：**
- `format_duration_human()` 函数
- `schedule_task_name()` 函数
- `schedule_frequency_name()` 函数
- `weekday_label()` 函数
- `timezone_label()` 函数
- `build_custom_schedule_text()` 函数
- `build_custom_schedule_keyboard()` 函数
- `build_custom_day_keyboard()` 函数
- `build_custom_hour_keyboard()` 函数
- `build_custom_minute_keyboard()` 函数
- `build_custom_timezone_keyboard()` 函数
- `build_cron_from_custom_state()` 函数
- `save_config()` 异步函数

### app/lifecycle.rs (~80 行)

**从 main.rs 迁入：**
- `notify_online()` 异步函数
- `notify_upgrade_success()` 异步函数
- `notify_bbr3_reboot_result()` 异步函数

**新增：**
- `pub async fn run_startup_checks(bot: &Bot, admin_id: i64)` 统一入口，依次调用上述三个函数

### handlers/mod.rs

```rust
pub mod callback;
pub mod command;
pub mod proxy;
pub mod security;
pub mod system;

pub use callback::handle_callback;
pub use command::{handle_command, handle_message};
```

### main.rs 瘦身后 (~150 行)

```rust
mod app;
mod bootstrap;
mod handlers;

use handlers::{handle_callback, handle_command, handle_message};
// ... 精简的 use 语句
// TOTP 常量保留
// main() 函数保留（含 Dispatcher 组装）
// #[cfg(test)] mod tests 保留
```

## 实施阶段

### 阶段 1：建立骨架 (Scaffolding)
1. 创建 `src/handlers/` 目录
2. 创建 `handlers/mod.rs` 及 5 个空文件
3. 创建 `app/lifecycle.rs` 空文件
4. 更新 `app/mod.rs` 加 `pub mod lifecycle`
5. 更新 `main.rs` 加 `mod handlers`
6. `cargo check` 通过

### 阶段 2：迁移 lifecycle (Lifecycle Extraction)
1. 将 `notify_online`, `notify_upgrade_success`, `notify_bbr3_reboot_result` 剪切到 `app/lifecycle.rs`
2. 创建 `run_startup_checks` 统一入口
3. 更新 `main.rs` 的 `use` 和调用
4. `cargo check` + `cargo test` 通过

### 阶段 3：迁移工具函数 (Utility Extraction)
1. 将 `format_duration_human` 等辅助函数剪切到 `handlers/system.rs`
2. 将 `show_reality_*` 系列剪切到 `handlers/proxy.rs`
3. 将 `validate_hash_prefix` 剪切到 `handlers/security.rs`
4. 将 `escape_html` 剪切到 `handlers/callback.rs`
5. 逐步 `cargo check` 确保每个迁移后编译通过

### 阶段 4：迁移 command handlers
1. 将 `Command` 枚举、`handle_command`、`handle_message`、`process_auth_code`、`send_main_menu`、`looks_like_totp_code` 剪切到 `handlers/command.rs`
2. 更新 `main.rs` 的 use 语句和 Dispatcher 引用
3. `cargo check` + `cargo test` 通过

### 阶段 5：迁移 callback handler (核心步骤)
1. 将 `handle_callback` 整体剪切到 `handlers/callback.rs`
2. 将巨型 match 内的每个分支提取为独立函数（如 `handle_m_main(bot, chat_id, msg_id) -> Result<CallbackAction>`），match arm 变为单行调用
3. 回调函数内部使用 `continue` 和 `break` 控制流，需通过返回值模拟（如 `enum CallbackAction { Continue, Break }`）
4. `cargo check` 确保通过

### 阶段 6：可见性修复与最终验证
1. 逐步添加 `pub(crate)` 到需要跨模块访问的项
2. `cargo check` 通过
3. `cargo test` 通过
4. 清理 `main.rs` 中无用的 `use` 语句

## 关键技术细节

### handle_callback 的 loop + continue 模式

`handle_callback` 使用 `loop` + `continue` 模式处理某些多步交互。提取子函数时，需要用一个返回枚举来模拟：

```rust
enum CallbackOutcome {
    Continue,  // 原来的 continue
    Done,      // 原来的 break Ok(())
}
```

子函数签名如：
```rust
async fn handle_m_main(bot: &Bot, chat_id: ChatId, msg_id: MessageId) -> Result<CallbackOutcome> { ... }
```

主路由改为：
```rust
match data.as_str() {
    "m_main" => handle_m_main(&bot, chat_id, msg_id).await?,
    // ... 每个分支返回 CallbackOutcome
}
```

但注意：`continue` 在当前巨型函数中使用较少（主要出现在 schedule 相关的 followup 流程），大部分分支以 `break Ok(())` 结束。需要在实施时逐个确认。

### 可见性策略

- 跨模块调用：`pub(crate)`
- 同模块内：保持私有
- 常量在原位置保留（如 TOTP_FAIL_MAX 仍在 main.rs 或移到 handlers/command.rs）

## 验证标准

1. `cargo check` 无错误
2. `cargo test` 全部通过（包括 `self_destruct_trigger_uses_executor_boundary` 和 `format_duration_human_*` 测试）
3. `main.rs` 行数 < 200 行
4. 无任何文件超过 1,500 行
5. 所有业务逻辑不变（git diff 仅显示文件搬迁 + 可见性调整）
