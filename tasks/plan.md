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

## Task List & Dependency Graph

```
Task 1: i18n 多语言文案 (zh/en/ja)
    │
    ▼
Task 2: TDD 编写菜单渲染与路由单元测试 (RED)
    │
    ▼
Task 3: 实施一级地区菜单与二级子菜单及路由分发 (GREEN & REFACTOR)
    │
    ▼
Task 4: 全量质量门禁与集成测试验证
```

## Risks and Mitigations
| Risk | Impact | Mitigation |
|------|--------|------------|
| 旧回调数据兼容性 | Low | 保留所有 `u_xdrive_p:` 与原有取消逻辑；`u_xdrive_menu` 仍指向一级菜单 |
| 多语言缺失导致运行期报错 | Med | 三语文件同步添加对应键值，并有 i18n parity 单元测试保障 |
