# Implementation Plan: XDRIVE 国际版 AWS S3 预设与纯国际服务商体系

## Overview
全面排除境内中国厂商，接入 AWS S3 国际版专属预设与问答向导，完善纯国际顶级云存储中转体系。

## Tasks

### Task 1: AWS S3 数据模型与预设构建器 (XdriveConfig AWS S3 Integration)
- **Files**: `rust/aegis/src/core/xray/xdrive.rs`
- **Description**: 实现 `new_aws_s3(region, bucket, access_key_id, secret_access_key, folder)`，设置 `path_style: false`，并提供 `extract_aws_credentials` 智能解析。
- **Verification**: `cargo test --lib core::xray::xdrive`

### Task 2: 国际服务商菜单与文案完善 (i18n & Menu Restructuring)
- **Files**: `rust/aegis/src/resources/i18n/{zh.yml,en.yml,ja.yml}`, `rust/aegis/src/shared/handlers/xray.rs`
- **Description**: 增加 `u_xdrive_p:aws` 按钮，补充 AWS S3 控制台开通指引与常用区域列表（东京、新加坡、美东）。
- **Verification**: `cargo test --lib shared::handlers`

### Task 3: 消息监听器集成 AWS S3 问答向导 (Message Handler Integration)
- **Files**: `rust/aegis/src/shared/handlers/message.rs`
- **Description**: 在 `handle_xdrive_wizard_input` 中接通 AWS S3 分步流程（Region -> Bucket -> AK -> SK），并支持一键粘贴提取。
- **Verification**: `cargo test --lib shared::handlers::message`

### Task 4: 全流程门禁与回归验证 (Quality Gates)
- **Description**: 执行 `rust-lint-format` 门禁（fmt, clippy, nextest, doc）。
- **Verification**: `cargo nextest run --cargo-profile fast-test`
