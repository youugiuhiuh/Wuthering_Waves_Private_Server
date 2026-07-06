use crate::core::paths::xray;
use anyhow::{Context, Result};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct RuleDef {
    pub id: &'static str,
    pub rule_type: &'static str,
    pub targets: &'static [&'static str],
    pub outbound: &'static str,
    pub default_enabled: bool,
}

pub static ROUTING_RULES: &[RuleDef] = &[
    RuleDef {
        id: "private_ip",
        rule_type: "ip",
        targets: &["geoip:private"],
        outbound: "blocked",
        default_enabled: true,
    },
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
    RuleDef {
        id: "private_domain",
        rule_type: "domain",
        targets: &["geosite:private"],
        outbound: "blocked",
        default_enabled: false,
    },
    RuleDef {
        id: "bt",
        rule_type: "protocol",
        targets: &["bittorrent"],
        outbound: "blocked",
        default_enabled: false,
    },
    RuleDef {
        id: "ads",
        rule_type: "domain",
        targets: &["geosite:category-ads-all"],
        outbound: "blocked",
        default_enabled: false,
    },
    RuleDef {
        id: "openai",
        rule_type: "domain",
        targets: &["geosite:openai"],
        outbound: "direct",
        default_enabled: false,
    },
];

pub struct RoutingManager;

impl RoutingManager {
    async fn read_rules() -> Result<Vec<Value>> {
        let (v, _) = Self::read_base_json().await?;
        Ok(v["routing"]["rules"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    async fn read_base_json() -> Result<(Value, String)> {
        let base_path = format!("{}/00_base.json", xray::CONF_DIR);
        let content = tokio::fs::read_to_string(&base_path)
            .await
            .context("读取 00_base.json 失败")?;
        let v: Value = serde_json::from_str(&content).context("解析 00_base.json 失败")?;
        Ok((v, base_path))
    }

    async fn write_rules(rules: &[Value]) -> Result<()> {
        let (mut v, base_path) = Self::read_base_json().await?;
        v["routing"]["rules"] = Value::Array(rules.to_vec());
        let new_content = serde_json::to_string_pretty(&v).context("序列化配置失败")?;
        tokio::fs::write(&base_path, new_content)
            .await
            .context("写入 00_base.json 失败")?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await
    }

    pub(crate) fn rule_def_to_json(rule: &RuleDef) -> Value {
        let mut obj = json!({"type": "field", "ruleTag": rule.id, "outboundTag": rule.outbound});
        match rule.rule_type {
            "ip" => {
                obj["ip"] = Value::Array(
                    rule.targets
                        .iter()
                        .map(|s| Value::String(s.to_string()))
                        .collect(),
                );
            }
            "domain" => {
                obj["domain"] = Value::Array(
                    rule.targets
                        .iter()
                        .map(|s| Value::String(s.to_string()))
                        .collect(),
                );
            }
            "protocol" => {
                obj["protocol"] = Value::Array(
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

    pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>> {
        let rules = Self::read_rules().await?;
        let enabled_ids: Vec<&str> = rules
            .iter()
            .filter_map(|r| r.get("ruleTag").and_then(|t| t.as_str()))
            .collect();
        Ok(ROUTING_RULES
            .iter()
            .map(|def| {
                let enabled = enabled_ids.contains(&def.id);
                (def, enabled)
            })
            .collect())
    }

    pub async fn toggle(rule_id: &str) -> Result<bool> {
        let rule_def = ROUTING_RULES
            .iter()
            .find(|r| r.id == rule_id)
            .ok_or_else(|| anyhow::anyhow!("未知规则: {}", rule_id))?;

        let mut rules = Self::read_rules().await?;
        let pos = rules
            .iter()
            .position(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some(rule_id));

        let now_enabled = if let Some(idx) = pos {
            rules.remove(idx);
            false
        } else {
            rules.push(Self::rule_def_to_json(rule_def));
            true
        };

        Self::write_rules(&rules).await?;
        Ok(now_enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_def_constants_count() {
        assert_eq!(ROUTING_RULES.len(), 7);
    }

    #[test]
    fn test_rule_def_has_private_ip_default() {
        let private = ROUTING_RULES.iter().find(|r| r.id == "private_ip").unwrap();
        assert!(private.default_enabled);
    }

    #[test]
    fn test_rule_def_to_json_ip() {
        let rule = RuleDef {
            id: "test",
            rule_type: "ip",
            targets: &["geoip:cn"],
            outbound: "blocked",
            default_enabled: false,
        };
        let json = RoutingManager::rule_def_to_json(&rule);
        assert_eq!(json["ruleTag"], "test");
        assert_eq!(json["type"], "field");
        assert_eq!(json["ip"][0], "geoip:cn");
        assert_eq!(json["outboundTag"], "blocked");
    }

    #[test]
    fn test_rule_def_to_json_domain() {
        let rule = RuleDef {
            id: "test_d",
            rule_type: "domain",
            targets: &["geosite:cn"],
            outbound: "direct",
            default_enabled: false,
        };
        let json = RoutingManager::rule_def_to_json(&rule);
        assert_eq!(json["domain"][0], "geosite:cn");
        assert_eq!(json["outboundTag"], "direct");
    }

    #[test]
    fn test_rule_def_to_json_protocol() {
        let rule = RuleDef {
            id: "test_p",
            rule_type: "protocol",
            targets: &["bittorrent"],
            outbound: "blocked",
            default_enabled: false,
        };
        let json = RoutingManager::rule_def_to_json(&rule);
        assert_eq!(json["protocol"][0], "bittorrent");
    }
}
