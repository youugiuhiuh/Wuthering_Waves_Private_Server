# TODO: REALITY fallback 流量兜底

Source spec: `docs/specs/SPEC-reality-fallback-limits.md`
Plan: `tasks/plan.md`

> 全部任务落在同一文件 `rust/aegis/src/core/xray/config.rs` → **串行执行，不并行派发**。

## Phase 1 — Foundation

- [x] T1 常量 + `patch_reality_fallback_limits`（先红后绿）
  - Acceptance：7 个 patch/生成器用例全绿（幂等、手工值、畸形输入）
  - Verify：`cargo +nightly nextest run -E 'test(reality_fallback)'`
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：— ｜ Size：S

- [x] T2 生成器注入（Vision + XHTTP 共用）
  - Acceptance：两协议 JSON 均含两字段且值 = 常量
  - Verify：同上定向命令
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T1 ｜ Size：S

- [x] Checkpoint 1：定向测试全绿 + diff 仅新增字段

## Phase 2 — Integration

- [x] T3 自愈接入 `ensure_base_config()`
  - Acceptance：存量补齐、幂等、容错、**不 reload**（避免与 `reload_core` 递归）、`warn` 一条汇总
  - Verify：定向测试（启动路径实机验证并入 T6）
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T1 ｜ Size：S

- [x] T4 自愈层测试（临时目录，无新依赖）
  - Acceptance：补齐 / 幂等 / 畸形跳过 / 缺失目录 / 权限保留，5 例全绿
  - Verify：`cargo +nightly nextest run -E 'test(reality_fallback)'`
  - Files：`rust/aegis/src/core/xray/config.rs`
  - Deps：T3 ｜ Size：S

- [x] Checkpoint 2：全量 nextest 绿；启动路径实机验证并入 T6

## Phase 3 — Verification

- [x] T5 质量门禁
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo +nightly nextest run --cargo-profile fast-test`
  - `CARGO_PROFILE_RELEASE_LTO=false cargo +nightly build --release`
  - Deps：T1–T4 ｜ Size：XS

- [ ] T6 手动验收（真实节点）
  - `xray -test` 通过
  - fallback：前 1 MiB 全速，之后 ≤512 KiB/s
  - 已认证用户 A/B 差异 ≤10%
  - 实测数值回填 spec §11
  - Deps：T5 ｜ Size：S

- [ ] Checkpoint 3：spec 验收清单全勾 + `code-review-and-quality` + `doubt-driven-development` 覆核
