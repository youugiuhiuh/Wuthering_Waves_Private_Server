# Task Breakdown & Checklist

- [x] Task 1: Core Xdrive Model & Template Generator in `rust/aegis/src/core/xray/xdrive.rs`
  - Acceptance: `ONEDRIVE_DEFAULT_CLIENT_ID` 和 `ONEDRIVE_DEFAULT_CLIENT_SECRET` 常量就绪；`new_onedrive(refresh_token, folder)` 默认填充公开凭据；`generate_onedrive_template_value()` 输出合规 JSON；单元测试全绿。
  - Verify: `cargo test --package aegis --lib core::xray::xdrive::tests`
  - Files: `rust/aegis/src/core/xray/xdrive.rs`

- [x] Task 2: Xray Config JSON Serialization in `rust/aegis/src/core/xray/config.rs` (Parallel Group A)
  - Acceptance: `build_xdrive_inbound` 当 service 为 `OneDrive` 时自动将 `service` 转译为 `"template"` 并注入 `template` JSON 对象；单元测试全绿。
  - Verify: `cargo test --package aegis --lib core::xray::config::tests`
  - Files: `rust/aegis/src/core/xray/config.rs`

- [x] Task 3: Wizard Token Auto-Detection & Old Azure Prompt Cleanup in `rust/aegis/src/shared/handlers/message.rs` (Parallel Group A)
  - Acceptance: `extract_onedrive_tokens` 在仅含 `refresh_token` 的 JSON/文本中自动补充默认 Client ID/Secret；清理旧版 Azure App ID 文案提示。
  - Verify: `cargo test --package aegis --lib shared::handlers::message`
  - Files: `rust/aegis/src/shared/handlers/message.rs`

- [x] Task 4: End-to-End Integration Tests & Quality Gates
  - Acceptance: 新增 `tests/xdrive_onedrive_template_test.rs` 测试端到端构造、序列化与解析；`cargo fmt`、`cargo clippy`、`cargo test` 全量通过。
  - Verify: `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
  - Files: `rust/aegis/tests/xdrive_onedrive_template_test.rs`
