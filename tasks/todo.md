# Todo: 用户回贴 SimpleX 安全码，Bot 自动验证

> Plan: `tasks/plan.md` ｜ Spec: `docs/superpowers/specs/2026-09-25-simplex-security-code-confirmation-design.md`

## Phase 1: Foundation

- [ ] S1: BotSettings 持久化字段 + 读改写修复（bootstrap.rs, state_ops.rs）
- [ ] S2: AppState 字段/方法 + main 接线（state.rs, main.rs）
- [ ] S4: dispatch 纯函数 normalize / looks_like（dispatch.rs）

### Checkpoint: Foundation
- [ ] `cargo test --lib` 相关用例通过，`cargo clippy` 无警告

## Phase 2: Core Features

- [ ] S3: 出站门控（auth.rs）
- [ ] S5: 入站确认 try_confirm_security_code（dispatch.rs）
- [ ] S6: i18n 三语

### Checkpoint: Core Features
- [ ] 一致/不一致/普通消息/无 TG 四类用例通过；i18n parity 通过

## Phase 3: Verify & Review

- [ ] S7: 质量门（fmt/clippy/nextest/doc）+ 真机验收
- [ ] S7: fresh-context 五轴 REVIEW，无 Critical

### Checkpoint: Complete
- [ ] spec §6 全部验收、atomic commit、可合并
