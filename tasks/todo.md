# TODO: Matrix 原生可点菜单（MSC3381 poll 承载 inline keyboard）

Source spec: `docs/specs/SPEC-matrix-poll-buttons.md`
Plan: `tasks/plan.md`
Baseline: `81a6f4e`，`cargo +nightly nextest run --cargo-profile fast-test` → **1400 passed, 1 skipped**

> 并发边界：T1→T2→T3 同在 `poll.rs`、T4 在 `adapter.rs`，**串行**；
> 只有 **T5 ｜ T6 可并行**（`adapter.rs` ｜ `runtime.rs`，互不写同一文件）。

## Phase 0 — 依赖可行性（硬门禁）

- [ ] T1 加 `ruma@0.18 + unstable-msc3381`，`poll_start_content` 构造器（先红后绿）
  - Acceptance：MSER3381 形状序列化断言 3 例全绿；`Cargo.lock` 只有一份 ruma
  - Verify：`cargo +nightly nextest run -E 'test(poll_start_content)' --cargo-profile fast-test`
  - Files：`Cargo.toml`、`Cargo.lock`、`src/gateways/matrix/poll.rs`、`src/gateways/matrix/mod.rs`
  - Deps：— ｜ Size：M
  - **门禁不通过 → 停，回退 spec §4 方案 A（自建 EventContent），不得继续**

## Phase 1 — 纯函数与状态表

- [ ] T2 `plan_send` 判定 + i18n `matrix.poll_question`
  - Acceptance：6 例（无 markup / 有 markup / row-major 拍平 / 20 边界 / 21 回退 / 长 label 截断 / 空 label 兜底）
  - Verify：`cargo +nightly nextest run -E 'test(plan_send)' --cargo-profile fast-test`
  - Files：`poll.rs`、`src/resources/i18n/{en,zh,ja}.yml`
  - Deps：T1 ｜ Size：M

- [ ] T3 `PollStore` + `answer_index`
  - Acceptance：5 例（解析边界 / 一次性消费 / 未知 poll 与越界不置位 / anchor 查找 / 64 上限淘汰）
  - Verify：`cargo +nightly nextest run -E 'test(take_unanswered) or test(find_by_anchor) or test(insert_evicts) or test(answer_index)'`
  - Files：`poll.rs`
  - Deps：T2 ｜ Size：M

- [ ] Checkpoint 1：`cargo clippy --all-targets -- -D warnings` 无新警告

## Phase 2 — adapter 接入

- [ ] T4 `send_message` 接入 + 既有降级测试改造 + README 已知限制
  - Acceptance：既有「编号清单」断言改为「正文不含内部命令 + 产出 poll」；poll 发送失败只 warn
  - Verify：`cargo +nightly nextest run -E 'test(send_message)' --cargo-profile fast-test`
  - Files：`adapter.rs`、`README.md`
  - Deps：T1–T3 ｜ Size：M

## Phase 3 — 刷新与入站（并行组）

- [ ] T5 `take_by_anchor` + `edit_message`/`delete_message` 接入
  - Acceptance：2 例（返回并移除一次 / 不误删别的 anchor）；redact 失败只 warn
  - Verify：`cargo +nightly nextest run -E 'test(take_by_anchor)' --cargo-profile fast-test`
  - Files：`poll.rs`、`adapter.rs`
  - Deps：T4 ｜ Size：S

- [ ] T6 入站 poll 响应 → `CallbackEvent`
  - Acceptance：2 例（取第一个选择 / 空或非数字返回 None）；非管理员只留 debug 日志
  - Verify：`cargo +nightly nextest run -E 'test(response_index)' --cargo-profile fast-test`
  - Files：`src/main/runtime.rs`
  - Deps：T3、T4 ｜ Size：M
  - **可与 T5 并行**（不同文件）

- [ ] Checkpoint 2：全量 `cargo +nightly nextest run --cargo-profile fast-test` + `cargo fmt --check` 绿

## Phase 4 — 验收与收尾

- [ ] 真机验收（`--matrix` 单跑 + Element）：可点选项 → 点一项推进 → 旧 poll 消失 → 连点只进一次 → 非管理员点选无反应
- [ ] Branch review（`code-review-and-quality` 整支 diff）
- [ ] PR + 合并 + 清理 worktree
