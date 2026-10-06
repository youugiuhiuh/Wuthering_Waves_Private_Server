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
