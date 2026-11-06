# Implementation Plan: XDRIVE 傻瓜式逐步问答向导与保姆级开通指引

## Overview
将 XDRIVE 配置流程由一次性多行输入彻底重构成逐步问答式向导（Conversational Wizard），并在第一屏附带官方控制台直达链接和 30 秒极速开通指引。

## Tasks

### Task 1: 会话状态机与会话管理 (Session State in AppState)
- **Files**: `rust/aegis/src/app/state.rs`
- **Description**: 定义 `XdriveWizardStep` 与 `XdriveWizardSession`，在 `AppState` 中增设线程安全的会话存储，实现会话启动、推进、获取、超时清理与取消。
- **Verification**: `cargo test --lib app::state`

### Task 2: 保姆级引导文案与多语言词条 (i18n Resources)
- **Files**: `rust/aegis/src/resources/i18n/{zh.yml,en.yml,ja.yml}`
- **Description**: 增加各云厂商控制台直达链接、零门槛开通指引、分步提问（账户 ID、Bucket、AK、SK）以及完成提示。
- **Verification**: `cargo check`

### Task 3: 地区向导初始化与取消处理 (UI Init & Cancel)
- **Files**: `rust/aegis/src/shared/handlers/xray.rs`
- **Description**: 点击地区后初始化会话并渲染第 1 步与取消按钮；点击取消按键即时清理会话。
- **Verification**: `cargo test --lib shared::handlers::xray`

### Task 4: 消息监听分流与端到端节点生成 (Message Handler Integration)
- **Files**: `rust/aegis/src/shared/handlers/message.rs`
- **Description**: 在消息主循环拦截 XDRIVE 会话，执行分步校验、推进提问；最后一步调用 `ConfigManager::create_xdrive_node` 落盘并下发链接。
- **Verification**: `cargo test --lib shared::handlers::message`

### Task 5: 全流程门禁与回归验证 (Quality Gates)
- **Description**: 执行 `rust-lint-format` 门禁（fmt, clippy, nextest, doc）。
- **Verification**: `cargo nextest run --cargo-profile fast-test`
