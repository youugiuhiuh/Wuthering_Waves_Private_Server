# Implementation Plan: XDRIVE Cloudflare R2 极速性能调优与中俄伊全通增强

## 1. 任务拆解与并发依赖边界

### [Parallel Group A: Rust 服务端]
- **Task 1: Core S3 构造器与参数调优** (`rust/aegis/src/core/xray/xdrive.rs`)
  - 依赖：无
  - 范围：`new_cloudflare_r2`, `new_cloudflare_r2_with_endpoint`, `extract_cloudflare_account_id`
  - 测试：`test_cloudflare_r2_optimized_parameters`, `test_cloudflare_r2_with_custom_endpoint`
- **Task 2: Telegram 向导与多语言提示更新** (`rust/aegis/src/shared/handlers/`, `resources/i18n/`)
  - 依赖：Task 1
  - 范围：`message.rs`, `xray.rs`, `zh.yml`, `en.yml`, `ja.yml`
  - 测试：`test_xdrive_wizard_cloudflare_custom_domain`

### [Parallel Group B: C# 客户端]
- **Task 3: aegis_v2rayN 预设优化与单测** (`~/Dark/aegis_v2rayN`)
  - 依赖：无（独立代码库，与 Task 1/2 并发安全）
  - 范围：`AddServerViewModel.cs`, `AddServerViewModelTests.cs`
  - 测试：`dotnet test`

### [Sequential Gate: 质量审查与交付]
- **Task 4: 格式化、Clippy、全量回归与合并**
  - 依赖：Task 1, 2, 3 全部通过
  - 执行：`cargo fmt`, `cargo clippy`, `cargo nextest`, `gh pr create`
