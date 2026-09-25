# 设计：组合形态下 secondary 平台不承担交互

> 模式：**strict** ｜ 状态：**待批准** ｜ 日期：2026-09-24
> 用户原话：「TG+simplex 交互交给 TG……当是 simplex+TG 不应该 simplex 交互！」
> 追加范围：「也改 Matrix+TG 的」——`--all` 的 Matrix 入站交互同样收敛。

## 1. 背景（已核实，CodeGraph + 源码）

| 事实 | 位置/证据 |
| --- | --- |
| `--tg-simplex` = TG 主 + SimpleX 次（敏感内容落点） | `rust/aegis/src/main.rs:213-225`、`:89-107` |
| SimpleX 事件循环**无条件**派发 `NewChatItems`，与 `enable_telegram` 无关（注释原文「事件循环不受影响，仍照常接收命令」） | `rust/aegis/src/main/runtime.rs:250`、`:364-400` |
| 派发用的是 `SimplexAdapter` 本身，回复也走 SimpleX | `rust/aegis/src/main/runtime.rs:250`（`adapter_for_events = handle.adapter`） |
| 管理员消息全放行；非管理员仅 6 位纯数字码放行（TOTP 自愈入口） | `rust/aegis/src/main/runtime.rs:653-660` |
| TOTP 成功且**来源平台为 SimpleX** 时重钉 `simplex_admin_id` 并改敏感落点 | `rust/aegis/src/app/auth.rs:34-57` |
| `with_simplex_repin` 对**两种** SimpleX 形态都开启（含 `--tg-simplex`） | `rust/aegis/src/main.rs:136-143` |
| 联系人审批通知已经发到 TG（含按钮） | `rust/aegis/src/main/runtime.rs:316-346` |
| `--all` = TG 主 + Matrix 次（敏感内容落点） | `rust/aegis/src/main.rs:213-225`、`rust/aegis/src/main/adapter.rs:15-30` |
| Matrix 同步循环**无条件**注册入站 handler，管理员命令在 Matrix 房间内被处理并回复 | `rust/aegis/src/main/runtime.rs:120-224` |

**后果（当前真实行为）**：
1. `--tg-simplex`：SimpleX 管理员发 `/menu`、文本 → 由 SimpleX 适配器处理并回复 = SimpleX 是完整交互面；任意 SimpleX 联系人发 6 位码 → 可重钉 `simplex_admin_id`、并把敏感内容落点改到该联系人 = SimpleX 能改安全态。
2. `--all`：Matrix 房间内管理员消息 → 由 Matrix 适配器处理并回复 = Matrix 是完整交互面。

## 2. 目标

**统一规则：primary 平台交互，secondary 平台只作出站落点。**

- **G1**：`--tg-simplex` 下 SimpleX 入站**不派发任何交互**（命令 / 文本 / TOTP），SimpleX 只作敏感内容**出站**落点。
- **G2**：`--tg-simplex` 下 `simplex_admin_id` 只能来自配置，不得被 SimpleX 入站改写。
- **G3**：`--all` 下 Matrix 入站**不派发任何交互**，Matrix 只作敏感内容**出站**落点。
- **G4**：纯 `--simplex` / 纯 `--matrix` 行为**完全不变**（命令、TOTP 自愈、调度器、启动通知）。

## 3. 设计（最小 diff）

一条规则、一个纯函数、两个调用点（SimpleX 与 Matrix 共用），作为唯一判据与 TDD 锚点：

```rust
/// 组合形态下 secondary 平台是否处理入站交互。
/// primary 已启用（TG）时 secondary（SimpleX / Matrix）只作出站落点。
pub(crate) fn secondary_handles_inbound(primary_enabled: bool) -> bool {
    !primary_enabled
}
```

调用点 A：`rust/aegis/src/main/runtime.rs` 的 Matrix 分支——把「注册消息 handler + 派发」整块包进 `if secondary_handles_inbound(enable_telegram)`（其局部绑定一并移入，避免 `-D warnings` 下的 unused）。加密事件 handler 与 `client.sync` 保持不动。

调用点 B：SimpleX 分支——把入站处置收敛成纯函数：

```rust
pub(crate) enum SimplexInbound {
    /// 交给 dispatch_event（命令/文本/登录码）。
    Dispatch,
    /// 丢弃（仅记日志）。
    Drop,
}

/// TG 启用时 SimpleX 一律不交互；纯 SimpleX 时沿用既有门禁
/// （管理员全放行，非管理员只放行 6 位纯数字码 = TOTP 自愈入口）。
pub(crate) fn classify_simplex_inbound(
    enable_telegram: bool,
    is_admin: bool,
    text: Option<&str>,
) -> SimplexInbound {
    if !secondary_handles_inbound(enable_telegram) {
        return SimplexInbound::Drop;
    }
    // …既有 should_forward_simplex_msg 判据逐字保留
}
```

调用点 `rust/aegis/src/main/runtime.rs` SimpleX 事件循环内，`NewChatItems` 分支：

```rust
// 在 for msg in mapped 之前
match classify_simplex_inbound(enable_telegram, is_admin, msg.text.as_deref()) { ... }
```

- `ContactConnected`（日志）与 `ReceivedContactRequest`（转发 TG 审批按钮）**保持不变**：它们是平台事件，不是交互面。
- `should_forward_simplex_msg` 被 `classify_simplex_inbound` 取代（纯 SimpleX 判据逐字保留，含 6 位码规则）。
- 丢弃时记 `log::warn!`，保留既有可观测性。

**不做**：不新增 CLI flag、不改 `RoutingAdapter`、不改能力表、不改 i18n、不改 `matrix_handle` 的连接与 `client.sync`。

## 4. 假设（请纠正）

1. 「不交互」= 连 SimpleX 入站的 TOTP 码也丢弃（`--tg-simplex` 的登录码应在 TG 发；安全码仍会由 TG 侧 TOTP 成功后发到 TG）。
2. `ReceivedContactRequest` 仍转发 TG 审批（否则 tg-simplex 无法审批新联系人）。
3. 丢弃入站时**不给发送者任何回复**（静默 + 日志），避免 secondary 出现任何交互回声。
4. 本地 CLI 旁路（`simplex.rs` 的 contactId 写入）不算「交互」，保持不变。
5. `--all` 下 Matrix 仍连接、仍 `sync`（发送所需），只是不再注册入站消息 handler。

## 5. 测试策略（TDD）

- 主战场：纯函数单测（`rust/aegis/src/main/runtime.rs` 既有 `#[cfg(test)] mod tests`）。
  - `secondary_handles_inbound(true) == false` / `(false) == true`（RED 先失败：函数尚不存在）。
  - `classify_simplex_inbound(true, …)` 管理员与非管理员 6 位码均 `Drop`。
  - `classify_simplex_inbound(false, …)` 的四种情形 = 既有 `should_forward_simplex_msg` 测试逐条迁移，保证纯 SimpleX 不回归。
- Matrix 侧无纯逻辑可单测（是 handler 注册开关），由真机验收覆盖。
- 质量门（项目根，LLVM 为准）：

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

## 6. 验收标准（真机）

1. `--tg-simplex` 启动后，从 SimpleX 联系人发 `/menu` 与任意文本 → 无任何回复，日志出现「已丢弃」。
2. `--tg-simplex` 下从 SimpleX 发 6 位码 → 不重钉、敏感落点不变（配置里的 `simplex_admin_id` 保持）。
3. `--tg-simplex` 下 TG 侧命令、TOTP、安全码输出、联系人审批全部照常。
4. `--all` 下在 Matrix 房间发 `/menu` 与任意文本 → 无任何回复。
5. `--all` 下 TG 侧命令、启动通知全部照常；敏感内容仍只落 Matrix。
6. 纯 `--simplex` / 纯 `--matrix`：命令、TOTP 自愈、调度器与启动通知全部照常。

## 7. 风险与边界

- **风险**：把 SimpleX 入站 TOTP 也关掉后，若用户仍习惯在 SimpleX 发码登录，会「没反应」；缓解 = 丢弃日志明确 + TG 侧流程不变。
- **边界**：`--all` 下 Matrix 入站被关闭后，若仍有人在 Matrix 房间发命令会「没反应」；缓解 = 丢弃日志 + TG 侧流程不变。这是用户明确要求的收敛。
