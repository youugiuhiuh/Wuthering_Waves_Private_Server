# Todo: 用户回贴 SimpleX 安全码，Bot 自动验证

> Plan: `tasks/plan.md` ｜ Spec: `docs/superpowers/specs/2026-09-25-simplex-security-code-confirmation-design.md`

## Phase 1: Foundation

- [x] S1: BotSettings 持久化字段 + 读改写修复（bootstrap.rs, state_ops.rs）
- [x] S2: AppState 字段/方法 + main 接线（state.rs, main.rs）
- [x] S4: dispatch 纯函数 normalize / looks_like（dispatch.rs）

### Checkpoint: Foundation
- [x] `cargo test --lib` 相关用例通过，`cargo clippy` 无警告

## Phase 2: Core Features

- [x] S3: 出站门控（auth.rs）
- [x] S5: 入站确认 try_confirm_security_code（dispatch.rs）
- [x] S6: i18n 三语

### Checkpoint: Core Features
- [x] 一致/不一致/普通消息/无 TG 四类用例通过；i18n parity 通过

## Phase 3: Verify & Review

- [x] S7: 质量门（fmt/clippy/nextest/doc）+ 真机验收
- [x] S7: fresh-context 五轴 REVIEW（verdict BLOCK on C1）
- [x] S7: 修复 C1（断言用 `t!` 译文）+ H2（parity 覆盖 simplex key）+ M1（落盘失败用例）+ L1；H1/M3/M4 已在 spec §4 登记
- [x] 门禁复跑全绿（1008 passed, clippy -D warnings clean）

### Checkpoint: Complete
- [x] spec §6 可测项全部验收、atomic commit
- [x] 复核（fresh-context `explore` `715ef2f4`）：`Merge verdict: OK with notes`，C1/H2/M1/L1 已解决，H1/M2/M3/M4 已登记，无新增 Critical/High；唯一 notes = 新测试未还原 `AEGIS_CONFIG_DIR`（与既有 set_config_dir 同模式，serial 下无害）
- [ ] 真机验收（`--tg-simplex` 实机，需用户在真实环境执行）
