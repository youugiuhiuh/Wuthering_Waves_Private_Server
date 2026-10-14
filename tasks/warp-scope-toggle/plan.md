# Plan: WARP 模式开关 + 缺陷修复 + 双路出口探测器

> 本文件属于 **`feat/warp-scope-toggle`** 分支。同目录外的 `tasks/plan.md` / `tasks/todo.md`
> 属于 `custom-allowlist` 特性，**不要混淆、不要覆盖**。

**Spec**: 已批准（见会话记录）
**Branch**: `feat/warp-scope-toggle`（base `95e9930`）
**Worktree**: `/home/ub/Dark/wwps-worktrees/warp-scope-toggle`
**任务清单**: `tasks/warp-scope-toggle/todo.md`

## 基线（已实测）

| 项 | 结果 |
|---|---|
| `cargo check --all-targets` | ✅ OK |
| `cargo nextest run --cargo-profile fast-test` | ✅ **1119 passed, 1 skipped**（权威） |
| `cargo test --lib` | ⚠️ 1 个既有失败（`sni/selector.rs` t4）——**nextest-only 设计**，代码注释 `selector.rs:247,253,410` 已说明，非本任务引入 |

**回归红线**：改完后 nextest 必须仍 ≥ 1119 通过。

## 已确认决策

| 决策 | 选择 |
|---|---|
| 本轮范围 | 开关 + Bug A + Bug B + 双路探测器 |
| 状态持久化 | `warp_account.json`，`#[serde(default)]` |
| 开关是否生效 | **不生效**（v1 只落状态） |
| usque 集成 | 独立 Spec 2，本轮不做 |
| Q1 路径常量 | 走 `core/paths.rs` |
| Q2 scope-only 变更 | **跳过 reload** |
| Q3 探测器并发 | **并行，总超时 3s** |
| Q4 探测结果缓存 | **不缓存** |
| Q5 dead_code | **配置项控制**（代码路径真实存在） |

## 依赖图

```
paths.rs (常量: WG_RX_BYTES_PATH / UDP_PROBE_ENDPOINT / TCP_PROBE_ENDPOINT)
    │
    ├── warp_api.rs (WarpScope 反序列化字段)
    │        │
    │        └── warp.rs (WarpScope enum + build_routing_rules + 签名变更)
    │                 │
    │                 ├── installer.rs (Bug A: install 保留配置
    │                 │                 Bug B: status 读 rx_bytes)
    │                 │
    │                 └── handlers/warp.rs (a_warp_switch_scope + a_warp_probe)
    │                          │
    │                          └── i18n {zh,en,ja}.yml
    │
    └── warp_probe.rs (新模块: UDP + TCP 双路探测)
             │
             └── handlers/warp.rs (a_warp_probe 接线)
```

## 关键既有代码位置（已核实）

| 位置 | 内容 |
|---|---|
| `core/paths.rs:94-97` | `pub mod warp { ACCOUNT_FILE, ROUTING_FILE }` |
| `core/xray/warp.rs:9-33` | `WarpMode` enum + `as_str` + `next` |
| `core/xray/warp.rs:36-139` | `update_warp_routing_rules(rules, mode)` |
| `core/xray/warp.rs:113-125` | routing rules 构建（catch-all 插入点） |
| `core/xray/warp.rs:141-183` | `get_warp_routing_rules() -> (Vec<String>, WarpMode)` |
| `core/network/warp_api.rs:13-21` | `WarpAccountConfig` struct |
| `core/xray/installer.rs:19-27` | `is_installed()`（Bug B 假阳性源头） |
| `core/xray/installer.rs:29-48` | `install()`（Bug A 源头） |
| `core/xray/installer.rs:58-62` | `restart_service()` → `reload_core()` |
| `core/xray/installer.rs:64-76` | `status()` |
| `shared/handlers/warp.rs:12-125` | `m_warp` 菜单 |
| `shared/handlers/warp.rs:128-146` | `a_warp_switch_mode`（新按钮样板） |
| `shared/handlers/warp.rs:439-472` | `a_warp_status`（输出样板） |
| `shared/handlers/message.rs:459` | `add_warp_routing_rules` 调用点 |
| `tests/test_self_destruct_e2e.rs:85-86` | 依赖 `10_warp_routing.json` 结构 |

## `get_warp_routing_rules` 签名变更需适配的调用点（6 处）

- `shared/handlers/warp.rs:42`（`m_warp`）
- `shared/handlers/warp.rs:130`（`a_warp_switch_mode`）
- `shared/handlers/warp.rs:214`（`a_warp_del_menu`）
- `shared/handlers/warp.rs:279`（`a_warp_del:`）
- `shared/handlers/warp.rs:341`（`a_warp_del_confirm:`）
- `core/xray/installer.rs:41`（`install()`）

## 探测基线（服务器实测，用作预期断言值）

| 探测 | 实测结果 |
|---|---|
| UDP `162.159.192.1:2408` | ❌ 静默（3 次重试全超时） |
| UDP `162.159.192.1:500/1701/4500` | ❌ 静默 |
| UDP `8.8.8.8:53` | ✅ 通（证明出站 UDP 本身可用） |
| TCP `162.159.198.2:443` + TLS | ✅ 可连，ALPN=`h2` |
| TCP `162.159.197.1:443` | ✅ 可连，ALPN=`h2` |
| TCP `162.159.192.1:443` | ✅ 可连，ALPN=`h2` |

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| 改 `install()` 影响重装路径 | 只在 worktree 跑测试；**绝不在生产服务器验证** |
| `clippy -D warnings` 因 dead_code 失败 | Q5 用配置项控制，不抑制 lint |
| 回归红线 `test_self_destruct_e2e` | 每个相关任务后单独跑该测试 |
| serde 兼容性 | TDD 覆盖「旧文件无 scope 字段」用例 |
| 探测对外发包 | 只发 1 个握手包 + 1 次 TLS 握手，不建连接、不改配置 |
| 与 `custom-allowlist` 任务文件冲突 | 本计划写在独立子目录 |

## 交付物 → 任务映射

| 交付物 | 任务 |
|---|---|
| 1. 流量范围开关 UI | T2–T5, T11, T13 |
| 2. Bug A：install 保留配置 | T9 |
| 3. Bug B：status 真实探测 | T10 |
| 4. 双路出口探测器 | T1, T6, T7, T8, T12 |
| 质量门禁 | T14, T15 |
