# Specification: Xray Geodata 迁移至 XTLS/Xray-rules-dat 官方维护源

> 状态：**已完成并验证通过**  
> 责任模块：`aegis` (Rust)  
> 涉及范围：路由配置、核心升级预检、维护数据更新（≤ 3 文件）  
> 对应模式：**normal 模式**

---

## 1. 背景与目标 (Background & Objective)

### 1.1 问题现状
Xray-core 在 Commit [`de1bbbff88ddee69fc4984775a425f77fd2799e7`](https://github.com/XTLS/Xray-core/commit/de1bbbff88ddee69fc4984775a425f77fd2799e7)（PR #7185，随 `v26.10.21` 发布）中，官方将构建资源从第三方 `Loyalsoldier/v2ray-rules-dat` 切换为官方维护的 **`XTLS/Xray-rules-dat`**。

- 官方维护的 `geosite.dat` 严格同步自上游标准 `v2fly/domain-list-community`，不包含 Loyalsoldier 自行魔改衍生的顶级 Code（如 `apple-cn`）。
- 现网节点的 `00_base.json` 配置中引用的 `essential_direct` 规则包含 `"geosite:apple-cn"`。
- 当执行核心自更新时，解压目录自带新版 `geosite.dat`，新版 Xray 执行配置预检时因找不到 `APPLE-CN` 报错中止升级：
  `failed to check code APPLE-CN from geosite.dat > EOF`
- 同时，后台定时任务 `update_geodata` 与初始化 `ensure_geodata` 目前仍硬编码指向 Loyalsoldier，导致与 Xray 自带数据源存在分裂。

### 1.2 目标与成功指标
1. **路由规范化**：将路由定义中的 `"geosite:apple-cn"` 迁移为上游标准语法 **`"geosite:apple@cn"`**，经实测该语法在旧版（v26.9.30 及以前）和新版（v26.10.21 及以后）均能 100% 正确解析。
2. **下载源官方化**：将 `MaintenanceManager` 中的 geodata 下载源迁移至 `XTLS/Xray-rules-dat`（`geosite.dat` 与 `geoip-lite.dat`），消除数据源分裂。
3. **升级零干预自愈**：在 `core_upgrade` 预检前自动规范化基准配置，使存量节点在下发自更新时无需管理员手动修改 JSON，自动完成配置迁移并通过预检。

---

## 2. 详细技术方案 (Technical Design)

### 2.1 路由规则定义 (`rust/aegis/src/core/xray/routing.rs`)
- 更新 `ROUTING_RULES` 中的 `essential_direct`：
  ```rust
  // ── Apple（2）──
  // 覆盖 ocsp/crl/swscan/swdist/swcdn/gs-loc/cl2-cl5/init.ess/guzzoni/...
  "geosite:apple@cn",
  // apple 直连唯一漏项
  "domain:init.itunes.apple.com",
  ```
- 同步更新单测白名单与断言：
  - `test_essential_direct_targets_use_explicit_prefix`：白名单中的 `"geosite:apple-cn"` 改为 `"geosite:apple@cn"`。
  - `test_essential_direct_contains_evidence_backed_hosts`：断言列表中的 `"geosite:apple-cn"` 改为 `"geosite:apple@cn"`。

### 2.2 数据源更新 (`rust/aegis/src/core/system/maintenance.rs`)
- `ensure_geodata_in_dir` 与 `update_geodata` 中的数据源数组调整为：
  ```rust
  let sources = [
      (
          "geoip.dat",
          "https://github.com/XTLS/Xray-rules-dat/releases/latest/download/geoip-lite.dat",
      ),
      (
          "geosite.dat",
          "https://github.com/XTLS/Xray-rules-dat/releases/latest/download/geosite.dat",
      ),
  ];
  ```
  *(注：Xray 官方 Release 打包内置的正是 `geoip-lite.dat`，重命名为 `geoip.dat`)*。

### 2.3 升级前配置自动迁移 (`rust/aegis/src/core/system/core_upgrade.rs`)
- 在 `preflight_check_config(&new_binary)` 之前，增加预检前基准配置规范化步骤：
  ```rust
  // 预检前执行规范化迁移：将存量 00_base.json 中的旧规则（如 apple-cn）按最新 ROUTING_RULES 规范化
  if let Err(err) = crate::core::xray::RoutingManager::ensure_direct_rules_in_base().await {
      log::warn!("升级预检前规范化 Xray direct 规则失败（仍尝试预检）: {err}");
  }
  ```
- 这样，存量节点在执行 `run_upgrade` 时，在解压后、预检前会自动将 `geosite:apple-cn` 刷新为 `geosite:apple@cn` 并保存，随后 `xray run -test` 顺畅通过，达成全自动无人值守升级。

---

## 3. 非功能约束与安全边界

1. **零风险防御保留**：核心升级的两道防线（替换前预检 + 替换后崩溃采样与自动回滚）完全保留且不受破坏。
2. **文件变更隔离**：改动严格限制在 3 个现有文件，不增加任何额外外部依赖，不改动其它协议逻辑。
3. **Rust 质量门**：必须通过 `cargo fmt --check`、`cargo clippy` 以及相关单测全绿。
