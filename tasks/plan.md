# Implementation Plan: XDRIVE 存储提供商按地区分类导航重构

## Overview
重构 Telegram Bot 的 XDRIVE 存储提供商选择流程，从扁平 9 项单屏列表改为两级“地区专区”导航体系：
- 一级菜单：中国大陆 (CN)、俄罗斯 (RU)、伊朗 (IR)、国际通用 (Global)、自定义 S3 兼容存储、返回。
- 二级子菜单：
  - CN：Cloudflare R2 / Backblaze B2 / Microsoft OneDrive
  - RU：Yandex Cloud S3 / Google Drive
  - IR：ArvanCloud S3 / Google Drive
  - Global：AWS S3 / Oracle Cloud S3
- 各二级菜单支持返回一级地区菜单，点击各具体提供商无缝触发原有向导流程。

## Architecture Decisions
1. **统一路由前缀**:
   - 地区子菜单路由采用 `u_xdrive_r:<region>`，其中 `cn`, `ru`, `ir`, `global` 展开二级菜单，`menu` 返回一级菜单。
   - 提供商路由维持原有的 `u_xdrive_p:<provider>`，保持完全向后兼容与向导状态机一致。
2. **三语本地化规范**:
   - 同步在 `zh.yml`, `en.yml`, `ja.yml` 中新增一级地区按钮、二级子菜单标题及返回文案。
3. **测试驱动安全**:
   - 先行添加/更新测试断言验证红灯（RED），实施代码转绿（GREEN），代码精简重构（REFACTOR）。

## Global Constraints
- 严格保持 `u_xdrive_p:<provider>` 路由行为和入参向导流程完全不变。
- zh/en/ja 语言包必须完整补齐，不能遗漏任何语言文件。
- 全部测试必须通过：`cargo test`、`cargo clippy`、`cargo fmt`。

## Task 1: i18n Localization Keys in zh.yml, en.yml, ja.yml
**Description:** 在 `zh.yml`、`en.yml`、`ja.yml` 中新增地区入口按钮文案、二级子菜单标题文案以及返回地区菜单文案。
- 地区按钮：`xdrive_region_cn`、`xdrive_region_ru`、`xdrive_region_ir`、`xdrive_region_global`
- 子菜单标题：`xdrive_submenu_cn_title`、`xdrive_submenu_ru_title`、`xdrive_submenu_ir_title`、`xdrive_submenu_global_title`
- 返回上一级按钮：`xdrive_back_to_regions` 或类似键值

**Acceptance criteria:**
- `zh.yml`, `en.yml`, `ja.yml` 均包含全部新增键值，翻译地道准确。
- 语言包键值对齐测试通过。

**Verification:**
- `cargo test --package aegis --lib resources`

**Files:**
- `rust/aegis/src/resources/i18n/zh.yml`
- `rust/aegis/src/resources/i18n/en.yml`
- `rust/aegis/src/resources/i18n/ja.yml`

## Task 2: TDD Red Phase — Unit Tests for Region Menu and Submenu in xray.rs
**Description:** 在 `rust/aegis/src/shared/handlers/xray.rs` 的测试模块中编写针对一级地区菜单和二级子菜单的断言用例。
- 编写 `test_xdrive_region_menu_renders_regions`，验证渲染 5 个主按钮（CN/RU/IR/Global/S3）和 1 个返回按钮。
- 编写 `test_xdrive_region_submenus`，分别测试 `cn`, `ru`, `ir`, `global` 渲染对应的存储提供商与返回上一级按钮。
- 编写回调路由测试，测试 `u_xdrive_r:cn` 等路由行为。
- 执行测试，验证在新功能实施前测试用例呈红灯（RED）。

**Acceptance criteria:**
- 完整包含一级菜单与四个二级子菜单的测试用例。
- 运行测试期望失败（RED），证明测试用例有效且非假阳性。

**Verification:**
- `cargo test --package aegis --lib shared::handlers::xray::tests::test_xdrive_region` (FAIL)

**Files:**
- `rust/aegis/src/shared/handlers/xray.rs`

## Task 3: Green & Refactor Phase — Implement Region Menu & Submenu Navigation in xray.rs
**Description:** 实施一级地区菜单和二级子菜单的渲染与事件分发。
- 重构 `handle_xdrive_menu` 为一级地区专区选择菜单（CN/RU/IR/Global/S3/返回）。
- 新增 `handle_xdrive_region_submenu(event: &CallbackEvent, region: &str) -> HandlerResult` 渲染对应地区的存储提供商。
- 在 `handle()` 的 callback 分发中，匹配 `u_xdrive_r:menu` 回退一级菜单，匹配 `u_xdrive_r:cn` / `ru` / `ir` / `global` 进入子菜单，原有 `u_xdrive_cancel` 取消后返回一级菜单。
- 运行测试用例全部转绿（GREEN），进行格式化和代码精简（REFACTOR）。

**Acceptance criteria:**
- Task 2 编写的所有测试用例 100% 绿灯。
- 原有 XDRIVE 相关测试不退化。

**Verification:**
- `cargo test --package aegis --lib shared::handlers::xray::tests`

**Files:**
- `rust/aegis/src/shared/handlers/xray.rs`

## Task 4: Full Quality Gates & Clean Baseline Verification
**Description:** 执行全量质量门禁检查和集成测试。
- 执行 `cargo fmt --check`
- 执行 `cargo clippy --workspace --all-targets -- -D warnings`
- 执行 `cargo nextest run` 全量测试验证

**Acceptance criteria:**
- 格式、Clippy、Nextest 全量无警告无报错。

**Verification:**
- `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run`

**Files:**
- All workspace files
