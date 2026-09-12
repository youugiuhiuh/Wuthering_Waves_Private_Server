# Xray 连通性检测端点直连规则 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `www.gstatic.com` 与 `connectivitycheck.gstatic.com` 的连通性探测请求走 `direct` 而非被 `cn_domain` 规则 blackhole，并为已部署机器迁移存量配置。

**Architecture:** 在 `ROUTING_RULES` 数组**首位**新增一条 `domain` 类型、`outbound: direct` 的规则（数组顺序即 Xray 匹配优先级，必须早于 `cn_ip`/`cn_domain`）。新增一对纯函数 + 加锁包装的迁移函数，仿照 sing-box 既有的 `ensure_rule_sets_value` / `ensure_rule_sets_in_base` 模式，在 `get_all_with_status()` 入口对存量 `00_base.json` 补插该规则。

**Tech Stack:** Rust 2024 edition、`serde_json`、`tokio`（fs + Mutex）、`once_cell::sync::Lazy`。测试用内置 `#[cfg(test)]` + `#[tokio::test]`。

**Spec:** `docs/superpowers/specs/2026-09-12-xray-connectivity-check-direct-design.md`

## Global Constraints

- **仅改 Xray**。`rust/aegis/src/core/singbox/**`、`rust/aegis/src/shared/handlers/singbox.rs` **一行都不许动**。
- **规则必须插在数组首位**。顺序即优先级；排在 `cn_ip`/`cn_domain` 之后就完全不生效。
- **域名清单恰为 2 项**，且顺序固定：
  `"www.gstatic.com"`, `"connectivitycheck.gstatic.com"`
  不得添加 `fonts.` / `ssl.` / `csi.` / `g0-g3.gstatic.com`（资源 CDN，非探测端点），不得添加 `connect.rom.miui.com`（非 gstatic 系）。
- **写主机名，不写 URL**。不可出现 `https://` 或 `/generate_204` —— Xray 只匹配 SNI / Host。
- **不触发 `reload_core()`**。迁移函数只写盘，不重载核心。
- **`00_base.json` 不存在时不特殊处理**（已知遗留，见 spec §5）。
- **工作目录**：`rust/aegis` 是独立 crate，**无 workspace 根**。所有 cargo 命令必须在 `/home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis` 下执行。
- **已知无关 flake**：`shared::dispatch::tests::xhttp_domain_provider_routes_to_xray_handler` 偶发失败于并行全量运行，单独运行必过。若遇到，重跑确认，不要为本改动去修它。

---

## File Structure

| 文件 | 职责 | 动作 |
|---|---|---|
| `rust/aegis/src/core/xray/routing.rs` | 规则定义（`ROUTING_RULES`）、迁移逻辑（`ensure_direct_rules_*`）、`get_all_with_status` 入口 | 修改 |
| `rust/aegis/src/core/xray/config.rs` | `00_base.json` 初始生成；本次仅同步断言 | 修改（仅测试块） |
| `rust/aegis/src/resources/i18n/zh.yml` | 中文按钮文案 | 修改（+1 行） |
| `rust/aegis/src/resources/i18n/en.yml` | 英文按钮文案 | 修改（+1 行） |

`routing.rs` 当前 237 行，本次新增约 60 行（含测试），仍在可读范围，**不拆分**。

---

## Task 1: 规则定义与优先级断言

先落地规则本身与"必须首位"的回归防线。此任务不涉及迁移，独立可测。

**Files:**
- Modify: `rust/aegis/src/core/xray/routing.rs`（`ROUTING_RULES` 定义，13-68 行；`tests` 模块，170-237 行）
- Modify: `rust/aegis/src/core/xray/config.rs`（`test_ensure_base_config_structure`，1276-1278 行）

**Interfaces:**
- Consumes: 无
- Produces:
  - `RuleDef` 结构体新增实例，`id: &'static str = "connectivity_check"`，`rule_type: &'static str = "domain"`，`outbound: &'static str = "direct"`
  - `ROUTING_RULES` 长度由 7 变为 8，索引 0 为 `connectivity_check`

---

- [ ] **Step 1: 写失败测试 —— 规则数量与首位顺序**

在 `rust/aegis/src/core/xray/routing.rs` 的 `mod tests` 内，把现有的资源数量断言改掉，并新增优先级断言。

先定位现有测试（约 173 行）：

```rust
    #[test]
    fn test_rule_def_constants_count() {
        assert_eq!(ROUTING_RULES.len(), 7);
    }
```

替换为：

```rust
    #[test]
    fn test_rule_def_constants_count() {
        assert_eq!(ROUTING_RULES.len(), 8);
    }

    /// 回归防线：连通性检测规则必须先于 cn_ip / cn_domain。
    /// Xray routing 顺序匹配、首条命中即停；排在 cn 规则之后就完全不生效，
    /// 而 cn_domain（geosite:cn）会把这些探测域名 blackhole。
    #[test]
    fn test_connectivity_check_must_precede_cn_rules() {
        let pos = |id: &str| {
            ROUTING_RULES
                .iter()
                .position(|r| r.id == id)
                .unwrap_or_else(|| panic!("规则 {} 不存在", id))
        };
        assert!(
            pos("connectivity_check") < pos("cn_ip"),
            "connectivity_check 必须排在 cn_ip 之前"
        );
        assert!(
            pos("connectivity_check") < pos("cn_domain"),
            "connectivity_check 必须排在 cn_domain 之前"
        );
    }

    /// 域名清单必须恰为这 2 个探测端点，且不得混入资源 CDN。
    #[test]
    fn test_connectivity_check_targets_are_probe_endpoints_only() {
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "connectivity_check")
            .expect("connectivity_check 规则必须存在");
        assert_eq!(rule.rule_type, "domain");
        assert_eq!(rule.outbound, "direct");
        assert!(rule.default_enabled, "新规则应默认启用");
        assert_eq!(
            rule.targets,
            &["www.gstatic.com", "connectivitycheck.gstatic.com"],
            "域名清单必须恰为这 2 个探测端点"
        );
        // 不得出现 scheme 或路径 —— Xray 只匹配 SNI / Host
        for t in rule.targets {
            assert!(
                !t.contains("://") && !t.contains('/'),
                "targets 必须是纯主机名，不能含 scheme 或路径: {}",
                t
            );
        }
        // 不得混入同家族的资源 CDN（非探测端点）
        for t in rule.targets {
            assert!(
                !t.starts_with("fonts.")
                    && !t.starts_with("ssl.")
                    && !t.starts_with("csi.")
                    && !t.starts_with("g0.")
                    && !t.starts_with("g1.")
                    && !t.starts_with("g2.")
                    && !t.starts_with("g3."),
                "不应放行资源 CDN（非探测端点）: {}",
                t
            );
        }
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray::routing::tests 2>&1 | tail -30
```

Expected: **编译失败**，错误形如
`error[E0004]` / `error[E0308]`，或 `assert_eq!(ROUTING_RULES.len(), 8)` 处
`left == 7, right == 8`；`test_connectivity_check_must_precede_cn_rules` 因
`connectivity_check 不存在` panic。

> 注：本 Step 允许编译错误作为 RED 信号（规则尚未定义，无法调用）。真正跑起来的
> 断言失败会在 Step 3 之后显现。

- [ ] **Step 3: 实现 —— 插入规则定义**

在 `rust/aegis/src/core/xray/routing.rs`，找到：

```rust
pub static ROUTING_RULES: &[RuleDef] = &[
    RuleDef {
        id: "private_ip",
```

改为（在 `private_ip` **之前**插入新规则）：

```rust
pub static ROUTING_RULES: &[RuleDef] = &[
    // 必须位于首位：Xray routing 顺序匹配、首条命中即停。
    // geosite:cn 收录了这些探测域名，若排在 cn_ip / cn_domain 之后即失效。
    RuleDef {
        id: "connectivity_check",
        rule_type: "domain",
        targets: &["www.gstatic.com", "connectivitycheck.gstatic.com"],
        outbound: "direct",
        default_enabled: true,
    },
    RuleDef {
        id: "private_ip",
```

- [ ] **Step 4: 同步 config.rs 断言**

在 `rust/aegis/src/core/xray/config.rs` 的 `test_ensure_base_config_structure` 中，找到（约 1276-1278 行）：

```rust
        assert_eq!(rules.len(), 3);
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(tags, vec!["private_ip", "cn_ip", "cn_domain"]);
```

替换为：

```rust
        assert_eq!(rules.len(), 4);
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(
            tags,
            vec!["connectivity_check", "private_ip", "cn_ip", "cn_domain"]
        );
```

- [ ] **Step 5: 运行测试确认通过**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray 2>&1 | tail -12
```

Expected: `test result: ok.`，且包含
`core::xray::routing::tests::test_connectivity_check_must_precede_cn_rules ... ok`
与 `core::xray::config::tests::test_ensure_base_config_structure ... ok`。

- [ ] **Step 6: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git add rust/aegis/src/core/xray/routing.rs rust/aegis/src/core/xray/config.rs
git commit -m "feat(xray): 连通性检测端点直连规则（优先级 + 域名清单）

新增 connectivity_check 规则（domain / direct），插在 ROUTING_RULES 首位。
geosite:cn 收录 www.gstatic.com 与 connectivitycheck.gstatic.com，
被 cn_domain 规则 blackhole，导致客户端连通性探测失败。

新增回归断言：规则必须先于 cn_ip / cn_domain（顺序即优先级），
且域名清单恰为 2 个探测端点、不含 scheme/路径、不混入资源 CDN。"
```

---

## Task 2: 迁移纯函数

实现 `ensure_direct_rules_value` —— 纯函数、可独立测试、不碰文件系统。
按用户选定的 A 方案：用 `todo!()` 空壳先取得真实断言失败。

**Files:**
- Modify: `rust/aegis/src/core/xray/routing.rs`（`impl RoutingManager` 块；`mod tests`）

**Interfaces:**
- Consumes: Task 1 的 `ROUTING_RULES` 中 `id == "connectivity_check"` 的 `RuleDef`
- Produces:
  - `RoutingManager::ensure_direct_rules_value(v: &mut serde_json::Value) -> bool`
    返回 `true` 表示有变更（执行了插入），`false` 表示无变更（已存在）。
    副作用：若 `v["routing"]["rules"]` 缺失或非数组，会先将其置为数组。

---

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/core/xray/routing.rs` 的 `mod tests` 末尾（最后一个 `}` 之前）追加：

```rust
    // ── ensure_direct_rules_value ────────────────────────────────────────

    fn base_with_rules(rules: Value) -> Value {
        serde_json::json!({
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": rules
            },
            "outbounds": [
                {"protocol": "freedom", "settings": {}, "tag": "direct"},
                {"protocol": "blackhole", "settings": {}, "tag": "blocked"}
            ]
        })
    }

    #[test]
    fn test_ensure_direct_rules_inserts_at_index_zero() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]}
        ]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
        assert_eq!(rules[0]["outboundTag"], "direct");
        assert_eq!(rules[1]["ruleTag"], "cn_ip");
    }

    #[test]
    fn test_ensure_direct_rules_is_idempotent() {
        let mut v = base_with_rules(serde_json::json!([]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v), "首次应插入");
        let after_first = v["routing"]["rules"].clone();
        assert!(!RoutingManager::ensure_direct_rules_value(&mut v), "二次应无变更");
        assert_eq!(v["routing"]["rules"], after_first, "二次不得重复插入");
        assert_eq!(v["routing"]["rules"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_ensure_direct_rules_handles_missing_rules_key() {
        let mut v = serde_json::json!({"routing": {"domainStrategy": "IPIfNonMatch"}});
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
    }

    #[test]
    fn test_ensure_direct_rules_handles_missing_routing_key() {
        let mut v = serde_json::json!({"log": {"loglevel": "warning"}});
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        assert_eq!(v["routing"]["rules"][0]["ruleTag"], "connectivity_check");
    }

    #[test]
    fn test_ensure_direct_rules_preserves_existing_rules() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "private_ip", "outboundTag": "blocked", "ip": ["geoip:private"]},
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]},
            {"type": "field", "ruleTag": "cn_domain", "outboundTag": "blocked", "domain": ["geosite:cn"]}
        ]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(
            tags,
            vec!["connectivity_check", "private_ip", "cn_ip", "cn_domain"]
        );
    }

    #[test]
    fn test_ensure_direct_rules_emits_expected_json_shape() {
        let mut v = base_with_rules(serde_json::json!([]));
        RoutingManager::ensure_direct_rules_value(&mut v);
        let rule = &v["routing"]["rules"][0];
        assert_eq!(rule["type"], "field");
        assert_eq!(rule["outboundTag"], "direct");
        assert_eq!(
            rule["domain"],
            serde_json::json!(["www.gstatic.com", "connectivitycheck.gstatic.com"])
        );
        assert!(rule.get("ip").is_none(), "domain 规则不应带 ip 键");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray::routing::tests 2>&1 | tail -20
```

Expected: **编译失败**，`error[E0599]: no function or associated item named
'ensure_direct_rules_value' found for struct 'RoutingManager'`。

- [ ] **Step 3: 添加 `todo!()` 空壳**

在 `rust/aegis/src/core/xray/routing.rs` 的 `impl RoutingManager` 块内，
`get_all_with_status` 之前插入：

```rust
    /// 纯函数：确保 routing.rules 含 connectivity_check（插在首位，幂等）。
    /// 返回是否发生变更。不触碰文件系统，便于单测。
    pub fn ensure_direct_rules_value(_v: &mut Value) -> bool {
        todo!("Task 2 Step 5 实现")
    }

```

- [ ] **Step 4: 运行测试，确认是断言失败而非编译失败**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray::routing::tests 2>&1 | tail -25
```

Expected: 编译通过，测试**运行并失败**，6 个 `ensure_direct_rules_*` 测试均
`panicked at 'not yet implemented: Task 2 Step 5 实现'`。

> 这是本计划真正的 RED：测试可执行、失败原因是断言/未实现，而非语法。

- [ ] **Step 5: 实现**

把 Step 3 的空壳替换为：

```rust
    /// 纯函数：确保 routing.rules 含 connectivity_check（插在首位，幂等）。
    /// 返回是否发生变更。不触碰文件系统，便于单测。
    ///
    /// 插在首位是必须的：Xray routing 顺序匹配、首条命中即停，
    /// 排在 cn_ip / cn_domain 之后则完全不生效。
    pub fn ensure_direct_rules_value(v: &mut Value) -> bool {
        const RULE_ID: &str = "connectivity_check";

        // 规范化 routing 与 routing.rules 的存在性
        if v.get("routing").map(|r| r.is_null()).unwrap_or(true) {
            v["routing"] = Value::Object(serde_json::Map::new());
        }
        if v["routing"]["rules"].as_array().is_none() {
            v["routing"]["rules"] = Value::Array(Vec::new());
        }

        let rules = v["routing"]["rules"].as_array_mut().unwrap();
        let already = rules
            .iter()
            .any(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some(RULE_ID));
        if already {
            return false;
        }

        let def = ROUTING_RULES
            .iter()
            .find(|r| r.id == RULE_ID)
            .expect("ROUTING_RULES 必须包含 connectivity_check");
        rules.insert(0, Self::rule_def_to_json(def));
        true
    }

```

- [ ] **Step 6: 运行测试确认通过**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray::routing::tests 2>&1 | tail -20
```

Expected: `test result: ok.`，6 个 `ensure_direct_rules_*` 测试全部 PASS。

- [ ] **Step 7: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git add rust/aegis/src/core/xray/routing.rs
git commit -m "feat(xray): 迁移纯函数 ensure_direct_rules_value

为存量 00_base.json 补插 connectivity_check（插首位，幂等）。
兼容 routing / routing.rules 键缺失的旧配置。
纯函数、不触文件系统，6 个单测覆盖插入位置、幂等、缺键、保序与 JSON 形状。"
```

---

## Task 3: 迁移接线到 `get_all_with_status`

把纯函数接到文件系统与菜单入口。

**Files:**
- Modify: `rust/aegis/src/core/xray/routing.rs`（`impl RoutingManager`；`get_all_with_status`）

**Interfaces:**
- Consumes: `RoutingManager::ensure_direct_rules_value(&mut Value) -> bool`（Task 2）
- Produces:
  - `RoutingManager::ensure_direct_rules_in_base() -> anyhow::Result<()>`
    读 `/etc/wwps/wwps-core/conf/00_base.json`，调用纯函数，有变更则写回。
    **不调用 `reload_core()`**。
  - `get_all_with_status()` 行为扩展：入口处先执行迁移。

---

- [ ] **Step 1: 写失败测试 —— 迁移读写的字节级行为**

由于 `read_base_json` 硬编码 `xray::CONF_DIR`，无法注入临时目录，
本任务测试**纯函数与真实路径的交互契约**：即 `ensure_direct_rules_in_base`
必须存在、且当 `00_base.json` 缺失时返回 `Err`（已知遗留，spec §5 明确保留）。

在 `rust/aegis/src/core/xray/routing.rs` 的 `mod tests` 末尾追加：

```rust
    /// ensure_direct_rules_in_base 必须存在且返回 Result。
    /// 参照 sing-box 既有模式：文件缺失时向上传播错误（已知遗留，spec §5）。
    #[tokio::test]
    async fn test_ensure_direct_rules_in_base_errors_when_base_missing() {
        // 生产路径不存在时（CI / 未部署环境）应返回 Err，而非 panic。
        // 若该路径恰好存在（已部署机器），则跳过此断言。
        let base_path = format!("{}/00_base.json", crate::core::paths::xray::CONF_DIR);
        if tokio::fs::try_exists(&base_path).await.unwrap_or(false) {
            eprintln!("跳过：{} 已存在", base_path);
            return;
        }
        let r = RoutingManager::ensure_direct_rules_in_base().await;
        assert!(r.is_err(), "00_base.json 缺失时应返回 Err");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray::routing::tests 2>&1 | tail -20
```

Expected: **编译失败**，
`no function or associated item named 'ensure_direct_rules_in_base'`。

- [ ] **Step 3: 实现 `ensure_direct_rules_in_base`**

在 `rust/aegis/src/core/xray/routing.rs` 的 `impl RoutingManager` 块内，
`ensure_direct_rules_value` 之后插入：

```rust
    /// 迁移：确保 00_base.json 的 routing.rules 含 connectivity_check（幂等）。
    ///
    /// 只写盘，**不重载核心** —— 避免打断现有连接。由 get_all_with_status()
    /// 在用户打开路由菜单时触发。
    ///
    /// 注意：00_base.json 不存在时向上传播错误（已知遗留，见 spec §5）。
    pub async fn ensure_direct_rules_in_base() -> Result<()> {
        let _lock = CONFIG_LOCK.lock().await;
        let base_path = format!("{}/00_base.json", xray::CONF_DIR);
        let content = tokio::fs::read_to_string(&base_path)
            .await
            .context("读取 00_base.json 失败")?;
        let mut v: Value = serde_json::from_str(&content).context("解析 00_base.json 失败")?;
        if !Self::ensure_direct_rules_value(&mut v) {
            return Ok(());
        }
        let new_content = serde_json::to_string_pretty(&v).context("序列化配置失败")?;
        tokio::fs::write(&base_path, new_content)
            .await
            .context("写入 00_base.json 失败")?;
        Ok(())
    }

```

- [ ] **Step 4: 接线到 `get_all_with_status`**

找到：

```rust
    pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>> {
        let rules = Self::read_rules().await?;
```

改为：

```rust
    pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>> {
        // 首次进入菜单即完成存量迁移（旧部署的 base 缺 connectivity_check，幂等）
        Self::ensure_direct_rules_in_base().await?;
        let rules = Self::read_rules().await?;
```

- [ ] **Step 5: 运行测试确认通过**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib core::xray 2>&1 | tail -12
```

Expected: `test result: ok.`，含
`test_ensure_direct_rules_in_base_errors_when_base_missing ... ok`。

- [ ] **Step 6: 全量回归**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib 2>&1 | tail -8
```

Expected: `test result: ok. 683+ passed; 0 failed`。
若出现 `xhttp_domain_provider_routes_to_xray_handler` 失败，单独重跑确认 flake
（见 Global Constraints），**不要**为本改动去修它。

- [ ] **Step 7: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git add rust/aegis/src/core/xray/routing.rs
git commit -m "feat(xray): 存量配置迁移接线到 get_all_with_status

新增 ensure_direct_rules_in_base（读-迁移-写盘，不重载核心），
在 get_all_with_status 入口触发，与 sing-box 既有迁移模式对齐。
用户打开路由菜单时静默完成迁移，不打断现有连接。"
```

---

## Task 4: i18n 文案

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml:472`
- Modify: `rust/aegis/src/resources/i18n/en.yml:467`

**Interfaces:**
- Consumes: 无（`handlers/xray.rs:596` 用 `format!("xray.routing_rule_{}", def.id)` 动态拼接）
- Produces: i18n key `xray.routing_rule_connectivity_check`（zh + en）

---

- [ ] **Step 1: 中文文案**

在 `rust/aegis/src/resources/i18n/zh.yml`，找到：

```yaml
  routing_rule_openai: "OpenAI直连"
```

改为：

```yaml
  routing_rule_openai: "OpenAI直连"
  routing_rule_connectivity_check: "连通性检测直连"
```

- [ ] **Step 2: 英文文案**

在 `rust/aegis/src/resources/i18n/en.yml`，找到：

```yaml
  routing_rule_openai: "OpenAI Direct"
```

改为：

```yaml
  routing_rule_openai: "OpenAI Direct"
  routing_rule_connectivity_check: "Connectivity Check Direct"
```

- [ ] **Step 3: 验证 YAML 可解析、key 存在、既有条目未被破坏**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
python3 -c "
import sys
try:
    import yaml
except ImportError:
    print('pyyaml 不可用，改用文本校验'); sys.exit(0)
for f in ['rust/aegis/src/resources/i18n/zh.yml','rust/aegis/src/resources/i18n/en.yml']:
    d = yaml.safe_load(open(f))
    x = d['xray']
    assert 'routing_rule_connectivity_check' in x, f+' 缺新 key'
    assert x['routing_rule_private_ip'], f+' 既有 key 被破坏'
    assert x['routing_rule_openai'], f+' 既有 key 被破坏'
    print(f, 'OK  key=', x['routing_rule_connectivity_check'])
"
```

Expected: 两个文件均打印 `OK`，且能读出对应的中/英文案。
（若 pyyaml 不可用则打印跳过提示，此时改用 `cargo test` 全量验证。）

- [ ] **Step 4: 全量测试确认无回归**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib 2>&1 | tail -8
```

Expected: `test result: ok.`，失败数为 0。

- [ ] **Step 5: 提交**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml
git commit -m "i18n(xray): 新增 connectivity_check 规则按钮文案

仅新增 key，不改既有条目。handlers 动态拼接 key 无需改动。"
```

---

## Task 5: lint / format 与收尾验证

按 `rust-lint-format` 技能要求执行强制质量门。

**Files:**
- 无源码改动（除非 fmt/clippy 要求）

**Interfaces:**
- Consumes: Task 1-4 的全部改动
- Produces: 通过 fmt / clippy / test 的干净分支

---

- [ ] **Step 1: 格式化**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo fmt --all
cargo fmt --all -- --check && echo "FMT OK"
```

Expected: `FMT OK`。若有自动改动，需重新跑测试。

- [ ] **Step 2: clippy**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo clippy --all-targets -- -D warnings 2>&1 | tail -20
```

Expected: 无 error。若报 warning 且位于本次改动行，修复之；
若为既存 warning，记录但不在本任务修。

- [ ] **Step 3: 全量测试**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check/rust/aegis
cargo test --lib 2>&1 | tail -8
```

Expected: 0 failed。

- [ ] **Step 4: 确认未触碰 sing-box**

Run:
```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git diff --stat main...HEAD
echo "--- 必须为空 ---"
git diff --name-only main...HEAD | grep -E 'singbox' && echo "!! 违反了 sing-box 约束 !!" || echo "sing-box 未被触碰 OK"
```

Expected: 打印 `sing-box 未被触碰 OK`，且 diff 仅含 4 个预期文件。

- [ ] **Step 5: 提交（如 fmt/clippy 有改动）**

```bash
cd /home/ub/Dark/Wuthering_Waves_Private_Server/.worktrees/xray-connectivity-check
git add -A
git commit -m "chore(xray): cargo fmt + clippy 清理" || echo "无改动，跳过"
```

- [ ] **Step 6: 人工验证迁移行为（可选，需真实环境）**

对一台存在 `00_base.json` 的机器：

```bash
# 备份并查看迁移前
cp /etc/wwps/wwps-core/conf/00_base.json /tmp/base.bak
python3 -c "import json;d=json.load(open('/etc/wwps/wwps-core/conf/00_base.json'));print([r.get('ruleTag') for r in d['routing']['rules']])"
```

打开 Telegram 路由菜单，再查看：

```bash
python3 -c "import json;d=json.load(open('/etc/wwps/wwps-core/conf/00_base.json'));print([r.get('ruleTag') for r in d['routing']['rules']])"
```

Expected: `connectivity_check` 出现在列表**首位**，且原有规则顺序与内容不变。
再次打开菜单，列表不再变化（幂等）。

---

## Self-Review

**1. Spec coverage**

| Spec 章节 | 覆盖任务 |
|---|---|
| §3.1 新增规则（首位、2 域名、写主机名） | Task 1 Step 3；Task 1 Step 1 断言覆盖"首位/2项/无 scheme" |
| §3.2 存量迁移（纯函数 + 加锁包装 + 入口接线） | Task 2（纯函数）、Task 3（包装 + 接线） |
| §3.3 断言同步（`len` 7→8、`config.rs` tags） | Task 1 Step 1（len）、Step 4（config） |
| §3.4 i18n（zh/en 各 +1，仅新增 key） | Task 4 |
| §3.5 两个域名的身份（保留理由） | 已写入 spec；Task 1 Step 1 断言固定清单 |
| §5 已知遗留（文件缺失不特殊处理） | Task 3 Step 1/2 显式断言返回 `Err` |
| §6 测试策略 6 项 | Task 1（2 项）、Task 2（4 项）、Task 3（1 项） |
| §8 验证方式（test/fmt/clippy） | Task 5 |
| Global：不触碰 sing-box | Task 5 Step 4 显式校验 |

**2. Placeholder scan** — 无 TBD / TODO / 「稍后实现」/「类似 Task N」。所有代码块为完整可粘贴内容。唯一刻意保留的 `todo!()` 在 Task 2 Step 3，是用户选定的 A 方案（RED 阶段），Step 5 已给出完整替换实现并明确要求替换。

**3. Type consistency**

- `ensure_direct_rules_value(v: &mut Value) -> bool` —— Task 2 定义，Task 3 Step 3 以 `Self::ensure_direct_rules_value(&mut v)` 调用，签名一致。
- `ensure_direct_rules_in_base() -> Result<()>` —— Task 3 定义与调用一致；`Result` 为文件顶部的 `anyhow::Result`（`use anyhow::{Context, Result}` 已存在）。
- `CONFIG_LOCK` —— 复用文件顶部既有 `static CONFIG_LOCK: Lazy<Mutex<()>>`，Task 3 未重复定义。
- `xray::CONF_DIR` —— 与 `read_base_json` 现有用法一致（`crate::core::paths::xray`）。
- `RuleDef` 字段名 `id` / `rule_type` / `targets` / `outbound` / `default_enabled` —— 与 Task 1 断言所用字段一致。
- `rule_def_to_json(def)` —— Task 2 调用既有方法（`&RuleDef` 参数），签名匹配。

**4. 风险提示**

- Task 3 Step 1 的测试在"已部署机器"上会走 `return` 跳过分支，等于未验证。这是刻意的：`read_base_json` 硬编码生产路径，无法注入临时目录。真正的迁移正确性由 Task 2 的纯函数单测 + Task 5 Step 6 的人工验证共同保证。
- 未添加"写入字节级"的集成测试，因为没有可注入的配置目录。若后续认为必要，应先把 `read_base_json` 参数化，那属于独立重构，不在本计划范围。
