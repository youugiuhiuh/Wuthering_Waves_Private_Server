# Implementation Plan: XDRIVE 按服务商/存储平台重构与 Google Drive 深度集成

## Overview
全面重构 XDRIVE 菜单为服务商分类模式（Cloudflare R2, Google Drive, Yandex S3, ArvanCloud S3, 自定义 S3），并依据官方指南接入 Google Drive 深度集成。

## Tasks

### Task 1: Google Drive 数据模型与调优预设 (XdriveConfig Google Drive Integration)
- **Files**: `rust/aegis/src/core/xray/xdrive.rs`
- **Description**: 实现 `new_google_drive` 预设，注入指南推荐参数，完善 `validate()` 检查与智能全文提取器。
- **Verification**: `cargo test --lib core::xray::xdrive`

### Task 2: Google Drive 客户端链接生成与域前置 (Google Drive Client Link with Domain Fronting)
- **Files**: `rust/aegis/src/core/xray/config.rs`
- **Description**: 实现 Google Drive 的 `google.com:443` 域前置客户端链接生成，验证架构与链接格式。
- **Verification**: `cargo test --lib core::xray::config`

### Task 3: 服务商主菜单与文案重构 (i18n & Menu Restructuring)
- **Files**: `rust/aegis/src/resources/i18n/{zh.yml,en.yml,ja.yml}`, `rust/aegis/src/shared/handlers/xray.rs`
- **Description**: 重构主菜单为 5 大平台选项，添加 Google Drive 保姆级指引与分步文案。
- **Verification**: `cargo test --lib shared::handlers`

### Task 4: 消息监听器集成 Google Drive 问答向导 (Message Handler Integration)
- **Files**: `rust/aegis/src/shared/handlers/message.rs`, `rust/aegis/src/app/state.rs`
- **Description**: 增加 Google Drive 分步问答（Folder ID -> Client ID -> Client Secret -> Refresh Token）与一键提取。
- **Verification**: `cargo test --lib shared::handlers::message`

### Task 5: 全流程门禁与回归验证 (Quality Gates)
- **Description**: 执行 `rust-lint-format` 门禁（fmt, clippy, nextest, doc）。
- **Verification**: `cargo nextest run --cargo-profile fast-test`
