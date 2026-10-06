# 任务清单：路由自定义放行（Xray）

> 详细验收标准与验证步骤见 `tasks/plan.md`；设计见 `SPEC.md`。
> 状态更新时间：Phase 1–3 已完成并由父会话独立核验。

## Phase 1 — 纯逻辑 ✅

- [x] T1 域名规范化与校验纯函数（`core/xray/routing.rs`）
- [x] T2 规则 JSON 插入/迁移/删除纯函数（`core/xray/routing.rs`）

### Checkpoint 1 ✅
- [x] `cargo fmt` + `cargo clippy --all-targets --all-features -- -D warnings`
- [x] T1/T2 单测通过 + 既有 `ensure_direct_rules_*` / `test_connectivity_check_*` 全通过（1061 passed）
- [x] 断言确认 `custom_direct` 索引 < `cn_domain` 索引（`test_custom_direct_index_precedes_cn_domain_regression`）
- [x] 删除行 = 0（既有测试一字未改）

## Phase 2 — 持久化与菜单 ✅

- [x] T3 I/O 薄封装：add / remove_at / list + reload（复用 `CONFIG_LOCK`）
- [x] T5 三语文案（18 key × 3 语言，zh 与 SPEC 逐字一致）
- [x] T4 回调前缀路由 + 菜单骨架（入口按钮为导航型、带条数、无 ✅/⬜；子菜单仅 list+back）

### Checkpoint 2 ✅
- [x] 四条质量门全绿（1074 passed）
- [x] 代码级核验：入口按钮无 ✅/⬜、子菜单无指向未实现 handler 的按钮
- [ ] **菜单文案人工确认**（待用户过目；zh 已按 SPEC 定稿，en/ja 由子代理翻译）

## Phase 3 — 交互 ✅

- [x] T6a 独立输入来源 + `message.rs` 先按来源分流（含 ACME 隔离锁死测试）
- [x] T6b 「添加域名」按钮 + handler
- [x] T7a 自检纯判定（`match_custom_direct` / `matches_connectivity_check`）
- [x] T7b 自检输入流程 + 按钮/handler（只读）

### Checkpoint 3 ✅
- [x] 四条质量门全绿（1096 passed / 1 skipped）
- [x] 删除行 = 0（9 个文件）
- [x] ACME 隔离锁死测试存在且通过
- [ ] 端到端真机验收（输入 `decodo.cn` → 落盘索引正确 → 自检命中 → 登录可用）—— 需部署机

## Phase 4 — 收口 ✅（仅剩真机验收与提交）

- [x] **附加修复**：`core/i18n.rs` 两个 locale 写入测试补 `#[serial]`（修先天竞态 flake，见下）
- [x] T8 三语 key 存在性回归测试（`routing_custom_translation_keys_exist`，显式 18 key × 3 语，带 `#[serial]` + 结尾恢复语言）
- [x] 四条质量门终检（1097 passed / 1 skipped）
- [x] **变异检验**：把 `ensure_custom_direct_value` 的锚点从 `connectivity_check` 改成 `cn_domain` → 回归断言立即红（`left: 4, right: 1`）→ 证明断言非空转
- [ ] 人工 review 后原子提交

## SPEC Success Criteria 逐条核对

| # | 条目 | 状态 | 证据 |
|---|---|---|---|
| 1 | 输入 `decodo.cn` → 规则落盘、含 `domain:decodo.cn`、索引早于 `cn_domain` | ✅ 逻辑层 | `test_custom_direct_index_precedes_cn_domain_regression`（含变异检验） |
| 2 | 该主机出站 `direct`、不再 blackhole | ⏳ 需部署机 | — |
| 3 | 重复添加幂等 | ✅ | `test_append_unique_is_idempotent_for_duplicate` |
| 4 | 删除后规则消失；清空后整条移除 | ✅ | `test_remove_at_*` / `test_custom_direct_idempotent_empty_list_and_missing_containers` |
| 5 | 非法输入被拒且不写盘 | ✅ | `test_normalize_custom_domain_rejects*` + `custom_allowlist_source_surfaces_normalizer_errors` |
| 6 | 三语文案齐全；`test_every_menu_button_data_is_routed` 通过 | ✅ | `routing_custom_translation_keys_exist` + mod.rs 路由测试 |
| 7 | 四条质量门全绿 | ✅ | 1097 passed / 1 skipped（skip 为既有） |
| 8 | `update_geodata` 后列表仍在 | ⏳ 需部署机 | 设计上成立（只写 `.dat`/`.db`/`.srs`） |
| 9 | 按序号删除，越界不写盘 | ✅ | `test_custom_del_idx_rejects_bad_input` / `test_remove_at_out_of_range_returns_none` |
| 10 | 生效自检只读 | ✅ | `check_source_is_read_only_and_leaves_acme_state_untouched` |
| 11 | 输入格式 6 接受 / 9 拒绝 | ✅ | `test_normalize_custom_domain_accepts` / `_rejects` |
| 12 | `connectivity_check` 内容与索引不受影响 | ✅ | 既有 `ensure_direct_rules_*` / `test_connectivity_check_*` 全通过（删除行=0） |

## 已知先天问题（非本功能引入，已实验确证）

| 问题 | 证据 | 处置 |
|---|---|---|
| `core::sni::selector::tests::t4_drawing_does_not_write_state_file` 在 `cargo test --lib` 下必失败 | 纯净 HEAD 跑 4 次，**4/4 失败** | **不修**（属 `core/sni` 子系统，超范围）。nextest 不受影响 |
| `core::i18n::tests::set_and_get_lang` / `default_lang_is_zh` 间歇失败 | 纯净 HEAD 跑 4 次，**1/4 失败** | **修根因**：补 `#[serial]`（本 crate 约定「Every locale writer is #[serial]」，这两个是漏网） |

> 说明：nextest 每测试独立进程，故全局 locale 不串、门是绿的；`cargo test` 单进程多线程才暴露。`rust-lint-format` 规定 nextest 为权威、`cargo test` 为兜底 —— 兜底路径此前是坏的。

## 真机人工验证（无单测覆盖）

- [ ] `update_geodata` 后用户列表仍在
- [ ] 实际出站为 `direct`、`dashboard.decodo.cn` 登录可用
- [ ] WARP 路由未被影响（若启用）

---

## Phase R — `routing.rs` 拆分（纯重构）⏳ 待批准

> 设计见 `SPEC.md` → `Module: routing-split`；分片/验收见 `tasks/plan.md` → `Phase R`。
> 目标：`routing.rs` 1325 行 → `routing.rs` ≈ 630 + 新增 `custom_direct.rs` ≈ 790，两文件均 < 1000。
> 铁律：**移动而非修改** —— 测试断言零增删、测试名集合不变、面积 40 项测试。

- [ ] **基线**：`cargo nextest run --cargo-profile fast-test xray` = 40 passed；`wc -l routing.rs` = 1325（已实测）
- [ ] **R1** 迁移纯逻辑（规范化 / 匹配 / 列表算术）+ `pub use` 转发 + `mod.rs`（3 files）
- [ ] **R2** 迁移规则 JSON 纯函数（`custom_direct_rule_json` / `remove_rule_by_tag` / `upsert_after` / `ensure_custom_direct_value`）（2 files）
- [ ] **R3** 迁移 I/O 薄封装（`add_` / `remove_` / `list_` + `custom_direct_domains_from` / `persist_base_json`）；`CONFIG_LOCK` 与 `read_base_json` 放行到 `pub(super)`（2 files）
- [ ] **R4** 迁移 25 个单测 + 夹具（含 `base_with_rules` 副本）；核对不变量（2 files）

### Checkpoint R
- [ ] `wc -l`：两文件均 < 1000
- [ ] `grep -c 'fn test_'`：15 + 25 = 40（与拆分前一致）
- [ ] `grep -c 'static CONFIG_LOCK'` = 1（只有一把锁）
- [ ] `assert` 行增删 = 0；测试名集合前后一致（`cargo nextest list` 对比）
- [ ] `handlers/` 零改动；仅 3 个代码文件被改
- [ ] 四条质量门全绿（1097 passed / 1 skipped，与拆分前一致）
- [ ] 人工 review `git diff`：无行为变更 / 无顺手重构 / 注释随迁完整
