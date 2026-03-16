# rust/tgbot 项目结构图谱

## 顶层目录

```
rust/tgbot/
├── src/
│   ├── main.rs          # 二进制入口：main、CLI、Dispatcher、handle_command/handle_message/handle_callback
│   ├── lib.rs           # 仅 pub mod logic；供 main 与 tests 使用
│   ├── bootstrap.rs     # 配置目录、加密配置、run_setup/verify_integrity、BotSettings
│   ├── app/             # 会话与流程状态、认证、自毁流程
│   └── logic/           # 业务逻辑（配置、安装、升级、调度、安全等）
├── tests/               # 集成/单元测试
├── examples/            # 示例（如 test_sni）
├── docs/                # 文档（如 BLOCKING_ANALYSIS.md）
└── src/resources/sni/   # 内嵌 SNI 资源（reality/xhttp 按国家）
```

## src/ 模块树

| 路径 | 职责 |
|------|------|
| **main.rs** | `main()` → harden_process → verify_integrity → CLI(--setup/--setup-stdin/--generate-totp-secret) 或 加载 config → 创建 AppState → 注册命令 → 构建 handler → 后台 start_scheduler + notify_* → Dispatcher.dispatch()。`handle_command`(Command)、`handle_message`(文本/文档)、`handle_callback`(回调 data 分支)。`send_main_menu`、`format_duration_human`、`save_config` 等。 |
| **lib.rs** | `pub mod logic`。 |
| **bootstrap.rs** | `CONFIG_DIR`(/etc/wwps/tgbot)、`KEY_FILE`(.key)、`CONFIG_FILE`(config.enc)、`EncryptedConfig`、`BotSettings`、`run_setup`/`run_setup_from_stdin`、`verify_integrity`。 |
| **app/state.rs** | `AppState`：sessions、failed_attempts、pending_destructs、self_destruct_key_hash、pending_warp_inputs、pending_schedule_inputs、session_timeout_secs。认证/自毁/定时输入/Warp 输入 等状态方法。 |
| **app/auth.rs** | TOTP 认证流程（process_auth_code 等），供 main 调用。 |
| **app/destruct_flow.rs** | `handle_message_flow`、`handle_callback_timeout`、`handle_callback_action`：自毁流程消息与回调处理，步骤 TOTP→确认→第二 TOTP→文件校验→执行。 |
| **logic/anti_debug.rs** | 防调试检查（main 启动时调用）。 |
| **logic/cmd_async.rs** | `run_cmd_output`/`run_cmd_status`/`run_cmd_checked`/`run_cmd_stream`：超时包装的 tokio::process::Command。 |
| **logic/config.rs** | `ConfigManager`：Reality/Vision/XHTTP 配置生成、批量创建、公网解析、SNI 选择、链接生成、独立/合并配置写入。 |
| **logic/fail2ban.rs** | fail2ban 安装/启用。 |
| **logic/firewall.rs** | 防火墙检测（nftables/iptables）。 |
| **logic/firewall_scanner.rs** | 端口/监听扫描（ss/netstat）、规则解析。 |
| **logic/firewalld.rs** | firewalld 相关。 |
| **logic/geoip.rs** | GeoIP 服务（国家码等）。 |
| **logic/installer.rs** | `RealityInstaller`/`RealityInstallerInternal`（PROGRESS_STATE）、`WarpInstaller`、`install_wwps_core`、`install_wwps_core_service`、包管理器检测、run_command。 |
| **logic/maintenance.rs** | `MaintenanceManager`：BBR3、Geo 更新、服务控制、防火墙加固、自毁目标擦除(wipe_targets)、系统信息等。 |
| **logic/operations.rs** | 重启、apt 更新等运维操作。 |
| **logic/scheduler/mod.rs** | `SchedulerManager`、`SCHEDULER` 全局、`get_manager`/`start_scheduler`、cron 任务启停、状态持久化(scheduler_state.json)。 |
| **logic/scheduler/task_types.rs** | `TaskType`(GeoUpdate 等)、`ScheduledTask`、执行逻辑。 |
| **logic/security.rs** | `SecurityManager`(.key 读写、AES-GCM 加解密)、`secure_wipe_path`。 |
| **logic/self_destruct.rs** | 自毁执行入口 `trigger`，调用注入的 `SelfDestructExecutor`。 |
| **logic/sni_selector.rs** | `SNISelector::get_for_country`，内存缓存 + 内嵌资源，next() 轮询域名。 |
| **logic/system.rs** | `SystemMonitor`：公网 IP(v4/v6)、服务状态、系统状态报告(spawn_blocking)。 |
| **logic/totp.rs** | `TotpManager::new`/`verify`/`generate_new_secret`。 |
| **logic/ufw.rs** | UFW 命令封装（UFW_MUTEX 串行化）。 |
| **logic/upgrade.rs** | `UpgradeManager`：自更新(Release API、下载、self_replace)、升级标记文件。 |
| **logic/upgrade/wwps_core/** | wwps-core 升级：fetch_release、download、extract、replace、restart_service。 |
| **logic/utils.rs** | `human_readable_size`、`format_download_progress`、`should_report`。 |
| **logic/warp_api.rs** | Warp API 交互。 |

## 关键静态/全局

- `PROGRESS_STATE`(installer)：Reality 安装进度。
- `UFW_MUTEX`(ufw)：UFW 串行。
- `SCHEDULER`(scheduler)：全局调度器 Manager。
- `SNI_CACHE`(sni_selector)：std RwLock 缓存（注意阻塞风险，见 docs/BLOCKING_ANALYSIS.md）。

## 配置与路径

- 配置目录：`/etc/wwps/tgbot`（CONFIG_DIR）。
- `.key`：32 字节密钥；不存在时 SecurityManager::new 会创建（若 config.enc 已存在则 main 先检查并报错）。
- `config.enc`：加密后的 token、admin_id、totp_secret、self_destruct_key_hash。
- `bot_settings.json`：session_timeout_secs 等。
- 调度器状态：`/etc/wwps/tgbot/scheduler_state.json`。
- 升级标记：`UPGRADE_FLAG_FILE`；BBR3：`BBR3_PENDING_FLAG_FILE`。

## 测试

- **tests/test_self_destruct.rs**：自毁状态机、超时、SHA256、secure_wipe 单元测试。
- **tests/test_self_destruct_e2e.rs**：沙盒 E2E 擦除流程。
- **tests/integration_totp_trim.rs**：TOTP 换行/trim 集成。
- **tests/integration_security.rs**：SecurityManager 加解密往返。
- **app/state.rs**、**logic/** 下多模块带 `#[cfg(test)]` 单元测试。
