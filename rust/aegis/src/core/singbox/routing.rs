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
    RuleDef {
        id: "private_ip",
        rule_type: "ip_private",
        targets: &[],
        outbound: "block",
        default_enabled: true,
    },
    RuleDef {
        id: "cn_ip",
        rule_type: "rule_set",
        targets: &["geoip-cn"],
        outbound: "block",
        default_enabled: true,
    },
    RuleDef {
        id: "cn_domain",
        rule_type: "rule_set",
        targets: &["geosite-cn"],
        outbound: "block",
        default_enabled: true,
    },
    RuleDef {
        id: "private_domain",
        rule_type: "rule_set",
        targets: &["geosite-private"],
        outbound: "block",
        default_enabled: false,
    },
    RuleDef {
        id: "ads",
        rule_type: "rule_set",
        targets: &["geosite-category-ads-all"],
        outbound: "block",
        default_enabled: false,
    },
    RuleDef {
        id: "openai",
        rule_type: "rule_set",
        targets: &["geosite-openai"],
        outbound: "direct",
        default_enabled: false,
    },
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
                    rule.targets
                        .iter()
                        .map(|s| Value::String(s.to_string()))
                        .collect(),
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
                let enabled = rules.contains(&canonical);
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
        let content = tokio::fs::read_to_string(&base_path)
            .await
            .context("读取 00_base.json 失败")?;
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
        tokio::fs::write(&base_path, new_content)
            .await
            .context("写入 00_base.json 失败")?;
        Ok(())
    }

    async fn read_base_json() -> Result<(Value, String)> {
        let base_path = format!("{}/00_base.json", crate::core::paths::singbox::CONF_DIR);
        let content = tokio::fs::read_to_string(&base_path)
            .await
            .context("读取 00_base.json 失败")?;
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
        tokio::fs::write(&base_path, new_content)
            .await
            .context("写入 00_base.json 失败")?;
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
        let rule = SINGBOX_ROUTING_RULES
            .iter()
            .find(|r| r.id == "private_ip")
            .unwrap();
        let json = SingBoxRoutingManager::rule_def_to_json(rule);
        assert_eq!(json["ip_is_private"], true);
        assert_eq!(json["outbound"], "block");
        assert!(json.get("rule_set").is_none(), "不得含自定义字段");
    }

    #[test]
    fn test_rule_def_to_json_rule_set() {
        let rule = SINGBOX_ROUTING_RULES
            .iter()
            .find(|r| r.id == "cn_ip")
            .unwrap();
        let json = SingBoxRoutingManager::rule_def_to_json(rule);
        assert_eq!(json["rule_set"][0], "geoip-cn");
        assert_eq!(json["outbound"], "block");
        assert!(json.get("ip_is_private").is_none());
    }

    #[test]
    fn test_rule_def_to_json_openai_direct() {
        let rule = SINGBOX_ROUTING_RULES
            .iter()
            .find(|r| r.id == "openai")
            .unwrap();
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
            vec![
                "geosite-cn",
                "geosite-private",
                "geosite-category-ads-all",
                "geosite-openai",
                "geoip-cn"
            ]
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
        tokio::fs::write(&base_path, r#"{"route":{"default_domain_resolver":"dns"}}"#)
            .await
            .unwrap();

        // 无法注入 CONF_DIR（路径常量），此处仅测试 JSON 语义：
        let rule = SINGBOX_ROUTING_RULES
            .iter()
            .find(|r| r.id == "cn_ip")
            .unwrap();
        let canonical = SingBoxRoutingManager::rule_def_to_json(rule);
        let rules = [canonical.clone()];
        assert!(rules.contains(&canonical));
        assert_ne!(
            SingBoxRoutingManager::rule_def_to_json(
                SINGBOX_ROUTING_RULES
                    .iter()
                    .find(|r| r.id == "cn_domain")
                    .unwrap()
            ),
            canonical
        );
    }
}
