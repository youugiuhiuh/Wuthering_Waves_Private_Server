# Plan: 组合形态下 secondary 平台不承担交互

> Spec: `docs/superpowers/specs/2026-09-24-secondary-platforms-no-interaction-design.md`（已批准）
> 模式：strict ｜ 全部改动集中在 `rust/aegis/src/main/runtime.rs`

## 依赖图

```
T1 规则纯函数 + 单测
      │
      ├── T2 SimpleX 分支接入（classify_simplex_inbound）
      └── T3 Matrix 分支接入（secondary_handles_inbound 门控 handler）
                    │
                    └── T4 质量门 + 真机验收
```

T2 与 T3 互相独立，可并行；都依赖 T1。

---

## Task 1: 规则纯函数 + 单测

**Description:** 在 `runtime.rs` 新增统一规则 `secondary_handles_inbound` 与 SimpleX 入站分类 `classify_simplex_inbound`（含 `SimplexInbound`）。先写失败测试，再实现。**本任务不改任何调用点**。

**Acceptance criteria:**
- [ ] `secondary_handles_inbound(true) == false`、`secondary_handles_inbound(false) == true`
- [ ] `classify_simplex_inbound(true, true, Some("/menu")) == Drop`
- [ ] `classify_simplex_inbound(true, false, Some("123456")) == Drop`（tg 形态下 SimpleX 的码也不放行）
- [ ] `classify_simplex_inbound(false, true, …) == Dispatch`（纯 SimpleX 不变）
- [ ] `classify_simplex_inbound(false, false, Some("123456")) == Dispatch`
- [ ] `classify_simplex_inbound(false, false, Some("/menu"|"hello"|None|"12345"|"1234567"|"12345a")) == Drop`

**Verification:**
- [ ] `cargo test --bin aegis classify_simplex_inbound secondary_handles_inbound`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`

**Dependencies:** None
**Files likely touched:** `rust/aegis/src/main/runtime.rs`

---

## Task 2: SimpleX 分支接入

**Description:** 在 SimpleX 事件循环的 `NewChatItems` 分支用 `classify_simplex_inbound` 取代 `should_forward_simplex_msg`：`Drop` 时记 `log::warn!` 并 `continue`。删除旧函数，把其原有测试迁移为 `classify_simplex_inbound(false, …)` 情形，保证纯 SimpleX 不回归。`ContactConnected` 与 `ReceivedContactRequest` 处理保持不变。

**Acceptance criteria:**
- [ ] `should_forward_simplex_msg` 不再存在，其测试全部迁移且有对应新测试
- [ ] `--tg-simplex`（`enable_telegram=true`）下 `NewChatItems` 一律不进入 `dispatch_event`
- [ ] 纯 `--simplex`（`enable_telegram=false`）行为与改动前逐条一致
- [ ] 丢弃路径有可观测日志

**Verification:**
- [ ] `cargo test --bin aegis simplex`
- [ ] `cargo nextest run --cargo-profile fast-test`

**Dependencies:** T1
**Files likely touched:** `rust/aegis/src/main/runtime.rs`

---

## Task 3: Matrix 分支接入

**Description:** 在 `runtime.rs` 的 Matrix 分支，把「注册入站消息 handler + 其中的命令派发」整块包进 `if secondary_handles_inbound(enable_telegram)`，并把该块用到的局部绑定移入块内（避免 unused 触发 `-D warnings`）。加密事件 handler 与 `client.sync` 不动；纯 `--matrix`（`enable_telegram=false`）仍注册。

**Acceptance criteria:**
- [ ] `--all`（`enable_telegram=true`）下矩阵房间消息不进入 `dispatch_event`，无任何回复
- [ ] 纯 `--matrix`（`enable_telegram=false`）行为与改动前一致
- [ ] `client.sync` 与加密日志 handler 仍在
- [ ] 无 unused 变量/未使用 import

**Verification:**
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`（关掉 handler 后不能有 unused）
- [ ] `cargo nextest run --cargo-profile fast-test`

**Dependencies:** T1
**Files likely touched:** `rust/aegis/src/main/runtime.rs`

---

## Task 4: 质量门 + 真机验收

**Description:** 跑完整质量门，按 spec §6 逐条真机验收，记录证据；完成后交 REVIEW（code-review-and-quality）。

**Acceptance criteria:**
- [ ] 质量门全绿：

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run --cargo-profile fast-test && \
cargo test --doc
```

- [ ] spec §6 六条真机验收全部有日志/截图证据（或明确标注无法在本环境验证的项与原因）
- [ ] 五轴 review 无 Critical

**Verification:**
- [ ] 上述命令输出
- [ ] 真机（`--tg-simplex` / `--all` / 纯 `--simplex` / 纯 `--matrix`）行为记录

**Dependencies:** T2, T3
**Files likely touched:** 无（验证）；如验收发现问题则回到对应任务

---

## 边界

- 不新增 CLI flag、不改 `RoutingAdapter`、不改能力表、不改 i18n。
- 不碰 `matrix_handle` 的连接与 `client.sync`。
- 不重构 `runtime.rs` 的其它部分。
