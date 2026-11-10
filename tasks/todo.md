# Task Breakdown & Checklist

- [x] Task 1: i18n Localization Keys in `zh.yml`, `en.yml`, `ja.yml`
  - Acceptance: 在 `zh.yml`、`en.yml`、`ja.yml` 中新增地区入口按钮文案（`xdrive_region_cn`、`xdrive_region_ru`、`xdrive_region_ir`、`xdrive_region_global`）及各二级子菜单标题文案（`xdrive_submenu_cn_title` 等），三语完全对齐。
  - Verify: `cargo test --package aegis --lib resources`
  - Files: `rust/aegis/src/resources/i18n/zh.yml`, `rust/aegis/src/resources/i18n/en.yml`, `rust/aegis/src/resources/i18n/ja.yml`

- [x] Task 2: TDD Red Phase — Unit Tests for Region Menu and Submenu in `xray.rs`
  - Acceptance: 编写测试用例 `test_xdrive_region_menu_renders_regions`、`test_xdrive_region_submenus` 与回调路由测试；在未实施前运行应失败（RED）。
  - Verify: `cargo test --package aegis --lib shared::handlers::xray::tests::test_xdrive_region` (期望 FAIL)
  - Files: `rust/aegis/src/shared/handlers/xray.rs`

- [x] Task 3: Green & Refactor Phase — Implement Region Menu & Submenu Navigation in `xray.rs`
  - Acceptance: 重构 `handle_xdrive_menu` 渲染五大地区入口；新增 `handle_xdrive_region_submenu` 渲染对应地区存储提供商；在 `handle()` 中分发 `u_xdrive_r:` 路由；测试用例全部转绿（GREEN）。
  - Verify: `cargo test --package aegis --lib shared::handlers::xray::tests`
  - Files: `rust/aegis/src/shared/handlers/xray.rs`

- [x] Checkpoint: Tasks 1-3 Functional Verification
  - [x] 地区选择一级菜单渲染正确，包含 CN / RU / IR / Global / 自定义 S3 / 返回
  - [x] CN 子菜单包含 R2 / B2 / OneDrive / 返回
  - [x] RU 子菜单包含 Yandex / Google Drive / 返回
  - [x] IR 子菜单包含 Arvan / Google Drive / 返回
  - [x] Global 子菜单包含 AWS S3 / Oracle Cloud / 返回
  - [x] 返回上一级逻辑与取消向导逻辑顺畅

- [x] Task 4: Full Quality Gates & Clean Baseline Verification
  - Acceptance: `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo nextest run` 100% 绿灯通过。
  - Verify: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run`
  - Files: All workspace files
