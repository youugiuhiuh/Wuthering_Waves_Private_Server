# Todo: XDRIVE 微软国际版 OneDrive 深度集成

- [x] Task 1: Core Xdrive 数据模型与构造器 (xdrive.rs)
  - Acceptance: `XdriveService::OneDrive` 完备，`new_onedrive` 预设参数正确注入 (concurrency=8, 512KB 分片)，`validate()` 严格校验，单测通过
  - Verify: `cargo test --lib core::xray::xdrive`
  - Files: `rust/aegis/src/core/xray/xdrive.rs`

- [x] Task 2: OneDrive 客户端 VLESS 链接生成与 SNI 域前置 (config.rs)
  - Acceptance: `generate_xdrive_client_link` 正确生成 `graph.microsoft.com:443` 与 `alpn=h2` 订阅串，单测覆盖
  - Verify: `cargo test --lib core::xray::config`
  - Files: `rust/aegis/src/core/xray/config.rs`

- [x] Task 3: Telegram 交互向导与多语言提示 (i18n, state.rs, xray.rs, message.rs)
  - Acceptance: 主菜单展示 OneDrive 服务商，分步向导（Folder -> Client ID -> Client Secret/可选 -> Refresh Token）完整支持，防 24h 过期引导提示完备
  - Verify: `cargo test --lib shared::handlers`
  - Files: `rust/aegis/src/resources/i18n/{zh.yml,en.yml,ja.yml}`, `rust/aegis/src/app/state.rs`, `rust/aegis/src/shared/handlers/xray.rs`, `rust/aegis/src/shared/handlers/message.rs`

- [x] Task 4: 全流程质量审查与门禁
  - Acceptance: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run --cargo-profile fast-test` 全部通过 (1363 passed)
  - Verify: 全部门禁绿色，Reviewer 子代理裁决 APPROVED
  - Files: Global
