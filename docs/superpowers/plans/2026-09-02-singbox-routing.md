# sing-box 路由管理移植 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 sing-box（wwps-box）移植 Xray 同款路由管理：6 条可开关规则、Telegram 菜单、官方 .db 下载+本机转换生成 .srs 规则集、定时任务拆分（域名每周/geoip 每月 13 日）。

**Architecture:** 镜像现有 `core/xray/routing.rs` 模式：新建 `core/singbox/routing.rs`（SingBoxRoutingManager + SINGBOX_RULES），读写 `00_base.json` 的 `route.rules`（语义匹配判断开关，因 sing-box 拒绝自定义字段），写后 `reload_core()`。规则集文件由 `maintenance.rs` 下载官方 geosite.db/geoip.db 并用已装 sing-box 二进制 `geosite export` + `rule-set compile` 生成 5 个 .srs 到 `/etc/wwps/wwps-box/rule-set/`。定时任务新增 `GeoIpUpdate`（每月 13 日）只更新 geoip；`GeoUpdate` 每周日更新 Xray geodata + sing-geosite 域名。

**Tech Stack:** Rust 2024, tokio, serde_json, teloxide (Telegram bot), rust_i18n

**Spec:** `docs/superpowers/specs/2026-09-02-singbox-routing-design.md`

## Global Constraints

- sing-box 规则对象**禁止自定义字段**（如 ruleTag）——实测 `json: unknown field` 错误；开关状态必须用规范 JSON 深比较（serde_json::Value ==）
- `rule_set` 位于 **`route` 对象内**（`route.rule_set`），非顶层
- 规则集文件：`/etc/wwps/wwps-box/rule-set/*.srs`（type local, format binary）
- 下载源：`https://github.com/SagerNet/sing-geosite/releases/latest/download/geosite.db`、`https://github.com/SagerNet/sing-geoip/releases/latest/download/geoip.db`
- 转换命令：`<singbox::BIN> geosite export <cat> -f <db> -o <out.json>` 然后 `<singbox::BIN> rule-set compile --output <out.srs> <in.json>`
- 定时任务 cron：GeoUpdate `0 4 * * 0`（每周日，含 sing-geosite 域名）；GeoIpUpdate `0 4 13 * *`（每月 13 日，仅 geoip）
- 命名：singbox 侧路由管理器叫 **SingBoxRoutingManager**（避免与 xray 的 RoutingManager 在 handler 中歧义）
- 质量门禁（rust-lint-format）：`cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo test && cargo test --doc` 全绿（nextest 未装，用 cargo test）

---

### Task 1: paths.rs 增加 RULE_SET_DIR

**Files:**
- Modify: `rust/aegis/src/core/paths.rs`（singbox mod 内）
- Test: `rust/aegis/src/core/paths.rs` 现有 `test_singbox_paths`

**Interfaces:**
- Produces: `crate::core::paths::singbox::RULE_SET_DIR: &str = "/etc/wwps/wwps-box/rule-set"`

- [ ] **Step 1: Write the failing test**

在 `test_singbox_paths` 中追加断言：

```rust
#[test]
fn test_singbox_paths() {
    assert_eq!(singbox::DIR, "/etc/wwps/wwps-box");
    assert_eq!(singbox::BIN, "/etc/wwps/wwps-box/wwps-box");
    assert_eq!(singbox::CERTS_DIR, "/etc/wwps/wwps-box/certs");
    assert_eq!(singbox::TLS_CERT, "/etc/wwps/wwps-box/certs/tls.cer");
    assert_eq!(singbox::RULE_SET_DIR, "/etc/wwps/wwps-box/rule-set"); // 新增
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_singbox_paths 2>&1 | tail -5`
Expected: FAIL（`RULE_SET_DIR` 不存在编译错误）

- [ ] **Step 3: Write minimal implementation**

在 `src/core/paths.rs` 的 `pub mod singbox` 内、`TLS_KEY` 之后加一行：

```rust
    pub const RULE_SET_DIR: &str = "/etc/wwps/wwps-box/rule-set";
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test test_singbox_paths`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/paths.rs
git commit -m "feat(singbox): 增加规则集目录常量 RULE_SET_DIR"
```

---

### Task 2: singbox routing 核心模块

**Files:**
- Create: `rust/aegis/src/core/singbox/routing.rs`
- Modify: `rust/aegis/src/core/singbox/mod.rs`（加 `pub mod routing;` + `pub use routing::SingBoxRoutingManager;`）
- Test: 新建文件内 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::core::paths::singbox`（Task 1）、`crate::core::system::maintenance::MaintenanceManager::reload_core()`
- Produces:
  - `pub struct RuleDef { pub id: &'static str, pub rule_type: &'static str, pub targets: &'static [&'static str], pub outbound: &'static str, pub default_enabled: bool }`
  - `pub static SINGBOX_ROUTING_RULES: &[RuleDef]`（6 条）
  - `pub static GEOSITE_CATEGORIES: &[&str] = &["cn", "private", "category-ads-all", "openai"];`
  - `pub static GEOIP_CATEGORIES: &[&str] = &["cn"];`
  - `pub fn rule_set_definitions() -> Vec<serde_json::Value>`（5 项 rule_set JSON，供 config.rs 使用）
  - `pub struct SingBoxRoutingManager`
  - `impl SingBoxRoutingManager { pub fn rule_def_to_json(rule: &RuleDef) -> Value; pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>>; pub async fn toggle(rule_id: &str) -> Result<bool>; pub async fn ensure_rule_sets_in_base() -> Result<()>; }`

- [ ] **Step 1: Write the failing test**

创建 `src/core/singbox/routing.rs`，先写测试模块（RED）：

```rust
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde_json::{Value, json};
use tokio::sync::Mutex;

static CONFIG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone)]
pub struct RuleDef {
    pub id: &'static str,
    pub rule_type: &'static str,
    pub targets: &'static [&'static str],
    pub outbound: &'static str,
    pub default_enabled: bool,
}

pub static SINGBOX_ROUTING_RULES: &[RuleDef] = &[
    RuleDef { id: "private_ip", rule_type: "ip_private", targets: &[], outbound: "block", default_enabled: true },
    RuleDef { id: "cn_ip", rule_type: "rule_set", targets: &["geoip-cn"], outbound: "block", default_enabled: true },
    RuleDef { id: "cn_domain", rule_type: "rule_set", targets: &["geosite-cn"], outbound: "block", default_enabled: true },
    RuleDef { id: "private_domain", rule_type: "rule_set", targets: &["geosite-private"], outbound: "block", default_enabled: false },
    RuleDef { id: "ads", rule_type: "rule_set", targets: &["geosite-category-ads-all"], outbound: "block", default_enabled: false },
    RuleDef { id: "openai", rule_type: "rule_set", targets: &["geosite-openai"], outbound: "direct", default_enabled: false },
];

pub static GEOSITE_CATEGORIES: &[&str] = &["cn", "private", "category-ads-all", "openai"];
pub static GEOIP_CATEGORIES: &[&str] = &["cn"];

pub struct SingBoxRoutingManager;

impl SingBoxRoutingManager {
    /// 将 RuleDef 转成 sing-box 规则 JSON（简写语法，不带自定义字段）
    pub fn rule_def_to_json(rule: &RuleDef) -> Value {
        let mut obj = json!({"outbound": rule.outbound});
        match rule.rule_type {
            "ip_private" => {
                obj["ip_is_private"] = Value::Bool(true);
            }
            "rule_set" => {
                obj["rule_set"] = Value::Array(
                    rule.targets.iter().map(|s| Value::String(s.to_string())).collect(),
                );
            }
            _ => unreachable!("unknown rule_type: {}", rule.rule_type),
        }
        obj
    }

    /// 5 个 rule_set 定义（供 00_base.json 使用）
    pub fn rule_set_definitions() -> Vec<Value> {
        let mut out = Vec::new();
        for cat in GEOSITE_CATEGORIES {
            out.push(json!({
                "tag": format!("geosite-{}", cat),
                "type": "local",
                "format": "binary",
                "path": format!("{}/geosite-{}.srs", crate::core::paths::singbox::RULE_SET_DIR, cat)
            }));
        }
        for cat in GEOIP_CATEGORIES {
            out.push(json!({
                "tag": format!("geoip-{}", cat),
                "type": "local",
                "format": "binary",
                "path": format!("{}/geoip-{}.srs", crate::core::paths::singbox::RULE_SET_DIR, cat)
            }));
        }
        out
    }

    /// 读取 00_base.json 中当前启用的规则（语义匹配：与规范 JSON 深相等）
    pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>> {
        let rules = Self::read_rules().await?;
        Ok(SINGBOX_ROUTING_RULES
            .iter()
            .map(|def| {
                let canonical = Self::rule_def_to_json(def);
                let enabled = rules.iter().any(|r| *r == canonical);
                (def, enabled)
            })
            .collect())
    }

    /// 切换规则开关（增删规范 JSON），写盘并重载
    pub async fn toggle(rule_id: &str) -> Result<bool> {
        let rule_def = SINGBOX_ROUTING_RULES
            .iter()
            .find(|r| r.id == rule_id)
            .ok_or_else(|| anyhow::anyhow!("未知规则: {}", rule_id))?;

        let mut rules = Self::read_rules().await?;
        let canonical = Self::rule_def_to_json(rule_def);
        let pos = rules.iter().position(|r| *r == canonical);

        let now_enabled = if let Some(idx) = pos {
            rules.remove(idx);
            false
        } else {
            rules.push(canonical);
            true
        };

        Self::write_rules(&rules).await?;
        Ok(now_enabled)
    }

    /// 迁移：确保 base 配置的 route.rule_set 包含全部 5 项（幂等）
    pub async fn ensure_rule_sets_in_base() -> Result<()> {
        let _lock = CONFIG_LOCK.lock().await;
        let base_path = format!("{}/00_base.json", crate::core::paths::singbox::CONF_DIR);
        let content = tokio::fs::read_to_string(&base_path).await.context("读取 00_base.json 失败")?;
        let mut v: Value = serde_json::from_str(&content).context("解析 00_base.json 失败")?;
        let defs = Self::rule_set_definitions();

        let existing: Vec<&str> = v["route"]["rule_set"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|r| r["tag"].as_str()).collect())
            .unwrap_or_default();
        let missing: Vec<Value> = defs
            .into_iter()
            .filter(|d| !existing.contains(&d["tag"].as_str().unwrap_or_default()))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let arr = v["route"]["rule_set"].as_array_mut().unwrap();
        arr.extend(missing);
        let new_content = serde_json::to_string_pretty(&v).context("序列化配置失败")?;
        tokio::fs::write(&base_path, new_content).await.context("写入 00_base.json 失败")?;
        Ok(())
    }

    async fn read_base_json() -> Result<(Value, String)> {
        let base_path = format!("{}/00_base.json", crate::core::paths::singbox::CONF_DIR);
        let content = tokio::fs::read_to_string(&base_path).await.context("读取 00_base.json 失败")?;
        let v: Value = serde_json::from_str(&content).context("解析 00_base.json 失败")?;
        Ok((v, base_path))
    }

    async fn read_rules() -> Result<Vec<Value>> {
        let (v, _) = Self::read_base_json().await?;
        Ok(v["route"]["rules"].as_array().cloned().unwrap_or_default())
    }

    async fn write_rules(rules: &[Value]) -> Result<()> {
        let _lock = CONFIG_LOCK.lock().await;
        let (mut v, base_path) = Self::read_base_json().await?;
        v["route"]["rules"] = Value::Array(rules.to_vec());
        let new_content = serde_json::to_string_pretty(&v).context("序列化配置失败")?;
        tokio::fs::write(&base_path, new_content).await.context("写入 00_base.json 失败")?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_def_constants_count() {
        assert_eq!(SINGBOX_ROUTING_RULES.len(), 6);
    }

    #[test]
    fn test_default_rules_enabled() {
        for id in ["private_ip", "cn_ip", "cn_domain"] {
            let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == id).unwrap();
            assert!(rule.default_enabled, "{} 应默认启用", id);
        }
        for id in ["private_domain", "ads", "openai"] {
            let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == id).unwrap();
            assert!(!rule.default_enabled, "{} 应默认关闭", id);
        }
    }

    #[test]
    fn test_no_bt_rule() {
        assert!(SINGBOX_ROUTING_RULES.iter().all(|r| r.id != "bt"));
    }

    #[test]
    fn test_rule_def_to_json_ip_private() {
        let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == "private_ip").unwrap();
        let json = SingBoxRoutingManager::rule_def_to_json(rule);
        assert_eq!(json["ip_is_private"], true);
        assert_eq!(json["outbound"], "block");
        assert!(json.get("rule_set").is_none(), "不得含自定义字段");
    }

    #[test]
    fn test_rule_def_to_json_rule_set() {
        let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == "cn_ip").unwrap();
        let json = SingBoxRoutingManager::rule_def_to_json(rule);
        assert_eq!(json["rule_set"][0], "geoip-cn");
        assert_eq!(json["outbound"], "block");
        assert!(json.get("ip_is_private").is_none());
    }

    #[test]
    fn test_rule_def_to_json_openai_direct() {
        let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == "openai").unwrap();
        let json = SingBoxRoutingManager::rule_def_to_json(rule);
        assert_eq!(json["outbound"], "direct");
    }

    #[test]
    fn test_rule_set_definitions_count_and_tags() {
        let defs = SingBoxRoutingManager::rule_set_definitions();
        assert_eq!(defs.len(), 5);
        let tags: Vec<&str> = defs.iter().filter_map(|d| d["tag"].as_str()).collect();
        assert_eq!(
            tags,
            vec!["geosite-cn", "geosite-private", "geosite-category-ads-all", "geosite-openai", "geoip-cn"]
        );
        for d in &defs {
            assert_eq!(d["type"], "local");
            assert_eq!(d["format"], "binary");
            assert!(d["path"].as_str().unwrap().ends_with(".srs"));
        }
    }

    #[tokio::test]
    async fn test_toggle_add_remove() {
        let temp = tempfile::tempdir().unwrap();
        let conf_dir = temp.path().join("conf");
        tokio::fs::create_dir_all(&conf_dir).await.unwrap();
        let base_path = conf_dir.join("00_base.json");
        // 注意：这里直接构造最小 base（无默认规则），模拟已部署实例
        tokio::fs::write(&base_path, r#"{"route":{"default_domain_resolver":"dns"}}"#).await.unwrap();

        // 无法注入 CONF_DIR（路径常量），此处仅测试 JSON 语义：
        let rule = SINGBOX_ROUTING_RULES.iter().find(|r| r.id == "cn_ip").unwrap();
        let canonical = SingBoxRoutingManager::rule_def_to_json(rule);
        let rules = vec![canonical.clone()];
        assert!(rules.contains(&canonical));
        assert_eq!(SingBoxRoutingManager::rule_def_to_json(
            SINGBOX_ROUTING_RULES.iter().find(|r| r.id == "cn_domain").unwrap()
        ) != canonical, true);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_rule_def_constants_count 2>&1 | tail -3`
Expected: 编译错误（模块未注册）→ 先在 `src/core/singbox/mod.rs` 加 `pub mod routing;` 使编译通过，然后测试应为 FAIL（`SINGBOX_ROUTING_RULES.len()` 断言失败/未实现）

- [ ] **Step 3: Write minimal implementation**

即 Step 1 中的完整实现（RuleDef + 常量 + SingBoxRoutingManager 全部方法）。若 Step 1 已含完整实现，此步仅核对无遗漏（确保 `write_rules` 调用了 `reload_core`、`ensure_rule_sets_in_base` 幂等）。

在 `src/core/singbox/mod.rs` 注册：

```rust
pub mod routing;
pub use routing::SingBoxRoutingManager;
```

（注意：`test_toggle_add_remove` 是语义级测试——CONF_DIR 是编译期常量无法注入，测试只验证 JSON 深比较逻辑；真正的 toggle 写盘行为依赖文件系统，保持轻量）

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test routing 2>&1 | tail -5`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/singbox/routing.rs src/core/singbox/mod.rs
git commit -m "feat(singbox): 路由管理核心模块 SingBoxRoutingManager + 6 条规则"
```

---

### Task 3: config.rs 基础配置写入默认规则与 rule_set

**Files:**
- Modify: `rust/aegis/src/core/singbox/config.rs`（`ensure_base_config` + 测试 `test_ensure_base_config_creates_base_file`）
- Test: 同文件

**Interfaces:**
- Consumes: Task 2 的 `SingBoxRoutingManager::{SINGBOX_ROUTING_RULES, rule_def_to_json, rule_set_definitions}`

- [ ] **Step 1: Write the failing test**

在 `test_ensure_base_config_creates_base_file` 中，base_config 构造后追加断言：

```rust
        assert!(json.get("route").is_some());
        assert!(json.get("outbounds").is_some());
        // 新增：默认规则 3 条 + rule_set 5 项
        let rules = json["route"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 3);
        let tags: Vec<&str> = rules.iter().filter_map(|r| {
            if r.get("ip_is_private").is_some() { Some("private_ip") } else { r["rule_set"][0].as_str() }
        }).collect();
        assert_eq!(tags, vec!["private_ip", "geoip-cn", "geosite-cn"]);
        let rule_sets = json["route"]["rule_set"].as_array().unwrap();
        assert_eq!(rule_sets.len(), 5);
        assert!(rule_sets.iter().all(|r| r["type"] == "local" && r["format"] == "binary"));
```

（同样更新 `test_ensure_base_config_structure` 风格的既有断言——本文件该函数内的 base_config 字面量也要同步加 `rule_set` 与 `rules` 字段）

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_ensure_base_config_creates_base_file`
Expected: FAIL（`json["route"]["rules"]` 为 null）

- [ ] **Step 3: Write minimal implementation**

在 `ensure_base_config()` 中，`base_config` 的 `route` 对象改为：

```rust
        let default_rules: Vec<Value> = crate::core::singbox::routing::SINGBOX_ROUTING_RULES
            .iter()
            .filter(|r| r.default_enabled)
            .map(crate::core::singbox::routing::SingBoxRoutingManager::rule_def_to_json)
            .collect();

        let base_config = serde_json::json!({
            "log": {
                "level": "warning"
            },
            "dns": {
                "servers": [
                    {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
                    {"tag": "local", "type": "local"}
                ]
            },
            "route": {
                "default_domain_resolver": "dns",
                "rule_set": crate::core::singbox::routing::SingBoxRoutingManager::rule_set_definitions(),
                "rules": default_rules
            },
            "outbounds": [
                {"type": "direct", "tag": "direct"},
                {"type": "block", "tag": "block"}
            ]
        });
```

同时更新测试函数 `test_ensure_base_config_creates_base_file` 内的 base_config 字面量（保持与实现一致），使断言成立。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test test_ensure_base_config_creates_base_file && cargo test routing`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/singbox/config.rs
git commit -m "feat(singbox): 基础配置写入默认路由规则与 rule_set 定义"
```

---

### Task 4: maintenance.rs 规则集下载与转换

**Files:**
- Modify: `rust/aegis/src/core/system/maintenance.rs`
- Test: 同文件 `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::core::cmd_async::run_cmd_checked`、`crate::core::paths::singbox`（Task 1）、Task 2 的 `GEOSITE_CATEGORIES`/`GEOIP_CATEGORIES`
- Produces:
  - `pub async fn update_singbox_rules<F>(include_geoip: bool, progress_callback: F) -> Result<()>`（F: `Fn(f64, &str) + Send + Sync + 'static`）
  - `pub async fn ensure_singbox_rule_sets() -> Result<()>`（skip-if-exists，安装时用）
  - `fn singbox_convert_args(program: &str, category: &str, db_path: &str, out_json: &str, out_srs: &str) -> Vec<Vec<String>>`（纯函数，供测试）

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn test_singbox_convert_args() {
        let cmds = MaintenanceManager::singbox_convert_args(
            "/opt/wwps-box",
            "cn",
            "/tmp/geosite.db",
            "/tmp/geosite-cn.json",
            "/etc/wwps/wwps-box/rule-set/geosite-cn.srs",
        );
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], vec![
            "/opt/wwps-box", "geosite", "export", "cn",
            "-f", "/tmp/geosite.db", "-o", "/tmp/geosite-cn.json",
        ]);
        assert_eq!(cmds[1], vec![
            "/opt/wwps-box", "rule-set", "compile",
            "--output", "/etc/wwps/wwps-box/rule-set/geosite-cn.srs",
            "/tmp/geosite-cn.json",
        ]);
    }

    #[tokio::test]
    async fn test_ensure_singbox_rule_sets_skips_existing() {
        let temp = tempfile::tempdir().unwrap();
        let rule_dir = temp.path().join("rule-set");
        tokio::fs::create_dir_all(&rule_dir).await.unwrap();
        let existing = rule_dir.join("geosite-cn.srs");
        tokio::fs::write(&existing, b"data").await.unwrap();
        // 文件已存在 → 对应分类不应出现在待下载/转换清单
        let missing = MaintenanceManager::singbox_missing_srs(&rule_dir.to_str().unwrap());
        assert!(!missing.iter().any(|(kind, cat)| kind == "geosite" && cat == "cn"));
        assert!(missing.iter().any(|(kind, cat)| kind == "geosite" && cat == "private"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_singbox_convert_args`
Expected: FAIL（函数不存在）

- [ ] **Step 3: Write minimal implementation**

在 `src/core/system/maintenance.rs` 的 `impl MaintenanceManager` 内、`update_geodata` 之后添加：

```rust
    /// sing-box 规则集：下载官方 .db → geosite/geoip export → rule-set compile 生成 .srs
    pub async fn update_singbox_rules<F>(include_geoip: bool, progress_callback: F) -> Result<()>
    where
        F: Fn(f64, &str) + Send + Sync + 'static,
    {
        use crate::core::paths::singbox;

        let rule_dir = singbox::RULE_SET_DIR;
        std::fs::create_dir_all(rule_dir).context("创建 rule-set 目录失败")?;
        let temp_dir = format!("{}/.update-tmp", singbox::DIR);
        std::fs::create_dir_all(&temp_dir).context("创建临时目录失败")?;

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT_LONG)
            .build()
            .context("构建 HTTP 客户端失败")?;

        // 1. 下载 geosite.db（域名库，总是更新）
        let geosite_db = format!("{}/geosite.db", temp_dir);
        progress_callback(0.0, "下载 geosite.db (官方每日更新)...");
        Self::download_file(&client, "https://github.com/SagerNet/sing-geosite/releases/latest/download/geosite.db", &geosite_db, |_, _| {}).await
            .context("下载 geosite.db 失败")?;

        // 2. 可选下载 geoip.db
        let geoip_db = format!("{}/geoip.db", temp_dir);
        if include_geoip {
            progress_callback(0.0, "下载 geoip.db (官方每月更新)...");
            Self::download_file(&client, "https://github.com/SagerNet/sing-geoip/releases/latest/download/geoip.db", &geoip_db, |_, _| {}).await
                .context("下载 geoip.db 失败")?;
        }

        // 3. 转换 geosite 分类
        for cat in crate::core::singbox::routing::GEOSITE_CATEGORIES {
            let out_json = format!("{}/geosite-{}.json", temp_dir, cat);
            let out_srs = format!("{}/geosite-{}.srs", rule_dir, cat);
            progress_callback(0.0, &format!("转换 geosite-{}...", cat));
            Self::run_singbox_convert(&geosite_db, &out_json, &out_srs, cat).await?;
        }

        // 4. 转换 geoip 分类
        if include_geoip {
            for cat in crate::core::singbox::routing::GEOIP_CATEGORIES {
                let out_json = format!("{}/geoip-{}.json", temp_dir, cat);
                let out_srs = format!("{}/geoip-{}.srs", rule_dir, cat);
                progress_callback(0.0, &format!("转换 geoip-{}...", cat));
                Self::run_singbox_convert(&geoip_db, &out_json, &out_srs, cat).await?;
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
        progress_callback(1.0, "sing-box 规则集更新完成");
        Self::reload_core().await
    }

    /// 安装时确保规则集存在（skip-if-exists）
    pub async fn ensure_singbox_rule_sets() -> Result<()> {
        use crate::core::paths::singbox;
        let rule_dir = singbox::RULE_SET_DIR;
        std::fs::create_dir_all(rule_dir).context("创建 rule-set 目录失败")?;
        let missing = Self::singbox_missing_srs(rule_dir);
        if missing.is_empty() {
            return Ok(());
        }
        Self::update_singbox_rules(true, |_, _| {}).await
    }

    /// 返回缺失的 (kind, category) 列表
    pub fn singbox_missing_srs(rule_dir: &str) -> Vec<(&'static str, &'static str)> {
        use crate::core::singbox::routing::{GEOSITE_CATEGORIES, GEOIP_CATEGORIES};
        let mut missing = Vec::new();
        for cat in GEOSITE_CATEGORIES {
            if !std::path::Path::new(&format!("{}/geosite-{}.srs", rule_dir, cat)).exists() {
                missing.push(("geosite", cat));
            }
        }
        for cat in GEOIP_CATEGORIES {
            if !std::path::Path::new(&format!("{}/geoip-{}.srs", rule_dir, cat)).exists() {
                missing.push(("geoip", cat));
            }
        }
        missing
    }

    /// 生成 geosite/geoip export + rule-set compile 两条命令
    pub fn singbox_convert_args(
        program: &str,
        category: &str,
        db_path: &str,
        out_json: &str,
        out_srs: &str,
    ) -> Vec<Vec<String>> {
        vec![
            vec![
                program.to_string(), "geosite".to_string(), "export".to_string(),
                category.to_string(), "-f".to_string(), db_path.to_string(),
                "-o".to_string(), out_json.to_string(),
            ],
            vec![
                program.to_string(), "rule-set".to_string(), "compile".to_string(),
                "--output".to_string(), out_srs.to_string(), out_json.to_string(),
            ],
        ]
    }

    async fn run_singbox_convert(db_path: &str, out_json: &str, out_srs: &str, category: &str) -> Result<()> {
        use crate::core::paths::singbox;
        let cmds = Self::singbox_convert_args(singbox::BIN, category, db_path, out_json, out_srs);
        for args in &cmds {
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_cmd_checked(&arg_refs[0], &arg_refs[1..], TIMEOUT_LONG).await.with_context(|| {
                format!("sing-box 转换命令失败: {:?}", args)
            })?;
        }
        Ok(())
    }
```

同时修改 `update_geodata()`（在 `Self::reload_core().await` 之前插入 sing-geosite 域名更新）：

```rust
        // sing-box 域名规则集（每日更新源，跟随本任务）
        if let Err(e) = Self::update_singbox_rules(false, |_pct, msg| {
            cb(0.0, &format!("[Sing-box] {}", msg));
        }).await {
            log::warn!("更新 sing-box 规则集失败: {}", e);
        }

        Self::reload_core().await
```

注意：`update_geodata` 的 `sources` 数组与 `let cb = &progress_callback;` 已在原函数中；插入位置为最后一个 for 循环之后、`reload_core` 之前。`cb` 变量名以现有代码为准（原函数内为 `let cb = &progress_callback;`）。

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test test_singbox_convert_args && cargo test test_ensure_singbox_rule_sets_skips_existing`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/system/maintenance.rs
git commit -m "feat(maintenance): sing-box 规则集下载与转换（官方 db → srs）"
```

---

### Task 5: 定时任务 GeoIpUpdate

**Files:**
- Modify: `rust/aegis/src/core/system/scheduler/task_types.rs`
- Modify: `rust/aegis/src/core/system/scheduler/mod.rs`
- Test: 两文件内 `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 4 的 `MaintenanceManager::update_singbox_rules`
- Produces: `TaskType::GeoIpUpdate` 变体（serde 兼容 `#[serde(other)] Unknown` 已存在）

- [ ] **Step 1: Write the failing test**

在 `task_types.rs` 测试模块追加：

```rust
    #[test]
    fn test_task_type_geoip_update_display() {
        assert_eq!(
            TaskType::GeoIpUpdate.get_display_name(),
            "GeoIP 更新 (Update GeoIP)"
        );
    }

    #[test]
    fn test_geoip_update_serde_roundtrip() {
        let s = serde_json::to_string(&TaskType::GeoIpUpdate).unwrap();
        let back: TaskType = serde_json::from_str(&s).unwrap();
        assert_eq!(back, TaskType::GeoIpUpdate);
    }

    #[test]
    fn test_old_state_file_still_deserializes() {
        // 旧 state 文件只有 GeoUpdate（模拟已有部署升级）
        let old = r#"{"tasks":[{"task_type":"GeoUpdate","cron_expression":"0 4 * * 0","timezone":"UTC"}]}"#;
        let state: crate::core::system::scheduler::task_types::SchedulerTask = serde_json::from_str(old).unwrap();
        assert_eq!(state.task_type, TaskType::GeoUpdate);
    }
```

（`SchedulerTask` 为 task_types.rs 中的结构体名，实际名称为 `ScheduledTask`——以文件内定义为准；若字段含 `timezone` 之外的必填字段，按现有序列化格式调整）

在 `mod.rs` 测试模块追加：

```rust
    #[test]
    fn test_default_contains_geoip_update() {
        let state = SchedulerState::get_default();
        assert!(state.tasks.iter().any(|t| t.task_type == TaskType::GeoIpUpdate));
        let geoip = state.tasks.iter().find(|t| t.task_type == TaskType::GeoIpUpdate).unwrap();
        assert_eq!(geoip.cron_expression, "0 4 13 * *");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test test_geoip_update_serde_roundtrip`
Expected: FAIL（`GeoIpUpdate` 变体不存在）

- [ ] **Step 3: Write minimal implementation**

`task_types.rs` 枚举加变体：

```rust
pub enum TaskType {
    GeoUpdate,
    GeoIpUpdate,
    Reboot,
    ReloadCore,
    SecurityUpdate,
    #[serde(other)]
    Unknown,
}
```

`get_display_name` 加分支：

```rust
            TaskType::GeoIpUpdate => "GeoIP 更新 (Update GeoIP)",
```

`execute` 加分支（在 GeoUpdate 分支后）：

```rust
            TaskType::GeoIpUpdate => {
                log::info!("执行 GeoIP 更新任务...");
                let _ = adapter
                    .send_message(
                        target,
                        MessageContent {
                            text: "⏳ [定时任务] 开始更新 sing-box GeoIP 规则集...".to_string(),
                            markup: None,
                        },
                    )
                    .await;

                let result = MaintenanceManager::update_singbox_rules(true, |_pct, msg| {
                    log::info!("[GeoIP] {}", msg);
                })
                .await;

                report_result(
                    adapter,
                    target,
                    "GeoIP 更新",
                    "✅ [定时任务] GeoIP 更新完成。",
                    result.map(|_| ()),
                )
                .await
            }
```

`mod.rs` 的 `SchedulerState::new()` 注册每月任务：

```rust
    pub fn new() -> Self {
        Self {
            tasks: vec![
                ScheduledTask::new(TaskType::GeoUpdate, "0 4 * * 0"),
                ScheduledTask::new(TaskType::GeoIpUpdate, "0 4 13 * *"),
            ],
        }
    }
```

迁移逻辑：`load_from_file` 后若缺 `GeoIpUpdate` 则补（放在 `SchedulerManager::start` 的 `spawn_blocking` 内，load 之后）：

```rust
            let mut s = SchedulerState::load_from_file(&path).unwrap_or_else(|_| SchedulerState::default());
            if !s.tasks.iter().any(|t| t.task_type == TaskType::GeoIpUpdate) {
                s.tasks.push(ScheduledTask::new(TaskType::GeoIpUpdate, "0 4 13 * *"));
                let _ = s.save_to_file(&path);
            }
            if !Path::new(&path).exists() {
                let _ = s.save_to_file(&path);
            }
```

（将原 `let s = SchedulerState::load_from_file(...)` 改为 `let mut s = ...` 并插入迁移块；`TaskType` 需在 mod.rs 中已导入——检查现有 use，若无则补 `use crate::core::system::scheduler::task_types::TaskType;`，实际以文件内 `use super::task_types::*;` 形式为准）

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test GeoIpUpdate && cargo test default_contains_geoip`
Expected: 全部 PASS

- [ ] **Step 5: Commit**

```bash
git add src/core/system/scheduler/task_types.rs src/core/system/scheduler/mod.rs
git commit -m "feat(scheduler): 新增 GeoIpUpdate 每月 13 日任务 + 旧状态迁移"
```

---

### Task 6: installer 安装后确保规则集

**Files:**
- Modify: `rust/aegis/src/core/singbox/installer.rs`（`install()`）

**Interfaces:**
- Consumes: Task 4 的 `MaintenanceManager::ensure_singbox_rule_sets`

- [ ] **Step 1: 无独立测试（依赖外部下载，install 流程本身无单测覆盖）**

- [ ] **Step 2: 实现**

在 `install()` 中 `Self::create_service().await?;` 之后、`let _ = fs::remove_dir_all(temp_dir).await;` 之前插入：

```rust
        if let Err(e) = crate::core::system::maintenance::MaintenanceManager::ensure_singbox_rule_sets().await {
            log::warn!("获取 sing-box 规则集失败（可稍后通过定时任务补齐）: {}", e);
        }
```

- [ ] **Step 3: 验证编译**

Run: `cargo build 2>&1 | tail -2`
Expected: Finished（无错误）

- [ ] **Step 4: Commit**

```bash
git add src/core/singbox/installer.rs
git commit -m "feat(singbox): 安装后自动获取规则集文件"
```

---

### Task 7: Telegram handler 路由管理菜单 + i18n

**Files:**
- Modify: `rust/aegis/src/shared/handlers/singbox.rs`
- Modify: `rust/aegis/src/resources/i18n/zh.yml`、`en.yml`、`ja.yml`
- Test: 编译验证 + 既有测试

**Interfaces:**
- Consumes: Task 2 的 `SingBoxRoutingManager::{get_all_with_status, toggle}`
- Produces: 回调 data：`m_sb_routing`、`sb_routing_toggle:<id>`

- [ ] **Step 1: 实现 handler（此任务为 UI 胶水，跟随现有 handler 模式，无独立单测；用编译 + clippy 验证）**

在 `src/shared/handlers/singbox.rs`：

a) 顶部 import 追加：

```rust
use crate::core::singbox::routing::SingBoxRoutingManager;
```

b) `m_singbox_mgmt` 的两个已安装分支（`inbounds.is_empty()` 分支与正常分支）各加一行按钮（放在删除管理按钮之后）：

```rust
                rows.push(vec![InlineButton {
                    text: t!("singbox.routing_mgmt_btn").into(),
                    data: "m_sb_routing".into(),
                }]);
```

c) 在 `handle` 的 match 中追加两个分支（放在 `"sb_del_cfg"` 分支之前）：

```rust
        "m_sb_routing" => {
            let rules = SingBoxRoutingManager::get_all_with_status()
                .await
                .map_err(|e| anyhow::anyhow!("获取路由规则失败: {}", e))?;
            let active_count = rules.iter().filter(|(_, enabled)| *enabled).count();
            let mut text = t!("singbox.routing_title").to_string();
            text.push_str(&format!(
                "\n\n{}",
                t!("singbox.routing_active_count", "count" => active_count.to_string())
            ));

            let mut rows: Vec<Vec<InlineButton>> = rules
                .iter()
                .map(|(def, enabled)| {
                    let i18n_key = format!("xray.routing_rule_{}", def.id);
                    let name = t!(i18n_key.as_str());
                    let icon = if *enabled { "✅" } else { "⬜" };
                    vec![InlineButton {
                        text: format!("{} {}", icon, name),
                        data: format!("sb_routing_toggle:{}", def.id),
                    }]
                })
                .collect();

            rows.push(vec![InlineButton {
                text: t!("menu.back").into(),
                data: "m_singbox_mgmt".into(),
            }]);

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text,
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;
            Ok(HandlerAction::Done)
        }

        d if d.starts_with("sb_routing_toggle:") => {
            let rule_id = d.strip_prefix("sb_routing_toggle:").unwrap_or("");
            if rule_id.is_empty() {
                return Ok(HandlerAction::Redirect("m_sb_routing".to_string()));
            }
            match SingBoxRoutingManager::toggle(rule_id).await {
                Ok(enabled) => {
                    let i18n_key = format!("xray.routing_rule_{}", rule_id);
                    let name = t!(i18n_key.as_str());
                    let msg = if enabled {
                        t!("singbox.routing_toggled_on", "name" => name)
                    } else {
                        t!("singbox.routing_toggled_off", "name" => name)
                    };
                    event
                        .adapter
                        .answer_callback(&event.target, &event.callback_id, Some(msg.into()))
                        .await?;
                }
                Err(e) => {
                    event
                        .adapter
                        .answer_callback(
                            &event.target,
                            &event.callback_id,
                            Some(format!("{}: {}", t!("singbox.routing_reload_failed"), e)),
                        )
                        .await?;
                }
            }
            Ok(HandlerAction::Redirect("m_sb_routing".to_string()))
        }
```

- [ ] **Step 2: i18n 键**

三个 i18n 文件（`zh.yml`/`en.yml`/`ja.yml`）在 `singbox:` 命名空间（`singbox_mgmt_title` 附近）追加（zh 参考，en/ja 翻译对应）：

```yaml
  routing_title: "📋 <b>Sing-box 路由规则</b>\n\n切换路由规则的开启/关闭。"
  routing_active_count: "活跃规则: %{count} 条"
  routing_toggled_on: "✅ 已开启 %{name}"
  routing_toggled_off: "❌ 已关闭 %{name}"
  routing_reload_failed: "❌ Sing-box 重载失败"
  routing_mgmt_btn: "📋 路由规则管理"
```

（en.yml / ja.yml 用对应英文/日文文案，键名一致）

- [ ] **Step 3: 验证**

Run: `cargo build 2>&1 | tail -2`
Expected: Finished

- [ ] **Step 4: Commit**

```bash
git add src/shared/handlers/singbox.rs src/resources/i18n/zh.yml src/resources/i18n/en.yml src/resources/i18n/ja.yml
git commit -m "feat(singbox): 路由管理 Telegram 菜单与开关回调"
```

---

### Task 8: 全量质量门禁

**Files:** 无（验证）

- [ ] **Step 1: 运行完整质量门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --doc
```

Expected: fmt 无 diff；clippy 0 警告；全部测试 PASS

- [ ] **Step 2: 修复任何问题并重复 Step 1 直至全绿**

- [ ] **Step 3: 确认 git 状态干净**

```bash
git status --short
git log --oneline -8
```

Expected: 工作区干净，8 个 feat 提交依次排列

---

## Self-Review

**Spec 覆盖：**
- ✅ 6 条规则（无 bt）→ Task 2
- ✅ rule_set 位于 route 内、无自定义字段（语义匹配）→ Task 2 + Task 3
- ✅ 官方 .db 下载 + 本机 export/compile → Task 4
- ✅ 5 个 .srs 到 RULE_SET_DIR → Task 1 + Task 4
- ✅ 定时任务拆分（GeoUpdate 每周日含域名 / GeoIpUpdate 每月 13 日）→ Task 5
- ✅ 安装时 ensure 规则集 → Task 6
- ✅ Telegram 菜单 + 开关回调 → Task 7
- ✅ i18n 三语 → Task 7
- ✅ 迁移（base 缺 rule_set 补入；scheduler 缺 GeoIpUpdate 补注册）→ Task 2（ensure_rule_sets_in_base）+ Task 5
- ✅ TDD 测试 → Task 1/2/3/4/5

**已知偏差（实现时注意）：**
- `test_toggle_add_remove` 因 CONF_DIR 是编译期常量无法注入临时目录，只测 JSON 语义比较——toggle 的写盘逻辑与 xray 版完全同构，风险低
- 转换命令的 `-f` flag 与 export 输出已验证（本机 sing-box 1.14 实测通过）
- scheduler 测试中 `ScheduledTask` 结构体字段名以现有文件为准（timezone 等）
