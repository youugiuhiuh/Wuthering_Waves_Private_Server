# Plan: usque-tcp（WARP over TCP/443）

**Spec**: `tasks/usque-tcp/SPEC.md`（已批准）
**Branch**: `feat/usque-tcp`（base `8c94d05`）
**Worktree**: `/home/ub/Dark/wwps-worktrees/usque-tcp`
**任务清单**: `tasks/usque-tcp/todo.md`

## 基线（已实测）

| 项 | 结果 |
|---|---|
| `cargo check --all-targets` | ✅ OK |
| `cargo nextest run --cargo-profile fast-test` | ✅ **1157 passed, 1 skipped**（权威，已含 Spec 1） |

**回归红线**：改完后 nextest 必须 ≥ 1157 通过。

## 依赖图

```
paths.rs (usque 路径常量 + DESTRUCT_SERVICES 增项)
    │
    ├── usque/pin.rs        (版本 + sha256 常量 + URL 构造)      ← 纯，无 I/O
    │        │
    │        └── usque/installer.rs (下载/校验/register/安装/卸载)
    │                 │
    │                 └── usque/unit.rs (systemd unit 模板生成)  ← 纯
    │
    ├── usque/engine.rs     (ProbeConclusion → 出口类型 + 内存缓存)
    │        │
    │        └── xray/warp.rs (socks outbound 生成 + enforce 改运行时判定)
    │
    └── usque/health.rs     (三层健康检查 + 强杀重启)
             │
             └── handlers/warp.rs (a_warp_status 体现 usque 健康)

  xray/installer.rs (uninstall 成对清理) ← 依赖 paths + usque/installer
```

## 关键既有代码位置（已核实）

| 位置 | 内容 |
|---|---|
| `core/network/warp_probe.rs:90-99` | `ProbeConclusion`（4 态，我要消费的判据） |
| `core/network/warp_probe.rs:306` | `run_probe()` |
| `core/paths.rs:110-128` | 探测常量（Spec 1） |
| `core/xray/warp.rs:30-35` | `enforce_global_warp()`（OQ5 要替换成运行时判定） |
| `core/xray/warp.rs:145` | `build_routing_rules(rules, scope, enforce)` |
| `core/xray/warp.rs:243-282` | wireguard outbound 生成（要加 socks 分支） |
| `core/xray/installer.rs` `uninstall()` | 只清 WireGuard 两文件（要成对清理） |
| `core/singbox/installer.rs:224-265` | `create_service()` —— unit 生成+启用模板 |
| `core/system/maintenance.rs:122` | `reload_core()` |
| `core/system/core_health.rs` | 「is-active 不够」的既有教训 |
| `core/security/self_destruct.rs` | 使用 `DESTRUCT_SERVICES` |

## 硬约束（来自 DECISIONS.md，实现时不可违反）

1. **必须 `socks --http2`，禁止 `l4-socks`**（无 `--http2`，QUIC-only）→ 需防回归测试
2. **健康检查须对付「卡死但不退出」**（`Restart=always` 无效）→ 三层判定
3. **`register` 须写 stdin `y`**（否则 EOF 失败）
4. **usque 配置格式与 WireGuard 不兼容**，不共用文件
5. **pin 版本 + pin sha256**，不用 latest
6. **Xray socks outbound 是扁平形状**（`settings.address/port`，非 `servers[]`）→ 需断言测试

## 已确认决策（摘要）

| 项 | 选择 |
|---|---|
| 与 WireGuard 关系 | 并存，按探测结果自动选 |
| SOCKS 模式 | `socks --http2` |
| 供应链 | pin 版本 + pin sha256 |
| 断连兜底 | 健康检查 + 强杀重启（**无**自动降级） |
| 流量接入 | 复用 WarpScope（Domain/Global） |
| TG 菜单 | 不做新菜单 |
| 部署 | 独立 systemd unit `wwps-usque.service` |
| OQ1 判据来源 | 探测缓存 TTL 10min |
| OQ2 缓存位置 | 内存 |
| OQ3 FullyBlocked | direct + `a_warp_status` 体现 |
| OQ4 self_destruct | 加入 `DESTRUCT_SERVICES` |
| OQ5 环境变量 | 移除，改运行时判定 |
| **卸载** | **成对检查并卸载两套，幂等** |

## 实测常量（供测试断言）

```
版本     v4.2.1
资产     usque_4.2.1_linux_amd64.zip
sha256   4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313
大小     12,906,658 B（解压后二进制）
监听     127.0.0.1:1080（-p 可改）
配置     config.json（-c 可改）
健康判据 走隧道请求 cdn-cgi/trace 含 warp=on
```

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 改 `uninstall()` 影响既有卸载路径 | 幂等设计 + 7 态矩阵穷举测试 |
| OQ5 移除环境变量改变既有行为 | `enforce` 语义文档化；Spec 1 的测试同步更新 |
| 网络 I/O 难以单测 | 纯函数拆分（pin/unit/engine/detect）；I/O 薄封装 |
| 有人在 CI 里真发网络包 | 测试全部用 fixture 字符串，不打网络 |
| 与并行的 `routing-split` 分支冲突 | 只碰 `warp.rs` 的 outbound 段，不碰 `routing.rs` |

## 交付物 → 任务映射

| 交付物 | 任务 |
|---|---|
| 路径常量与 pin | T1, T2 |
| systemd unit 生成 | T3 |
| 安装（下载/校验/register） | T4, T5 |
| 出口引擎选择 + 缓存 | T6, T7 |
| Xray socks outbound | T8 |
| 健康检查 + 强杀 | T9, T10 |
| **成对卸载** | T11, T12 |
| self_destruct + 状态展示 | T13, T14 |
| 质量门禁与审查 | T15, T16 |
