# 禁回国流量默认启用 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `cn_ip`（geoip:cn）与 `cn_domain`（geosite:cn）两条禁回国流量规则设为新部署默认启用。

**Architecture:** 修改 `ROUTING_RULES` 静态表中两条规则的 `default_enabled` 为 `true`；`00_base.json` 生成逻辑（`.filter(|r| r.default_enabled)`）自动生效，无需改动生成代码。仅影响新部署（`ensure_base_config` 幂等，存量不动）。

**Tech Stack:** Rust（edition 2024），serde_json，tokio。

**Spec:** `docs/superpowers/specs/2026-09-01-cn-traffic-block-default-design.md`

## Global Constraints

- 改动文件：仅 `rust/aegis/src/core/xray/routing.rs`、`rust/aegis/src/core/xray/config.rs`（测试同文件）
- 不引入新依赖（dependency-management 技能不适用）
- 不新增迁移逻辑（D2：仅新部署）
- 不涉及 singbox（D3）
- 规则顺序保持 `ROUTING_RULES` 现有顺序不变（private_ip → cn_ip → cn_domain → …）
- 提交前必须通过：`cargo fmt && cargo clippy -- -D warnings && cargo test`（rust-lint-format 强制门槛）
- 工作区：`/home/fe/Dark/Wuthering_Waves_Private_Server/.worktrees/feat/cn-block-default`，所有命令在该目录 `rust/aegis/` 下执行

---

### Task 1: 更新测试为预期行为（RED）

**Files:**
- Modify: `rust/aegis/src/core/xray/config.rs:1097-1126`（`test_ensure_base_config_structure`）
- Modify: `rust/aegis/src/core/xray/routing.rs`（tests 模块，新增测试）

**Interfaces:**
- Consumes: 现有 `ROUTING_RULES`、`RoutingManager::rule_def_to_json`、`ConfigManager::ensure_base_config`
- Produces: 断言新默认行为的测试（Task 2 的 GREEN 判据）

- [ ] **Step 1: 修改 `test_ensure_base_config_structure` 断言 3 条默认规则**

将 `config.rs` 中（约 1125-1126 行）：

```rust
        let rules = base_config["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["ruleTag"], "private_ip");
```

改为：

```rust
        let rules = base_config["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 3);
        let tags: Vec<&str> = rules
            .iter()
            .filter_map(|r| r["ruleTag"].as_str())
            .collect();
        assert_eq!(tags, vec!["private_ip", "cn_ip", "cn_domain"]);
```

- [ ] **Step 2: 在 `routing.rs` tests 模块新增默认启用断言测试**

在 `routing.rs` tests 模块中（`test_rule_def_has_private_ip_default` 之后，约 184 行后）新增：

```rust
    #[test]
    fn test_rule_def_has_cn_rules_default_enabled() {
        for id in ["cn_ip", "cn_domain"] {
            let rule = ROUTING_RULES.iter().find(|r| r.id == id).unwrap();
            assert!(
                rule.default_enabled,
                "{} 应默认启用（禁回国流量）",
                id
            );
        }
    }
```

- [ ] **Step 3: 运行测试确认 RED**

Run: `cargo test core::xray::routing::tests::test_rule_def_has_cn_rules_default_enabled core::xray::config::tests::test_ensure_base_config_structure 2>&1 | tail -20`

Expected: FAIL（当前 `cn_ip`/`cn_domain` 的 `default_enabled` 为 `false`；结构测试 `rules.len()` 为 1 而非 3）

### Task 2: 翻转默认值（GREEN）

**Files:**
- Modify: `rust/aegis/src/core/xray/routing.rs:26-38`（`cn_ip` 与 `cn_domain` 的 `default_enabled`）

**Interfaces:**
- Consumes: Task 1 的测试
- Produces: 新默认行为（新部署 `00_base.json` 含 3 条默认规则）

- [ ] **Step 1: 翻转两条规则的 `default_enabled`**

`routing.rs` 中（约 26-38 行）：

```rust
    RuleDef {
        id: "cn_ip",
        rule_type: "ip",
        targets: &["geoip:cn"],
        outbound: "blocked",
        default_enabled: false,
    },
    RuleDef {
        id: "cn_domain",
        rule_type: "domain",
        targets: &["geosite:cn"],
        outbound: "blocked",
        default_enabled: false,
    },
```

改为两条的 `default_enabled: false,` → `default_enabled: true,`。其余字段与顺序不动。

- [ ] **Step 2: 运行测试确认 GREEN**

Run: `cargo test core::xray::routing::tests::test_rule_def_has_cn_rules_default_enabled core::xray::config::tests::test_ensure_base_config_structure 2>&1 | tail -10`

Expected: PASS（两条测试均通过）

- [ ] **Step 3: 全量测试 + 强制质量门槛**

Run: `cargo fmt && cargo clippy -- -D warnings 2>&1 | tail -5 && cargo test 2>&1 | grep -E "test result|FAILED"`

Expected: fmt 无输出、clippy 无 warning、全部 test result ok、无 FAILED

- [ ] **Step 4: 提交**

```bash
git add rust/aegis/src/core/xray/routing.rs rust/aegis/src/core/xray/config.rs
git commit -m "feat(routing): 禁回国流量（cn_ip/cn_domain）默认启用"
```

### Task 3: 收尾验证

**Files:** 无改动

- [ ] **Step 1: 验证提交内容**

Run: `git show --stat HEAD && git log --oneline -3`

Expected: 单 commit，含 routing.rs + config.rs 两个文件；`feat/routing` 在 `docs` 提交之后

- [ ] **Step 2: 向用户报告，按 finishing-a-development-branch 选择处置**

报告内容：新部署默认 3 条规则（private_ip/cn_ip/cn_domain）；存量部署不受影响；测试基线 610+ 全绿。

## 自检记录

- **Spec 覆盖**：D1（两条都开）→ Task 2 Step 1；D2（仅新部署，无迁移）→ 设计约束，无迁移代码；D3（只动 xray）→ 未触碰 singbox。测试更新（§改动范围）→ Task 1。
- **占位符**：无。
- **类型一致性**：`RuleDef` 字段名、`ruleTag` 断言值与现有代码一致。
