# Implementation Plan: Xray 批量菜单聚合与 XDRIVE 地区向导

## Overview
重构 Xray 主管理菜单中的 4 个批量创建按钮为统一入口，并新增 XDRIVE 存储中转多地区（IR, CN, RU, Custom）渐进式表单向导。

## Tasks
1. **Task 1: 多语言文案与地区向导解析器 (i18n & presets)**
   - 文件：`rust/aegis/src/core/xray/xdrive.rs`, `rust/aegis/src/resources/i18n/{zh,en,ja}.yml`
   - 目标：定义地区预设解析器 `parse_region_wizard_input` 并编写单测；添加多语言键条。
2. **Task 2: 主菜单聚合与二级批量子菜单 (menu_aggregation)**
   - 文件：`rust/aegis/src/shared/handlers/xray.rs`
   - 目标：聚合 `handle_mgmt` 按钮，实现 `handle_batch_menu` 路由。
3. **Task 3: XDRIVE 地区选择与向导卡片渲染 (xdrive_ui_handlers)**
   - 文件：`rust/aegis/src/shared/handlers/xray.rs`
   - 目标：实现 `handle_xdrive_menu` 与 `handle_xdrive_region_prompt`，挂载至回调分发器。
4. **Task 4: 端到端节点生成与辅助构建 (wizard_exec)**
   - 文件：`rust/aegis/src/core/xray/xdrive.rs`, `rust/aegis/src/core/xray/config.rs`
   - 目标：实现 `create_xdrive_inbound_config`，生成完整 JSON 文件并导出客户端链接。
5. **Task 5: 质量把关与测试验证 (quality_gate)**
   - `cargo fmt --check`, `cargo clippy -- -D warnings`, 全量单测通过。
