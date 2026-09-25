# Plan: 用户回贴 SimpleX 安全码，Bot 自动验证（A 方案）

> Spec: `docs/superpowers/specs/2026-09-25-simplex-security-code-confirmation-design.md`（已批准）
> 模式：strict ｜ 分支：`feat/simplex-code-confirm` ｜ Worktree：`.worktrees/simplex-code-confirm`
> 前置计划已归档：`tasks/archive/2026-09-24-secondary-no-interaction.plan.md`

## Architecture Decisions

- 持久化复用 `BotSettings`（明文 JSON，contactId 非机密），不碰 config.enc 加密层。
- 「已验证」判定 = 存储的 contactId == 当前 `simplex_admin_id`；重钉即自然失效，无需显式清零。
- 比对前归一化（去空白只比数字），不假设码的固定分组/长度。
- fail-open：取码/落盘失败只告警，不阻塞 TOTP 与普通消息处理。
- 转换回复强制走 `send_message_primary`（避免敏感分流改投 SimpleX）。

## 依赖图

```
S1 持久化(BotSettings+读改写) ──→ S2 AppState + main 接线 ──┬─→ S3 出站门控(auth)
                                                             │
S4 dispatch 纯函数(normalize/looks_like) ───────────────────┴─→ S5 入站确认(dispatch)
                                                                        │
                                                          S3 + S5 ──────┴─→ S6 i18n 三语
                                                                                  │
                                                                                  └─→ S7 质量门 + 真机 + REVIEW
```

S1 与 S4 独立可并行；S2 仅依赖 S1；S3 仅依赖 S2；S5 依赖 S2+S4；S6 依赖 S3+S5；S7 依赖全部。

---

## Task S1: BotSettings 持久化字段 + 读改写修复

**Description:** `BotSettings` 增 `simplex_code_verified_for: Option<i64>`（`#[serde(default)]`），新增 `set_simplex_code_verified(config_dir, Option<i64>)`（load→set→save）。把 `state_ops.rs` 的超时写入从「整体重写」改为「读-改-写」，避免抹掉新字段。先写失败测试再实现。

**Acceptance criteria:**
- [ ] 旧 `bot_settings.json`（无新字段）能反序列化，`simplex_code_verified_for == None`
- [ ] `set_simplex_code_verified(dir, Some(5))` 后 `BotSettings::load()` 读到 `Some(5)`；`Some(5)→None` 可清空
- [ ] 超时 setter 保存后，先前写入的 `simplex_code_verified_for` 仍在

**Verification:**
- [ ] `cargo test --lib bootstrap::` 与 `cargo test --lib shared::state_ops`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`

**Dependencies:** None
**Files likely touched:** `rust/aegis/src/bootstrap.rs`, `rust/aegis/src/shared/state_ops.rs`

---

## Task S2: AppState 字段/方法 + main 接线

**Description:** `AppState` 增 `simplex_code_verified_for: AtomicI64`（0=未验证），`new()` 默认 0；新增 `with_simplex_code_verified_for`、`simplex_code_verified`、`set_simplex_code_verified_for`。`main.rs` 在 `.with_simplex_repin()` 旁接上 `app_config.bot_settings.simplex_code_verified_for`。

**Acceptance criteria:**
- [ ] `new()` 后 `simplex_code_verified()` == false
- [ ] `with_simplex_code_verified_for(Some(5))` 且 `simplex_admin_id()==Some(5)` → true；`simplex_admin_id()==Some(7)` → false
- [ ] `set_simplex_code_verified_for(Some(7))` 后随当前 admin id 变化即时反映
- [ ] `main.rs` 编译通过且启动路径把配置值带入

**Verification:**
- [ ] `cargo test --lib app::state`
- [ ] `cargo build --bin aegis`（或 `cargo check --all-targets`）

**Dependencies:** S1
**Files likely touched:** `rust/aegis/src/app/state.rs`, `rust/aegis/src/main.rs`

---

## Task S3: 出站门控（已验证不再发码）

**Description:** `app/auth.rs` 的安全码发送块追加 `&& !state.simplex_code_verified()`。已验证时完全跳过 `contact_security_code` 与发送。

**Acceptance criteria:**
- [ ] 未验证：行为与现状一致（取码 + `send_message_primary` 各 1 次）
- [ ] 已验证（`verified_for == simplex_admin_id`）：`contact_security_code` 0 次、`send_message_primary` 0 次
- [ ] TOTP 成功结果本身不受影响

**Verification:**
- [ ] `cargo test --lib app::auth`
- [ ] `cargo nextest run --cargo-profile fast-test -E 'test(auth)'`

**Dependencies:** S2
**Files likely touched:** `rust/aegis/src/app/auth.rs`

---

## Task S4: dispatch 纯函数 normalize / looks_like

**Description:** 在 `shared/dispatch.rs` 新增 `normalize_security_code(&str) -> String` 与 `looks_like_security_code(&str) -> bool`（纯数字+空白、去空白后 >= 12 位），含 `MIN_CODE_DIGITS` 常量。先写失败测试。

**Acceptance criteria:**
- [ ] `normalize_security_code("54440 24092\n64994") == "544402409264994"`；含非数字字符时只保留数字（函数本身不校验合法性）
- [ ] `looks_like_security_code` 对跨行/纯数字（>=12 位）为真
- [ ] 对含字母、命令（`/menu`）、6 位纯数字 TOTP、空串、<12 位为假

**Verification:**
- [ ] `cargo test --lib shared::dispatch`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`

**Dependencies:** None
**Files likely touched:** `rust/aegis/src/shared/dispatch.rs`

---

## Task S5: 入站确认 try_confirm_security_code

**Description:** `dispatch_event` 的 `BotEvent::Message` 分支在 `handle_message` 前调用 `try_confirm_security_code`。命中判据：未验证 + 有 TG 管理员 + `looks_like_security_code`。现取码比对：一致 → 落盘 + 内存置位 + 回复 `simplex.code_verified` + 短路；不一致 → 回复 `simplex.code_mismatch` + 告警 + 不落盘 + 短路。取码失败 fail-open 返回 false。

**Acceptance criteria:**
- [ ] 一致：`simplex_code_verified()` true、`BotSettings` 落盘 `Some(sx_admin)`、回复含 `code_verified`、后续 `handle_message` 未调用（消息被吞）
- [ ] 不一致：不落盘、`simplex_code_verified()` false、回复含 `code_mismatch`
- [ ] 普通消息（`/menu`、域名、6 位 TOTP、含字母）：不进比对，行为不变
- [ ] 纯 `--simplex`（`admin_id == None`）或 `simplex_admin_id == None`：不进比对
- [ ] 取码 Err：warn 后返回 false，不打断普通处理
- [ ] 回复走 `send_message_primary`

**Verification:**
- [ ] `cargo test --lib shared::dispatch`
- [ ] `cargo nextest run --cargo-profile fast-test -E 'test(dispatch)'`

**Dependencies:** S2, S4
**Files likely touched:** `rust/aegis/src/shared/dispatch.rs`

---

## Task S6: i18n 三语

**Description:** 改 `simplex.security_code` 为「请在客户端复制后直接发到这里自动验证」；新增 `simplex.code_verified`、`simplex.code_mismatch`。zh/en/ja 齐全。

**Acceptance criteria:**
- [ ] 三个 locale 文件都含 3 个 key，无缺漏
- [ ] `simplex.security_code` 保留 `%{0}` 占位
- [ ] parity 测试通过

**Verification:**
- [ ] `cargo nextest run --cargo-profile fast-test -E 'test(i18n_parity)'`
- [ ] `cargo test --doc`

**Dependencies:** S3, S5
**Files likely touched:** `rust/aegis/src/resources/i18n/zh.yml`, `en.yml`, `ja.yml`

---

## Task S7: 质量门 + 真机验收 + REVIEW

**Description:** 跑完整质量门，按 spec §6 逐条验收；fresh-context 子代理做 code-review-and-quality 五轴审查。

**Acceptance criteria:**
- [ ] `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc` 全绿
- [ ] spec §6 可测条件逐条有测试证据；真机项记录证据或标注无法在本环境验证及原因
- [ ] 五轴 review 无 Critical

**Verification:**
- [ ] 上述命令输出
- [ ] review 报告路径

**Dependencies:** S1–S6
**Files likely touched:** 无（验证）；发现问题回到对应任务

---

## 边界

- 不新增 CLI flag、不改 `RoutingAdapter`、不改能力表、不改 `BotAdapter` 签名。
- 不调 `/_verify`（spec §7 后续项）。
- 不重构 `auth.rs` / `dispatch.rs` 其它部分。
- 每次 writer 只在一个 worktree 改动，避免并发写冲突。

## Risks and Mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| `BotSettings` 整体重写抹掉新字段 | 高 | S1 读改写 + 回归测试 |
| 入站判据过宽吞掉普通消息 | 高 | `looks_like_security_code` 严格（纯数字空白 + >=12 位）+ 未验证前置 + 普通消息用例 |
| 重钉后仍显示「已验证」 | 中 | 判定用「存储 contactId == 当前 simplex_admin_id」 |
| 取码失败阻塞普通消息 | 中 | fail-open 返回 false |
