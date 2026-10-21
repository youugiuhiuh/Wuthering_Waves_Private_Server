# Tasks: Xray Geodata 迁移至 XTLS/Xray-rules-dat 官方源

- [x] **Task 1**: 路由规则升级为 `geosite:apple@cn` (TDD)
  - [x] 1.1 更新 `routing.rs` 相关测试断言为 `geosite:apple@cn` (RED)
  - [x] 1.2 更新 `ROUTING_RULES` 中的 `geosite:apple-cn` 为 `geosite:apple@cn` (GREEN)
  - [x] 1.3 运行 `cargo test -p aegis core::xray::routing` 确认全绿
- [x] **Task 2**: 切换 geodata 下载源为 XTLS/Xray-rules-dat
  - [x] 2.1 修改 `maintenance.rs` 中 `ensure_geodata_in_dir` 数据源 URL
  - [x] 2.2 修改 `maintenance.rs` 中 `update_geodata` 数据源 URL
  - [x] 2.3 验证单测正常
- [x] **Task 3**: 核心升级预检前自动规范化基准配置
  - [x] 3.1 在 `core_upgrade.rs` 的 `run_upgrade` 预检前加入 `ensure_direct_rules_in_base()` 规范化步骤
  - [x] 3.2 运行 `cargo test -p aegis core::system::core_upgrade` 验证
- [x] **Task 4**: 质量门检查与收尾
  - [x] 4.1 运行 `cargo clippy --all-targets`
  - [x] 4.2 运行 `cargo fmt --check`
  - [x] 4.3 整体回归验证
