# rust/tgbot 完整流程梳理

## 1. 进程启动 (main)

1. **harden_process()**：进程加固。
2. **anti_debug::check_debugger()**：防调试，异常则退出。
3. **verify_integrity()**：校验可执行文件完整性。
4. **CLI 分支**（若带参数）：
   - `--setup <token> <admin_id> <totp_secret>`：bootstrap::run_setup（trim 后加密写入 config.enc，.key 由 SecurityManager 创建）。
   - `--setup-stdin`：从 stdin 读 JSON，run_setup。
   - `--generate-totp-secret`：打印 TotpManager::generate_new_secret() 后退出。
   - `-v`/`--version`：打印版本后退出。
5. **正常启动**：
   - 若 `config.enc` 存在且 `.key` 不存在 → 直接 bail，提示一并部署或本机 --setup。
   - **SecurityManager::new**(key_path) 读/创建 .key。
   - 读 **config.enc**，解密 token、admin_id、totp_secret（**totp_secret 做 trim**），解析 admin_id。
   - **TotpManager::new**(totp_secret)，失败则 log 并退出。
   - **BotSettings::load()**，**AppState::new**(admin_id, totp_manager, production_executor(), self_destruct_key_hash, session_timeout_secs)。
   - **Bot::new**(token)，**register_bot_commands**。
   - 构建 **handler**：三支
     - `Update::filter_message().filter_command::<Command>()` → **handle_command**
     - `Update::filter_message()` → **handle_message**
     - `Update::filter_callback_query()` → **handle_callback**
   - **tokio::spawn** 后台：start_scheduler(bot, admin_id) → notify_upgrade_success → notify_bbr3_reboot_result → notify_online。
   - **Dispatcher::builder(bot, handler).dependencies(deps![state]).dispatch().await**。

## 2. 命令处理 (handle_command)

- **Command**：Help、Start、Menu、Auth(code)、SetSecurityFile。
- **Start**：直接 send_message 欢迎 + TOTP 提示。
- **Help**：发送 Command::descriptions()。
- **Auth**：process_auth_code(bot, chat_id, user_id, code, state)。
- **SetSecurityFile**：需已认证；从消息/回复中取 document 或 photo file_id → get_file + download_file → SHA256 → state.set_self_destruct_key_hash + save_config。
- **Menu**：未认证则提示 TOTP；否则 send_main_menu(bot, chat_id)。

## 3. 消息处理 (handle_message)

- 仅处理 **is_admin_user(user_id)** 的用户。
- **schedule_timeout_status**：若处于定时任务输入态且超时 → 移除并提示；若 Active 且为文本/文件 → 提示通过面板操作。
- **take_warp_input_status**：Warp 输入态同理。
- 其余：**destruct_flow::handle_message_flow**（自毁步骤中的 TOTP/确认/文件等）→ 若 Handled 则结束；否则根据当前 state 处理 Warp、定时、配置删除等流程。

## 4. 回调处理 (handle_callback) 主分支

- 未认证 → answer_callback_query 会话过期。
- **destruct_flow::handle_callback_timeout**（自毁超时取消）→ 若 Handled 则 return。
- 自定义定时相关(s_custom_ui/s_custom_set/s_custom_confirm/cancel)：检查 schedule 180s 超时，可能递归到 s_add_custom_menu。
- **destruct_flow::handle_callback_action**（自毁按钮：a_destroy_confirm/cancel、步骤推进等）→ 若 Handled 则 return。
- **match data.as_str()** 主要入口：
  - **m_main**：主菜单（状态监控、用户管理、运维中心、系统设置）。
  - **m_ops_center**：运维中心（网络优化、安全防护、系统指令、日志审计）。
  - **m_settings**：系统设置（核心管理、定时任务、Geo、Bot 更新、会话有效期、危险区域）。
  - **m_usr**：用户管理 → Reality 批量(Vision/XHTTP)、Warp、订阅等。
  - **m_mon**：状态监控。
  - **a_wwps_core_menu** / **wwps_core_tag**：wwps-core 版本与升级。
  - **a_upgrade**：Bot 自更新（UpgradeManager::run）。
  - **m_sched** / **a_geo_sched_menu** 等：定时任务与 Geo 调度。
  - **a_geo**：GeoData 更新（MaintenanceManager::update_geodata）。
  - **a_fw**：防火墙加固（MaintenanceManager::harden_firewall，spawn + 45s 超时）。
  - **u_batch_ip_init:** / **u_batch_exec:** 等：Reality 批量 IP 选择与执行（ConfigManager::batch_create_*）。
  - **s_add** / **s_del** 等：定时任务增删。
  - **m_danger**：危险区域 → 自毁流程入口（destruct_flow）。
- 大量以 **data.starts_with("...")** 的分支：如 `u_batch_exec:`、`cfg_del_*`、`s_custom*`、`wwps_core_tag:` 等，需看 main.rs 中 match 顺序与前缀。

## 5. Reality 安装与配置流程

- 用户管理 → 选择 Reality(Vision) 或 XHTTP → **show_reality_batch_prompt**（IPv4/IPv6/双栈）→ **show_reality_qty_prompt**（数量）→ 回调 **u_batch_exec:** 或 **u_xhttp_batch_exec:**。
- **ConfigManager::batch_create_reality_enhanced** / **batch_create_xhttp_reality_enhanced**：解析 IP 版本、公网 IP、GeoIP 国家、**SNISelector::get_for_country**、循环 generate_enhanced_config + build_reality_vless_inbound + generate_client_link + MaintenanceManager::allow_port；最后 create_standalone_config 或 update_existing_config。
- 若需“安装 Reality 运行环境”（sing-box/xray 等）：**RealityInstaller::execute_reality_install**（PROGRESS_STATE 锁、步骤更新、installer.execute()）。

## 6. 调度器 (scheduler)

- **start_scheduler(bot, admin_id)**：SchedulerState::load_from_file（spawn_blocking 读 scheduler_state.json）→ JobScheduler::new → SchedulerManager::start_all_tasks → 写入 **SCHEDULER** 全局。
- 定时任务执行：task_types::TaskType::execute（如 GeoUpdate）。
- 添加/删除任务：get_manager() → manager.add_new_task / remove_task 等，持久化 state_path。

## 7. 自毁流程 (destruct_flow)

- 入口：危险区域 → **begin_destruct**（state.pending_destructs 插入 AwaitFirstTotp）。
- **handle_message_flow**：touch_destruct 超时则取消；否则按 step：AwaitFirstTotp（收 TOTP → confirm_first_destruct_totp）→ AwaitConfirm（按钮）→ AwaitSecondTotp（第二 TOTP，防重放）→ AwaitFile（上传文件，SHA256 与 self_destruct_key_hash 比对）→ AwaitFinalConfirm。
- **handle_callback_action**：a_destroy_confirm → 调用 state 的 executor（production_executor → MaintenanceManager::wipe_targets + 服务停止 + 自毁脚本/重启）。
- 擦除目标见 maintenance 与 test_self_destruct_e2e 中的常量。

## 8. Bot 自更新与 wwps-core 升级

- **UpgradeManager::run**：fetch_latest_release（多源）、下载、校验、self_replace、写升级标记、exit。
- **wwps-core**：WwpsCoreUpgradeManager：fetch_release、download_release、extract_archive、replace_core、restart_service（systemctl restart）；可选版本选择（wwps_core_tag:）。

## 9. 配置持久化

- **save_config**：读 config.enc → 解密后更新 self_destruct_key_hash（从 state）→ 再写回 config.enc。
- **BotSettings**：session_timeout_secs 存 bot_settings.json。
- 调度器：scheduler_state.json 由 SchedulerManager 在 start_all_tasks、add、remove 等时写入。

## 10. 阻塞与并发注意点

- 见 **docs/BLOCKING_ANALYSIS.md**：sni_selector 的 std::sync::RwLock 在 async 路径易阻塞；UFW_MUTEX 持锁时间过长；config.enc 存在但 .key 缺失时不再自动建 key。
