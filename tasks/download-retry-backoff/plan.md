# 实施计划：下载瞬态故障指数退避重试（Exponential Backoff Retry）

## 概述
针对在服务器日志中发现的 GitHub Release CDN / 规则文件下载瞬态网络故障（HTTP 504 Gateway Timeout、502/503、连接超时及连接重置），为 `rust/aegis` 的网络下载链路引入指数退避重试（Exponential Backoff Retry）机制，使瞬时网络波动时能自动重试恢复，杜绝更新/下载直接失败。

## 架构与设计决策
1. **重试判定准则**：
   - HTTP 状态码为可恢复服务端/网关错误：500, 502, 503, 504, 408 (Request Timeout), 429 (Too Many Requests)。
   - 网络底层错误：`e.is_timeout()`, `e.is_connect()`, 连接重置 (ConnectionReset) 等瞬态 IO 错误。
   - 客户端永久错误（400, 401, 403, 404 等）不进行无谓重试。
2. **退避策略**：
   - 最大重试次数：3 ~ 4 次。
   - 指数退避间隔：初次 2s，后续 4s，8s。
   - 重试前输出 `log::warn!` 记录重试原因与尝试次数。
3. **改造点**：
   - `core/network/release_api.rs`：封装通用可重试判定及退避辅助函数，补充完备单测。
   - `core/system/maintenance.rs`：在 `MaintenanceManager::download_file` 中应用重试逻辑，确保规则文件下载不因瞬时 504 中断。
   - `core/system/core_upgrade.rs`：在 `download_release` 与 `download_sha256_manifest` 中应用重试逻辑，保证 Xray-core 二进制与校验文件下载健壮。

## 任务拆解与垂直切片
- Task 1: 编写重试判定与退避重试核心逻辑及单元测试（`release_api.rs`）
- Task 2: 在 `maintenance.rs` 的 `download_file` 中接入重试逻辑
- Task 3: 在 `core_upgrade.rs` 的 `download_release` 与 `download_sha256_manifest` 中接入重试逻辑
- Checkpoint: 全量执行 `cargo fmt`、`clippy`、`nextest`

## 风险与应对
- 风险：下载中间断开若未清理临时文件可能导致脏数据写入。
- 应对：重试每次开启新流前清空/覆盖临时文件，并在最终失败时清理 `.tmp` 临时文件。
