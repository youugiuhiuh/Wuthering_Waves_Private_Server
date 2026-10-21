# Implementation Plan: Xray Geodata 迁移至 XTLS/Xray-rules-dat 官方维护源

> 对应 `tasks/xray-rules-dat-migration/spec.md`。  
> 状态：**已完成并验证通过**。

---

## 1. 任务分解 (Task Breakdown)

### Task 1: 路由规则升级为 `geosite:apple@cn` (TDD)
- **文件路径**：`rust/aegis/src/core/xray/routing.rs`
- **操作**：
  1. 修改 `test_essential_direct_targets_use_explicit_prefix` 与 `test_essential_direct_contains_evidence_backed_hosts` 断言为 `"geosite:apple@cn"`（此时测试红灯 RED）。
  2. 修改 `ROUTING_RULES` 中的 `"geosite:apple-cn"` 为 `"geosite:apple@cn"`，并调整注释。
  3. 运行 `cargo test -p aegis core::xray::routing` 确认测试绿灯（GREEN）。

### Task 2: 切换 geodata 下载源为 XTLS/Xray-rules-dat
- **文件路径**：`rust/aegis/src/core/system/maintenance.rs`
- **操作**：
  1. 更新 `ensure_geodata_in_dir` 中的下载 URL 为 `XTLS/Xray-rules-dat` 的 `geoip-lite.dat` 与 `geosite.dat`。
  2. 更新 `update_geodata` 中的下载 URL 为 `XTLS/Xray-rules-dat` 的 `geoip-lite.dat` 与 `geosite.dat`。
  3. 运行单测确认编译与测试正常。

### Task 3: 核心升级预检前自动规范化基准配置
- **文件路径**：`rust/aegis/src/core/system/core_upgrade.rs`
- **操作**：
  1. 在 `run_upgrade` 解压完成后、执行 `preflight_check_config` 前，增加调用 `RoutingManager::ensure_direct_rules_in_base()`。
  2. 添加单元测试或在既有预检测试中验证迁移逻辑覆盖。
  3. 运行 `cargo test -p aegis core::system::core_upgrade` 验证通过。

### Task 4: 完整质量门覆核与格式检查
- **操作**：
  1. 运行 `cargo clippy --all-targets`，确保 0 warning。
  2. 运行 `cargo fmt --check`，确保格式一致。
  3. 运行项目全量单元测试确认基线稳健。
