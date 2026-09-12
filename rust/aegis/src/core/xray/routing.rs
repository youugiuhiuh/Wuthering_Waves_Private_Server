use crate::core::paths::xray;
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
        default_enabled: true,
    },
    RuleDef {
        id: "cn_domain",
        rule_type: "domain",
        targets: &["geosite:cn"],
        outbound: "blocked",
        default_enabled: true,
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
        let _lock = CONFIG_LOCK.lock().await;
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

    pub async fn get_all_with_status() -> Result<Vec<(&'static RuleDef, bool)>> {
        // 首次进入菜单即完成存量迁移（旧部署的 base 缺 connectivity_check，幂等）
        Self::ensure_direct_rules_in_base().await?;
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

    #[test]
    fn test_rule_def_has_private_ip_default() {
        let private = ROUTING_RULES.iter().find(|r| r.id == "private_ip").unwrap();
        assert!(private.default_enabled);
    }

    #[test]
    fn test_rule_def_has_cn_rules_default_enabled() {
        for id in ["cn_ip", "cn_domain"] {
            let rule = ROUTING_RULES.iter().find(|r| r.id == id).unwrap();
            assert!(rule.default_enabled, "{} 应默认启用（禁回国流量）", id);
        }
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
        assert!(
            RoutingManager::ensure_direct_rules_value(&mut v),
            "首次应插入"
        );
        let after_first = v["routing"]["rules"].clone();
        assert!(
            !RoutingManager::ensure_direct_rules_value(&mut v),
            "二次应无变更"
        );
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

    // ── ensure_direct_rules_in_base ───────────────────────────────

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
}
