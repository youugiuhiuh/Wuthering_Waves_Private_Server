# Implementation Plan: XDRIVE 微软国际版 OneDrive 深度集成

## Overview
为 XDRIVE 全面引入微软国际版 OneDrive（Microsoft Graph API）作为高隐蔽、大容量网盘中转通道。严格落实官方限流防御（保守并发 8、512KB 分片、50ms 轮询）与 SNI 域前置（`graph.microsoft.com:443`），支持公共客户端免 Secret 与 Web 客户端双重授权。

## Architecture Decisions
1. **服务枚举标识：** `XdriveService::OneDrive`，序列化名称为 `onedrive`。
2. **Secrets 组织标准：** `[client_id, client_secret, refresh_token]`（当 client_secret 留空时使用空字符串占位以兼容免 Secret 公共客户端）。
3. **参数预设：** `concurrency = 8`（严防 429），`segment_bytes = 512KB`，`poll_interval_ms = 50ms`，`max_poll_interval_ms = 300ms`。
4. **客户端连接串：** SNI 与 Address 均指向 `graph.microsoft.com:443`，ALPN 协商 `h2`。

---

## Task List & Concurrency Boundaries

### [Sequential Phase 1: Core 数据模型与构造器]
- **Task 1: Core Xdrive 数据模型与构造器**
  - **Files:** `rust/aegis/src/core/xray/xdrive.rs`
  - **Description:** 实现 `XdriveService::OneDrive` 枚举、`new_onedrive` 预设构造器、合法性校验 `validate()` 以及单元测试。
  - **Verification:** `cargo test --lib core::xray::xdrive`

### [Sequential Phase 2: 客户端订阅链接生成]
- **Task 2: OneDrive 客户端 VLESS 链接生成与 SNI 域前置**
  - **Files:** `rust/aegis/src/core/xray/config.rs`
  - **Description:** 在 `generate_xdrive_client_link` 中支持 OneDrive，配置 `graph.microsoft.com:443` 与 `alpn=h2`，补充单测。
  - **Verification:** `cargo test --lib core::xray::config`

### [Sequential Phase 3: 多语言文案与交互向导]
- **Task 3: Telegram 交互向导与多语言提示**
  - **Files:**
    - `rust/aegis/src/resources/i18n/{zh.yml,en.yml,ja.yml}`
    - `rust/aegis/src/app/state.rs`
    - `rust/aegis/src/shared/handlers/xray.rs`
    - `rust/aegis/src/shared/handlers/message.rs`
  - **Description:** 增加主菜单 OneDrive 选项、分步引导文案（标注必须选择 Mobile/desktop 应用避免 24h token 过期）、输入解析与状态流转。
  - **Verification:** `cargo test --lib shared::handlers`

### [Checkpoint & Quality Gates]
- **Task 4: 全流程质量审查与门禁**
  - **Description:** 执行 `rust-lint-format` 门禁（`cargo fmt`, `cargo clippy`, `cargo nextest`）。
  - **Verification:** `cargo nextest run --cargo-profile fast-test`
