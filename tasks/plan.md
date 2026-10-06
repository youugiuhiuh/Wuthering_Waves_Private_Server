# Implementation Plan: 路由自定义放行（Xray）

> 对应 `SPEC.md` 的 `custom-allowlist` 模块。状态：**待批准**，批准后开始 TDD 实现。
> 任务勾选清单见 `tasks/todo.md`。

## Overview

给 Telegram 网关增加「自定义放行」能力：管理员输入域名（`decodo.cn`）→ 规范化成 Xray `domain:` 项 → 写入 `00_base.json` 的 `routing.rules`，插在 `connectivity_check` 之后、`cn_ip`/`cn_domain` 之前 → 重载核心。附列表/按序号删除/只读自检。仅 Xray，仅 `direct`，仅域名。

## Architecture Decisions

1. **规则载体 = 单条 `ruleTag: "custom_direct"` 规则**，`domain` 数组承载全部用户条目。不新建独立文件 → 天然随 `00_base.json` 备份/回滚，`update_geodata` 不覆盖。
2. **插入点 = 索引 1**（紧跟 `connectivity_check`）。两者都是 `direct`，先后无行为差异，但这样**不破坏**现有 `ensure_direct_rules_*` 的 6 个测试断言。
3. **空列表不落盘**：列表清空时整条规则移除（否则会改变既有测试断言的数组长度）。
4. **全部顺序/幂等逻辑写成纯函数**（`&mut Value` → `bool`）。原因：`00_base.json` 路径硬编码为 `/etc/wwps/...`，单测无法写生产路径；纯函数是唯一可测的载体（沿用 `ensure_direct_rules_value` 既有模式）。
5. **白名单用 `domain:` 而非 `full:`**：`geosite:cn` 大量条目是 `Full`（精确匹配），用 `domain:` 可覆盖子域并对未来新增子域免疫。
6. **自检只读且能力受限**：不解析 geo 数据 → 只确定性判定 `custom_direct` / `connectivity_check`，其余提示查核心日志（见 SPEC「真实能力边界」）。

## Dependency Graph

```
T1 域名规范化（纯）
      │
T2 规则 JSON 插入/迁移/删除（纯）        ← 依赖 T1 的输出格式
      │
T3 I/O 薄封装（add/remove_at/list + reload）
      │
      ├── T4 回调前缀路由 + 菜单骨架
      │        │
      │        ├── T5 三语文案
      │        └── T6 文本输入状态机
      │
      └── T7 生效自检（只读，纯判定 + 回调）
                 │
T8 集成 parity 测试 + 全量质量门  ← 依赖全部
```

## Task List

> **执行顺序调整（Phase 2）**：T5（三语文案）先于 T4（菜单）。菜单要引用 T5 新增的 i18n key，先有 key 再引用，保证每一步都留下可编译、测试全绿的树。
> **再调整**：T4 只做「菜单入口 + 列表 + 按序号删除」；「添加域名」按钮与路由归 T6（依赖输入状态机），「生效自检」按钮与路由归 T7。避免出现指向未实现 handler 的按钮。
> **Phase 3 拆片（每片 ≤ 3 文件）**：
> - T6a = 独立输入来源 + `message.rs` 先按来源分流（`core/types.rs` + `handlers/message.rs`），含「CustomAllowlist 不进入 ACME 路径」的锁死测试
>   注：新增枚举变体会让同 crate 两处穷尽 match 编译失败（`shared/dispatch.rs:310`、`handlers/xray.rs:225`），故 T6a 实际动 **4 个文件**（各 +1 行 arm）。这是编译强制的下限，不是范围蔓延，已批准。xray.rs 的映射细化仍归 T6b；工作流严格顺序执行，故无并发写冲突。
> - T6b = 「添加域名」按钮 + handler（`handlers/xray.rs`）
> - T7a = 自检纯判定函数（`core/xray/routing.rs`）
> - T7b = 自检输入流程 + 按钮/handler（`core/types.rs` + `handlers/message.rs` + `handlers/xray.rs`）
>
> 注：T6b 需要把 T4 新写的「子菜单只有 list+back」测试更新为「list + add + back」。那是本功能自己的新测试，可以随功能演进修改（不属「既有测试」保护范围）。

### Phase 1 — 纯逻辑（TDD 主场）

- [ ] **T1: 域名规范化与校验纯函数**
  - 实现 `normalize_custom_domain(&str) -> Result<String, CustomDomainError>`：按 SPEC 的 6 步归一化 + 9 类拒绝 + 长度/数量校验。
  - Acceptance: 接受 6 种形式、拒绝 9 类非法输入（逐条对应 SPEC 表格）；`cn` / `localhost` 被拒；IDN 转 punycode；大写转小写；尾点去除。
  - Verify: `cargo nextest run --cargo-profile fast-test normalize_custom_domain`
  - Files: `rust/aegis/src/core/xray/routing.rs`
  - Scope: S（1 file）

- [ ] **T2: 规则 JSON 插入/迁移/删除纯函数**
  - 实现 `ensure_custom_direct_value(&mut Value, &[String]) -> bool`、`remove_rule_by_tag`、`upsert_after`、`custom_direct_rule_json`。
  - Acceptance: 索引恰为 `connectivity_check` 之后、`private_ip` 之前；空列表 → 整条移除；幂等（二次调用返回 false）；**不改动** `connectivity_check` 内容；既有规则顺序保持。
  - Verify: `cargo nextest run --cargo-profile fast-test custom_direct`
  - Files: `rust/aegis/src/core/xray/routing.rs`
  - Scope: S（1 file）

### Checkpoint 1（纯逻辑）
- [ ] `cargo fmt` + `cargo clippy --all-targets --all-features -- -D warnings` 通过
- [ ] T1/T2 单测通过；**既有** `ensure_direct_rules_*` 与 `test_connectivity_check_*` 全部仍通过
- [ ] 人工确认：断言里明确写了「`custom_direct` 索引 < `cn_domain` 索引」

### Phase 2 — 持久化与菜单

- [ ] **T3: I/O 薄封装**
  - 实现 `add_custom_direct_domain`、`remove_custom_direct_at(idx)`、`list_custom_direct_domains`：读 `00_base.json` → 纯函数 → 写回 → `reload_core()`；全程复用既有 `CONFIG_LOCK`。
  - Acceptance: 写盘加锁；越界序号返回 Err 且**不写盘**；列表清空 → 规则整体移除；写后触发 reload。
  - Verify: `cargo nextest run --cargo-profile fast-test custom_direct`（纯函数路径）；手工在部署机验证一次写盘
  - Files: `rust/aegis/src/core/xray/routing.rs`
  - Scope: S（1 file）

- [ ] **T4: 回调前缀路由 + 菜单骨架**
  - `route_callback` 新增 `m_routing_custom` / `routing_custom_add` / `routing_custom_list` / `routing_custom_del:<idx>` / `routing_custom_check` 路由；`handle_routing_menu` 增加入口按钮；新增子菜单渲染。
  - Acceptance: 新按钮全部被路由（`test_every_menu_button_data_is_routed` 通过）；入口按钮位于 `m_routing` 菜单内、**在「返回」之前自成一行**、**不带** ✅/⬜（与开关型按钮可区分）、文本含当前条数；子菜单可进可回。
  - 依据：`handle_routing_menu`（`handlers/xray.rs:585`）现有结构 = 8 条规则开关按钮 + 返回行。见 SPEC「菜单位置与按钮」。
  - Verify: `cargo nextest run --cargo-profile fast-test menu_button`
  - Files: `rust/aegis/src/shared/handlers/mod.rs`, `rust/aegis/src/shared/handlers/xray.rs`
  - Scope: S（2 files）

- [ ] **T5: 三语文案**
  - `i18n/{zh,en,ja}.yml` 增加菜单/提示/错误文案（key 清单见 SPEC「文案规范」）。
  - Acceptance: 三语 key 齐全、无缺失；**输入引导文案必须含格式示例与不支持项**（用户明确要求）；子菜单标题含「仅对域名生效，覆盖不了纯 IP 访问」；自检文案含「geo 类规则无法本地判定」。
  - Verify: `cargo nextest run --cargo-profile fast-test i18n` + 全量 `cargo nextest run --cargo-profile fast-test`
  - **需人工审阅文案**：交付时把三语实际文本贴出来确认，不自作主张定稿。
  - Files: `rust/aegis/src/resources/i18n/zh.yml`, `en.yml`, `ja.yml`
  - Scope: S（3 files）

### Checkpoint 2（持久化与菜单）
- [ ] 四条质量门全绿
- [ ] 手工：在 bot 里能进入子菜单、能触发添加流程（此时尚未接输入状态机，可先接临时桩）
- [ ] 与人工确认一次菜单文案

### Phase 3 — 交互

- [ ] **T6: 文本输入状态机**
  - 复用 `DomainInputStep::AwaitDomain` + `DomainFlowSource` 新增变体（或等价隔离手段）；接上 T1 的校验与 T3 的写盘；非法输入回提示、不写盘；带超时。
  - ⚠️ **高风险耦合**：`message.rs:74-190` 目前对 `DomainInputStep::AwaitDomain` **无条件**走 ACME 路径（`AcmeManager::validate_domain` → 可能触发证书签发）。**必须先按来源分流再进 ACME 分支**，否则用户在「添加放行域名」时输入的域名会被当成申请证书的域名。
  - Acceptance: 输入 `decodo.cn` → 写盘并回报；非法输入（如 `https://x/y`、`cn`）→ 提示且不写盘；**ACME 域名输入流程逐项不受影响**（用既有 `message.rs` 测试 + 新增一条「放行来源不走 ACME」测试锁死）。
  - Verify: `cargo nextest run --cargo-profile fast-test message`
  - Files: `rust/aegis/src/shared/handlers/message.rs`, `rust/aegis/src/app/state.rs`
  - Scope: S（2 files）

- [ ] **T7: 生效自检（只读）**
  - 纯函数判定「是否命中 `custom_direct`（第 N 项）/ `connectivity_check`」；回调输出结论 + 「其余规则请查核心日志」提示。
  - Acceptance: 只读、不写盘、不 reload；命中/未命中均有明确输出；**不宣称**能判定 `geosite:cn` / `geoip:cn` / `category-ads-all`。
  - Verify: `cargo nextest run --cargo-profile fast-test custom_direct_check`
  - Files: `rust/aegis/src/core/xray/routing.rs`, `rust/aegis/src/shared/handlers/xray.rs`
  - Scope: S（2 files）

### Checkpoint 3（交互闭环）
- [ ] 端到端手工验收：输入 `decodo.cn` → 规则落盘在正确索引 → 自检报「命中自定义放行」→ `dashboard.decodo.cn` 可登录
- [ ] 删除该条目 → 规则消失 → 自检报「未命中」

### Phase 4 — 收口

- [ ] **T8: 集成 parity 测试 + 全量质量门**
  - 三语文案 key 存在性测试：照抄 `core/i18n.rs::domain_translation_keys_exist` 的既有模式（显式 key 列表 + 遍历三语断言 `!value.is_empty() && value != key`）。
  - ⚠️ **不得用「三语 key 数量相等」做 parity** —— zh 与 en/ja 的 2 空格缩进 key 数量本就不同，数量比较会误报。必须用显式 key 列表。
  - Acceptance: 新增的 `xray.routing_custom_*` key 在三语中均非空且不等于 key 本身；四条质量门全绿。
  - Verify: 四条质量门命令
  - Files: `rust/aegis/src/core/i18n.rs`
  - Scope: S（1 file）

### Checkpoint 4（完成）
- [ ] SPEC 的 12 条 Success Criteria 逐条勾选
- [ ] 四条质量门全绿
- [ ] 人工 review 后按 `git-workflow-and-versioning` 做原子提交

## Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| 规则索引写错 → 静默失效（历史 bug 复现） | High | T2 单测显式断言 `custom_direct` 索引 < `cn_domain` 索引；Checkpoint 1 人工确认 |
| 与 `ensure_direct_rules_value` 互相覆盖/移位 | High | T2 覆盖「两者共存」场景；若必须改既有函数语义 → **Ask first**（SPEC Boundaries） |
| `00_base.json` 是硬编码生产路径，单测无法写盘 | Med | 逻辑全放纯函数；I/O 只做薄封装；沿用「路径不存在则跳过」的既有测试惯例 |
| 输入状态机与 ACME 域名输入互相干扰 | **High** | `AwaitDomain` 当前**无条件**走 ACME（可能触发证书签发）。必须先按来源分流；T6 加「放行来源不走 ACME」单测 + 既有 `message.rs` 测试全通过 |
| 自检被误解为「完整规则判定」 | Med | 文案明确边界；SPEC 写入能力表；Success Criteria 10 只要求受限版本 |
| 三语漏 key | Low | T8 parity 测试 |
| **先天测试隔离缺陷**（非本功能引入，REVIEW 阶段发现） | Med | `core/sni` 的 `t4_drawing_does_not_write_state_file` 在 `cargo test --lib` 下必失败；`core/i18n` 两个 locale 写入测试缺 `#[serial]` 导致间歇失败。前者**不修**（超范围，nextest 不受影响），后者**补 `#[serial]` 修根因**。已实验确证（纯净 HEAD 跑 4 次：sni 4/4 失败、i18n 1/4 失败） |
| 改动量蔓延到 sing-box | Med | SPEC Never 明确禁止；单任务 ≤3 文件 |

## Open Questions

1. **是否纳入 `geosite:` 前缀**——当前决定**不纳入**（v1 只收域名）。纳入后用户可自行覆盖整类场景，但也能整体绕过「中国域名封锁」。
2. **受限版自检是否够用**——当前决定**接受**。完整判定需单独立项解析 geo 数据。

## 需要人工在真机验证的项（无单测覆盖）

- `update_geodata` 执行后用户列表仍在（SPEC SC 8）
- 实际出站为 `direct`、`dashboard.decodo.cn` 登录可用（SPEC SC 2）
- WARP 路由未被影响（若启用了 WARP）

---

## Phase R — `routing.rs` 拆分（纯重构，对应 SPEC `routing-split`）

> 状态：**已完成并验证**（2026-10-06，worktree `refactor/routing-split`）。基线已实测：`cargo nextest run --cargo-profile fast-test xray` = **40 passed**（`routing.rs` 1325 行）。
> 拆分映射与不变量见 `SPEC.md` → `Module: routing-split`。勾选清单见 `tasks/todo.md`。

### 分片原则（每片 ≤ 3 文件、≤ ~200 行、每片后可编译且测试全绿）

- **R1 先立骨架的思路被否决**：先建空文件再迁移会让中间态出现重复定义（无法编译）。
  ⇒ **改为按符号分组的「加一份 + 同片删一份」**：每片结束时树可编译、测试可跑。

- [x] **R1: 迁移自定义放行纯逻辑（规范化 + 匹配 + 列表算术）**
  - 移入 `custom_direct.rs`：`CustomDomainError`、`is_ip_or_cidr`、`normalize_custom_domain`、`CUSTOM_DIRECT_LIMIT`、`CustomAddOutcome`、`append_unique`、`remove_at`、`normalize_host`、`entry_matches_host`、`match_custom_direct`、`matches_connectivity_check`（含全部 doc comment，逐字）。
  - `routing.rs` 删除上述块，并加 `pub use super::custom_direct::{CustomAddOutcome, CustomDomainError, match_custom_direct, matches_connectivity_check, normalize_custom_domain};`。
  - Acceptance: `matches_connectivity_check` 仍能读到 `ROUTING_RULES`（由 `routing.rs` 提供，`use super::routing::ROUTING_RULES`）；`xray` 域测试仍 40 passed；文件头注释含「为什么存在这个文件」。
  - Verify: `cargo nextest run --cargo-profile fast-test xray`
  - Files: `core/xray/custom_direct.rs`（新）、`core/xray/routing.rs`、`core/xray/mod.rs`
  - Scope: S（3 files）
  - 备注: `routing.rs` 中 `use serde_json::{Value, json}` 若因此片变为未使用，须同片修正（否则 clippy -D warnings 失败）。

- [x] **R2: 迁移规则 JSON 纯函数**
  - 移入 `custom_direct.rs`（新 `impl RoutingManager` 块）：`custom_direct_rule_json`、`remove_rule_by_tag`、`upsert_after`、`ensure_custom_direct_value`。
  - Acceptance: `test_custom_direct_rule_json_shape` / `test_custom_direct_index_precedes_cn_domain_regression` / `test_custom_direct_idempotent_empty_list_and_missing_containers` / `test_custom_direct_overwrites_moves_and_preserves_order` / `test_remove_and_upsert_after_report_change` 全部原样通过。
  - Verify: `cargo nextest run --cargo-profile fast-test custom_direct`
  - Files: `core/xray/custom_direct.rs`、`core/xray/routing.rs`
  - Scope: S（2 files）

- [x] **R3: 迁移 I/O 薄封装 + 打开最小可见性**
  - 移入 `custom_direct.rs`：`custom_direct_domains_from`、`persist_base_json`、`add_custom_direct_entry`、`remove_custom_direct_at`、`list_custom_direct_domains`。
  - `routing.rs`：`CONFIG_LOCK` → `pub(super)`；`read_base_json` → `pub(super)`。
  - Acceptance: 三个方法体内**一字未改**（`git diff` 中 `assert` 行增删 = 0；`anyhow::bail!` 的越界文案原样）；`list_custom_direct_domains` 仍持锁只读。
  - Verify: `cargo nextest run --cargo-profile fast-test xray`
  - Files: `core/xray/custom_direct.rs`、`core/xray/routing.rs`
  - Scope: S（2 files）

- [x] **R4: 迁移测试并核对不变量**
  - 把 `routing.rs::tests` 中 25 个 custom_direct 测试 + 其夹具搬到 `custom_direct.rs::tests`；`custom_direct.rs::tests` 内另存 `base_with_rules` 等价副本（`direct_chain()` 依赖它）。
  - Acceptance（SPEC Success Criteria 1–7）：
    - `wc -l` 两文件均 < 1000；
    - `grep -c 'fn test_' routing.rs` = 17、`custom_direct.rs` = 23、合计 40；
    - 行级多重集差集的**消失集恰为 2 行**（`CONFIG_LOCK` / `read_base_json` 的可见性改动）；
    - 测试函数名集合与拆分前完全相等（`fn test_[a-z_]*` 提取后 set 对比）；
    - 测试**名称集合**与拆分前完全一致（用 `cargo nextest list` 前后对比）；
    - `handlers/` 零改动（`git status` 中不出现）。
  - Verify: `cargo nextest list --cargo-profile fast-test -p aegis xray` 前后 diff；`wc -l`；四条质量门
  - Files: `core/xray/custom_direct.rs`、`core/xray/routing.rs`
  - Scope: S（2 files）

### Checkpoint R（重构完成）
- [x] 四条质量门全绿（`cargo fmt` / `cargo clippy --all-targets --all-features -- -D warnings` / `cargo nextest run --cargo-profile fast-test` / `cargo test --doc`）
- [x] 全量通过数与拆分前一致（1097 passed / 1 skipped）
- [x] 人工对比 `git diff`：确认无行为变更、无测试断言改动、无顺手重构
- [x] `code-review-and-quality` 五轴审查（重点：行为保持、接口稳定、注释随迁完整）

### Risks（routing-split）

| Risk | Impact | Mitigation |
|---|---|---|
| 移动中手滑改动注释/断言（静默语义漂移） | High | R4 的 `assert` 行 diff 不变量 + 测试名集合对比 + 逐字搬运 |
| `CONFIG_LOCK` 被复制成两把锁 → 并发写坏 `00_base.json` | **High** | `CONFIG_LOCK` **只留在 `routing.rs`**，`custom_direct.rs` 只 `use`；`grep -c 'static CONFIG_LOCK'` 必须 = 1 |
| `pub use` 转发漏项 → 调用方编译失败 | Med | 编译即暴露；R1 片内一次补全 5 个名 |
| 移入块变为 `unused import`（`Value`/`json`/`xray`） | Low | 每片跑 clippy `--all-targets -- -D warnings` |
| 测试搬入新模块后 `use super::*` 拿不到 `RoutingManager` | Low | `custom_direct.rs` 顶层 `use super::routing::RoutingManager;`（私有 `use` 对子模块可见，与现状同构） |

---

# Implementation Plan: essential-direct（Phase E）

> 对應 `SPEC.md` → `Module: essential-direct`。状态：**已批准并已完成（E0–E8，2026-10-06）**。
> 任务勾选清单见 `tasks/todo.md` → 「Phase E」。
> worktree：`/home/ub/Dark/wwps-worktrees/essential-direct`（分支 `feat/essential-direct`，base `04f34a5`）。

## Overview

新增内建规则 `essential_direct`（39 条，domain/geosite 混合）作为**独立菜单按钮**，位置紧接 `connectivity_check` 之后；修两个既有缺陷（迁移不 reload / `toggle` 把 direct 规则推到 blocked 之后）；自检泛化为「命中哪条内建规则」；并把 `connectivity_check` 5 条正規化为 `domain:` 前缀。**仅 Xray；不放行广告/追踪；不动 sing-box。**

## Architecture Decisions

1. **新规则而非扩 `connectivity_check`** — 后者被守护测试 `test_connectivity_check_targets_are_probe_endpoints_only` 锁死（「恰为这 5 项」+ 禁 CDN/遥测前缀）。菜单按钮自动生成（`handle_routing_menu` 遍历 `ROUTING_RULES`），无需改渲染层。
2. **条目一律显式前缀** — Xray 裸字符串 = `keyword:` 子字符串（官方文件），会产生 `www.gstatic.com.evil.com` 误命中并与自检函数语义不一致；`domain:` = apex＋子域。
3. **`geosite:apple-cn` / `geosite:microsoft-pki` 直接引用分类** — 由 geodata 维护（本机实测：apple-cn 165 条覆盖 20/21、零广告；microsoft-pki 6 条），避免抄写并随 geodata 更新。
4. **不可变式写纯函数** — 迁移逻辑留在 `ensure_direct_rules_value(&mut Value) -> bool`（`00_base.json` 路径硬编码，无法单测 I/O）；I/O 包装只做「读—纯函数—**有变更才写盘+reload**」。
5. **不变量泛化**：所有 `outbound == "direct"` 的规则必须按 `ROUTING_RULES` 顺序位于所有 blocked 规则之前。`toggle()` 仍 push（不改），由迁移在下次开菜单时修正——顺带修好既有 `openai` 死规则问题。
6. **自检向后兼容** — `matches_connectivity_check` 保留为薄封装（转调 `matches_builtin_direct`），避免动 `message.rs` 之外的调用方。

## Dependency Graph

```
E0 基线（worktree + 四道门）
      │
E1 RED：新规则存在/形状/顺序（含 len 8→9）
      │
E2 GREEN：RuleDef essential_direct（占位 1 条）+ 三语 i18n
      │─────────────┐
E3 清单不变式（前缀/denylist/唯一性）      E7 菜单按钮测试（依赖 i18n）
      │
E4 迁移泛化 + 变更才 reload（含 openai 回归）
      │
E5 自检泛化 matches_builtin_direct + message.rs 文案
      │
E6 索引/夹具同步（direct_chain、cd == essential_direct+1、cc 前缀正規化）
      │
E8 文档收口 + 四道门终检 + code-review
```

## Tasks

- [x] **E0：基线确认** ✅（2026-10-06）
  - Acceptance: worktree 建立；四道门在**改动前**全绿；基线数字写入本文件。
  - Verify: `cd rust/aegis && cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc`
  - Files: 无（只跑门）
  - Scope: —

- [x] **E1：RED — 新规则存在/形状/顺序**
  - Acceptance: `test_rule_def_constants_count` 期望 `9`；新增 `test_essential_direct_rule_shape`、`test_essential_direct_precedes_cn_rules`（`cc < ed < cn_ip/cn_domain`）；此时跑测必须**红**（规则不存在）。
  - Verify: `cargo nextest run --cargo-profile fast-test xray` → 预期失败信息含 `essential_direct`
  - Files: `src/core/xray/routing.rs`（仅 tests）
  - Scope: S（1 file）

- [x] **E2：GREEN — RuleDef + i18n**
  - Acceptance: `ROUTING_RULES` 新增 `essential_direct`（插在 `connectivity_check` 之后，先放 `domain:recaptcha.net` 一条占位）；三语新增 `xray.routing_rule_essential_direct`：zh「外网必需服务直连」/ en `Essential Services Direct` / ja「必須サービスをダイレクト」；E1 测试转绿。
  - Verify: `cargo nextest run --cargo-profile fast-test xray`；`cargo fmt`
  - Files: `src/core/xray/routing.rs`、`src/resources/i18n/{zh,en,ja}.yml`、`src/core/xray/config.rs`
  - 注：`src/core/xray/config.rs` 仅测试期望同步（默认启用规则集 4→5），经 Ruling 7 授权。
  - Scope: S（4 files，各 1–3 行）

- [x] **E3：RED→GREEN — 清单不变式与完整清单**
  - Acceptance: 先写四个测试（前缀/denylist/必需项在场/唯一小写无 scheme），跑必红；再补全 39 条使其转绿；`test_essential_direct_excludes_ads_and_tracking` 的 denylist 含 14 个具体域名 + 8 个前缀模式。
  - Verify: `cargo nextest run --cargo-profile fast-test essential_direct`
  - Files: `src/core/xray/routing.rs`
  - Scope: M（~120 行：清单 39 + 测试 4）

- [x] **E4：RED→GREEN — 迁移泛化 + 变更才 reload**
  - Acceptance: 全量 `cargo nextest run --cargo-profile fast-test` 绿（含 5 个既有测试的 len/tags 期望同步，Ruling 10/11 授权）。
  - 步骤 1d（把 `test_ensure_direct_rules_updates_stale_targets_at_index_zero` 与 `test_ensure_direct_rules_emits_expected_json_shape` 的期望同步为 `domain:` 前缀）：**已作废**，移至 E6（Ruling 9）。
  - Verify: `cargo nextest run --cargo-profile fast-test xray`；`git diff` 审阅：无变更路径不得出现 `reload_core`
  - Files: `src/core/xray/routing.rs`
  - Scope: M（~100 行）

- [x] **E5：RED→GREEN — 自检泛化**
  - Acceptance: `matches_builtin_direct(host) -> Option<&'static str>`（返回规则 id）；`matches_connectivity_check` 保留为薄封装；`custom_check_reply` 用命中规则 id 取 i18n 名；新增 `test_matches_builtin_direct_reports_rule_id`、`test_matches_builtin_direct_rejects_substring_false_positive`。
  - Verify: `cargo nextest run --cargo-profile fast-test`（全量）+ `cargo clippy --all-targets --all-features -- -D warnings`
  - Files: `src/core/xray/custom_direct.rs`、`src/shared/handlers/message.rs`
  - Scope: M（~70 行）

- [x] **E6：cc 正規化 + custom_direct 锚点 + E3 deferred minors（四部分）**
  - (1) cc 正規化：`connectivity_check` 5 条加 `domain:` 前缀，并同步其 3 个测试——`test_connectivity_check_targets_are_probe_endpoints_only`（剥前缀后仍「恰为这 5 项」）、`test_ensure_direct_rules_emits_expected_json_shape`、`test_ensure_direct_rules_updates_stale_targets_at_index_zero`（Ruling 9）。
  - (2) custom_direct 锚点改 `essential_direct`：`ensure_custom_direct_value` 的锚点由 `connectivity_check` 改为 `essential_direct`，并同步索引测试（`cd == tag_index("essential_direct") + 1` 且仍 `< cn_ip/cn_domain`）与 `direct_chain()` 夹具（Ruling 12）。
  - (3) apex denylist 断言：`essential_direct` 条目剥前缀后不得等于 apex `google.com`/`googleapis.com`/`gstatic.com`（E3 minor-2）。
  - (4) 注释简繁统一为简体（E3 minor-3）。
  - Verify: `cargo nextest run --cargo-profile fast-test custom_direct`
  - Files: `src/core/xray/routing.rs`、`src/core/xray/custom_direct.rs`
  - Scope: S（2 files）

- [x] **E7：菜单按钮**
  - Acceptance: `handle_routing_menu` 注释「8 条」→「9 条」；新增断言：菜单按钮包含 `routing_toggle:essential_direct`，文字 = `t!("xray.routing_rule_essential_direct")`；三语 key 存在性测试涵盖新 key。
  - Verify: `cargo nextest run --cargo-profile fast-test xray`；真机目视（部署后）
  - Files: `src/shared/handlers/xray.rs`
  - Scope: S（1 file）

- [x] **E8：收口**
  - Acceptance: SPEC/plan/todo 与本实现一致；四道门全绿；`code-review-and-quality` 五轴审查完成，Critical 为零；给出交付建议（PR / merge / keep）。
  - Verify: 四道门 + 审查报告 + `git log --oneline` 原子提交串
  - Files: `SPEC.md`、`tasks/plan.md`、`tasks/todo.md`
  - Scope: S（文档）

### Checkpoint E（每个 M 级任务后）
- [ ] 只跑本域快速回路 `cargo nextest run --cargo-profile fast-test xray` ≤ 30 秒，失败不进入下一任务。
- [ ] `git diff --stat` 单片 ≤ 200 行、≤ 3 文件；超出就拆。
- [ ] 每任务一个原子提交（`feat(essential-direct): ...` / `fix(routing): ...` / `test(...): ...`）。

## Baseline（E0 实测，2026-10-06）

| 门 | 基线结果 |
|---|---|
| `cargo fmt --check` | **OK** |
| `cargo clippy --all-targets --all-features -- -D warnings` | **OK**（`Finished dev profile in 35.45s`，无 clippy 告警；仅既有构建脚本 warning） |
| `cargo nextest run --cargo-profile fast-test` | **1097 passed / 1 skipped**（summary 5.7s） |
| `cargo test --doc` | **ok**（0 tests） |

> 加速跑法（避免 fresh worktree 冷构建）：`CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`（共享主树 86G 缓存）。
> 基线数字与 `tasks/todo.md` 里 custom-allowlist/routing-split 记录的终数为同一终数（1097 passed / 1 skipped）。

## Risks（essential-direct）

| Risk | Impact | Mitigation |
|---|---|---|
| 清单把广告/追踪域名一并放行（违反用户硬约束） | **High** | denylist + 前缀双断言，并把 14 个具体域名写进测试 |
| 裸字符串条目被 Xray 当成子字符串 → 越权匹配 `www.gstatic.com.evil.com` | High | 强制显式前缀测试；`cc` 5 条一并正規化；自检同步 |
| 迁移不 reload 未修好 ⇒ 存量机器上功能「不存在」（菜单 ✅、核心旧规则） | **High** | E4 只在该变时调 `reload_core`；真机验收步骤：改完后笺看核心重启与 `Reading 00_base.json` |
| `ensure_direct_rules_value` 泛化后破坏既有只插 `connectivity_check` 的语义 | Med | 保留原有四分支行为（无/错位/过时/正常）并新增 direct-ordering 不变量测试 |
| `geosite:apple-cn` 不存在于旧版 `geosite.dat` | Low | 本机实测存在（165 条）；geodata 更新走既有 `update_geodata` + reload；若缺失仅少一层覆盖，不会导致核心启动失败 |
| 与 sing-box 侧行为不一致（本模块不碰） | Low | 已在 SPEC 非目标与遗留项里明写 |
| diff 超 200 行/片 | Med | E3 拆为「测试先行」+「清单补齐」两个提交 |
