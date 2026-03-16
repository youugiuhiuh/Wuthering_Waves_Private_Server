---
name: wwps-tgbot-map
description: >
  提供 rust/tgbot (WWPS Telegram Bot) 的完整项目图谱与流程梳理。在需要理解或修改 tgbot 代码结构、请求路由、启动流程、认证与自毁、Reality/配置、调度器、Bot 自更新与 wwps-core 升级时使用。适用于代码导航、重构、排错与新人上手。
---

# rust/tgbot 项目图谱与流程

本 Skill 提供 **rust/tgbot** 的模块结构图与端到端流程说明，便于快速定位入口与数据流。

## 何时查阅

- 需要找“某功能在哪个文件/哪个函数”时 → 先看 [references/project-map.md](references/project-map.md)。
- 需要理清“从启动到回调、从菜单到具体逻辑”的完整路径时 → 看 [references/flows.md](references/flows.md)。
- 分析阻塞/死锁或并发问题时 → 见项目内 `rust/tgbot/docs/BLOCKING_ANALYSIS.md`。

## 核心入口一览

| 入口 | 位置 | 说明 |
|------|------|------|
| 进程 main | `src/main.rs` `main()` | harden → verify_integrity → CLI 或 加载 config → AppState → Dispatcher |
| 命令 | `handle_command` | /start, /help, /menu, /auth, /setsecurityfile |
| 消息 | `handle_message` | 仅 admin；定时/Warp 输入态、自毁流程消息 |
| 回调 | `handle_callback` | 菜单与操作分支（m_main, m_usr, a_* 等） |
| 自毁消息/回调 | `app/destruct_flow.rs` | handle_message_flow, handle_callback_timeout, handle_callback_action |
| 配置与密钥 | `bootstrap.rs` + `logic/security.rs` | CONFIG_DIR, .key, config.enc, run_setup, SecurityManager |
| 状态 | `app/state.rs` | AppState：会话、失败次数、自毁态、定时/Warp 输入 |

## 关键流程速查

- **启动**：config.enc 存在且 .key 不存在会直接报错；TOTP 解密后做 trim 再交给 TotpManager；调度器与通知在后台 spawn，不阻塞 /start。
- **认证**：TOTP 验证 → record_auth_success / record_auth_failure；失败次数与锁定时长见 main 中 LOCKOUT_DURATIONS。
- **Reality 批量**：用户管理 → 选协议与 IP 版本（IPv4 / IPv6 / 双栈分离 v6上v4下 / v4上v6下） → 选数量 → 回调 u_batch_exec: 等 → ConfigManager::batch_create_* → SNISelector + 端口放行 + 写配置。
- **自毁**：危险区域 → begin_destruct → 两轮 TOTP + 确认 + 文件 SHA256 校验 → executor（wipe_targets + 服务与自毁脚本）。
- **调度器**：start_scheduler 在后台执行；get_manager() 取 Arc<SchedulerManager>；任务持久化在 scheduler_state.json。

## 参考文件

- **[references/project-map.md](references/project-map.md)**：目录树、模块表、静态/全局、配置路径、测试分布。
- **[references/flows.md](references/flows.md)**：启动、命令、消息、回调、Reality、调度器、自毁、自更新与 wwps-core、配置持久化、阻塞注意点。
- **测试覆盖与规范**：见 **tgbot-testing** Skill 与 `rust/tgbot/docs/TEST_COVERAGE.md`；补 CLI/bootstrap/安装器测试时按其中约定（如 TGBOT_CONFIG_DIR、stdout 行数断言）执行。

修改或扩展功能时，按“入口 → state/flow → logic 模块”顺序追踪即可。
