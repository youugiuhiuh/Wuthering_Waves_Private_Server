# Implementation Plan: XDRIVE 个人版 OneDrive Template 集成与旧版 Azure App 废除

## 架构依赖图 (Dependency Graph)
```
Task 1: Core Xdrive Model & Template Generator (rust/aegis/src/core/xray/xdrive.rs)
   │
   ├── Task 2: Xray Config Serialization (rust/aegis/src/core/xray/config.rs) [Parallel Group A]
   │
   └── Task 3: Wizard Token Auto-Detection (rust/aegis/src/shared/handlers/message.rs) [Parallel Group A]
           │
           └── Task 4: E2E Integration Tests & Quality Gates (rust/aegis/tests/xdrive_onedrive_template_test.rs)
```

## 任务列表概要
- **Task 1 (核心基础)**: `xdrive.rs` 实现默认凭据常量、`new_onedrive` 构造重构、`generate_onedrive_template_value()` 模板生成与校验逻辑。
- **Task 2 (配置序列化)**: `config.rs` 拦截 `XdriveService::OneDrive`，输出 `service: "template"` 与对应 JSON 模板。
- **Task 3 (向导解析优化)**: `message.rs` 升级 `extract_onedrive_tokens`，自动注入公共 Client ID/Secret 并清理 Azure App 旧提示。
- **Task 4 (集成测试与门禁)**: 编写独立集成测试 `tests/xdrive_onedrive_template_test.rs`，执行 `cargo fmt`、`cargo clippy`、`cargo test` 全量检查。

## 并发安全策略
- Task 2 与 Task 3 严格隔离文件写入（`config.rs` 与 `message.rs`），在 Task 1 成功落地后并发派发。
