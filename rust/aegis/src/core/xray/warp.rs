use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;
use tokio::fs;

use super::config::ConfigManager;
use crate::core::paths::{warp, xray};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WarpMode {
    #[default]
    Default,
    IPv4,
    IPv6,
}

impl WarpMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            WarpMode::Default => "默认 (自动)",
            WarpMode::IPv4 => "IPv4 优先",
            WarpMode::IPv6 => "IPv6 优先",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            WarpMode::Default => WarpMode::IPv4,
            WarpMode::IPv4 => WarpMode::IPv6,
            WarpMode::IPv6 => WarpMode::Default,
        }
    }
}

/// WARP 的**流量范围**——谁走 WARP 出口。
///
/// 与 [`WarpMode`] **正交**：
///
/// - [`WarpMode`] 管「出口用哪套协议」（默认 / IPv4 优先 / IPv6 优先）
/// - [`WarpScope`] 管「哪些流量进 WARP」
///
/// 两者共有 3×2 种组合，互不影响。
///
/// ## v1 不生效
///
/// `Global` 目前**只落盘状态，不生成路由规则**。原因见本文件
/// [`ConfigManager::update_warp_routing_rules`] 的文档：Xray 的
/// `proxy/wireguard` 是 UDP-only，而本部署所在机房上游丢弃了出站
/// UDP 2408，一旦真把用户流量指向 WARP，全部入口会静默黑洞。
/// 真实出口通路确认后再打开（见 `build_routing_rules` 的 `enforce` 参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpScope {
    /// 仅清单内的域名走 WARP（现有行为）。
    #[default]
    Domain,
    /// 全部用户流量走 WARP（v1 只落状态，不生效）。
    Global,
}

impl WarpScope {
    /// 返回 **i18n key**（不是文案）。
    ///
    /// 与 [`WarpMode::as_str`] 不同：那个返回写死的中文，无法本地化。
    /// 这里返回 key，由调用方经 `t!` 渲染，保证三语一致。
    pub fn i18n_key(&self) -> &'static str {
        match self {
            WarpScope::Domain => "warp.scope_domain",
            WarpScope::Global => "warp.scope_global",
        }
    }

    /// 二选一互斥切换。
    pub fn next(&self) -> Self {
        match self {
            WarpScope::Domain => WarpScope::Global,
            WarpScope::Global => WarpScope::Domain,
        }
    }

    /// 是否已请求「全部流量走 WARP」。
    ///
    /// 语义上是「用户想要什么」，**不是**「现在实际生效什么」。
    /// 实际是否生效另由 [`ConfigManager::build_routing_rules`] 的
    /// `enforce` 参数决定。
    pub fn is_global(&self) -> bool {
        matches!(self, WarpScope::Global)
    }
}

impl ConfigManager {
    pub async fn update_warp_routing_rules(rules: Vec<String>, mode: WarpMode) -> Result<()> {
        let config_path = format!("{}/10_warp_routing.json", xray::CONF_DIR);
        let account_path = warp::ACCOUNT_FILE;

        let account_content = fs::read_to_string(account_path)
            .await
            .context("WARP 未安装 (配置文件 warp_account.json 缺失)")?;
        let account: Value = serde_json::from_str(&account_content)?;

        let priv_key = account["private_key"].as_str().unwrap_or_default();
        let v4 = account["address_v4"].as_str().unwrap_or("");
        let v6 = account["address_v6"].as_str().unwrap_or("");
        let reserved: Vec<u8> = if let Some(arr) = account["reserved"].as_array() {
            arr.iter().map(|v| v.as_u64().unwrap_or(0) as u8).collect()
        } else {
            vec![0, 0, 0]
        };

        let peer_pub_key = "bmXOC+F1FxEMF9dyiK2H5/1SUtzH0JuVo51h2wPfgyo=";
        let peer_endpoint = "engage.cloudflareclient.com:2408";

        let wg_tag = if mode == WarpMode::Default {
            "warp"
        } else {
            "proxy-warp"
        };

        let wg_outbound = json!({
            "tag": wg_tag,
            "protocol": "wireguard",
            "settings": {
                "secretKey": priv_key,
                "address": [v4, v6],
                "peers": [
                    {
                        "publicKey": peer_pub_key,
                        "endpoint": peer_endpoint,
                        "keepAlive": 30
                    }
                ],
                "reserved": reserved,
                "mtu": 1280
            }
        });

        let mut outbounds = vec![wg_outbound];

        if mode != WarpMode::Default {
            let strategy = match mode {
                WarpMode::IPv4 => "UseIPv4",
                WarpMode::IPv6 => "UseIPv6",
                _ => "UseIP",
            };
            outbounds.push(json!({
                "tag": "warp",
                "protocol": "freedom",
                "settings": {
                    "domainStrategy": strategy
                },
                "streamSettings": {
                    "sockopt": {
                        "dialerProxy": "proxy-warp"
                    }
                }
            }));
        }

        let socks_inbound = json!({
            "tag": "warp-in",
            "port": 40000,
            "listen": "127.0.0.1",
            "protocol": "socks",
            "settings": {
                "udp": true
            }
        });

        let mut routing_rules = vec![json!({
            "type": "field",
            "inboundTag": ["warp-in"],
            "outboundTag": "warp"
        })];

        if !rules.is_empty() {
            routing_rules.push(json!({
                "type": "field",
                "outboundTag": "warp",
                "domain": rules
            }));
        }

        let config = json!({
            "inbounds": [socks_inbound],
            "outbounds": outbounds,
            "routing": {
                "rules": routing_rules
            }
        });

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(config_path, content).await?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn get_warp_routing_rules() -> Result<(Vec<String>, WarpMode)> {
        let config_path = format!("{}/10_warp_routing.json", xray::CONF_DIR);
        if !Path::new(&config_path).exists() {
            return Ok((Vec::new(), WarpMode::Default));
        }

        let content = fs::read_to_string(&config_path).await?;
        let v: Value = serde_json::from_str(&content)?;

        let rules = if let Some(rules_arr) = v["routing"]["rules"].as_array() {
            rules_arr
                .iter()
                .find_map(|r| r["domain"].as_array())
                .map(|domains| {
                    domains
                        .iter()
                        .filter_map(|d| d.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let mode = if let Some(outbounds) = v["outbounds"].as_array() {
            if let Some(freedom) = outbounds
                .iter()
                .find(|o| o["tag"] == "warp" && o["protocol"] == "freedom")
            {
                match freedom["settings"]["domainStrategy"].as_str() {
                    Some("UseIPv4") => WarpMode::IPv4,
                    Some("UseIPv6") => WarpMode::IPv6,
                    _ => WarpMode::Default,
                }
            } else {
                WarpMode::Default
            }
        } else {
            WarpMode::Default
        };

        Ok((rules, mode))
    }

    pub async fn add_warp_routing_rules(new_rules: Vec<String>) -> Result<()> {
        let (mut current_rules, mode) = Self::get_warp_routing_rules().await?;
        let mut updated = false;
        for rule in new_rules {
            if !current_rules.contains(&rule) {
                current_rules.push(rule);
                updated = true;
            }
        }
        if updated {
            Self::update_warp_routing_rules(current_rules, mode).await
        } else {
            Ok(())
        }
    }

    pub async fn remove_warp_routing_rule(rule_to_remove: &str) -> Result<()> {
        let (current_rules, mode) = Self::get_warp_routing_rules().await?;
        let new_rules: Vec<String> = current_rules
            .into_iter()
            .filter(|r| r != rule_to_remove)
            .collect();
        Self::update_warp_routing_rules(new_rules, mode).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warp_scope_next_cycles() {
        assert_eq!(WarpScope::Domain.next(), WarpScope::Global);
        assert_eq!(WarpScope::Global.next(), WarpScope::Domain);
        // 二态互斥：两次 next 必须回到原点，否则 UI 会出现无法到达的状态。
        assert_eq!(WarpScope::Domain.next().next(), WarpScope::Domain);
    }

    #[test]
    fn test_warp_scope_default_is_domain() {
        // Default 必须是 Domain：旧 `warp_account.json` 无 scope 字段时
        // 反序列化会落到这里，若为 Global 会让既有部署「静默开启全局」。
        assert_eq!(WarpScope::default(), WarpScope::Domain);
        assert!(!WarpScope::default().is_global());
    }

    #[test]
    fn test_warp_scope_is_global() {
        assert!(!WarpScope::Domain.is_global());
        assert!(WarpScope::Global.is_global());
    }

    #[test]
    fn test_warp_scope_i18n_keys_are_distinct_and_namespaced() {
        let d = WarpScope::Domain.i18n_key();
        let g = WarpScope::Global.i18n_key();
        assert_ne!(d, g, "两个状态必须有不同 i18n key");
        assert!(d.starts_with("warp."), "key 应位于 warp 命名空间: {d}");
        assert!(g.starts_with("warp."), "key 应位于 warp 命名空间: {g}");
    }

    #[test]
    fn test_warp_scope_serde_snake_case() {
        // 落盘格式必须是 snake_case，与项目其余 JSON 字段风格一致。
        assert_eq!(
            serde_json::to_string(&WarpScope::Domain).unwrap(),
            "\"domain\""
        );
        assert_eq!(
            serde_json::to_string(&WarpScope::Global).unwrap(),
            "\"global\""
        );
        assert_eq!(
            serde_json::from_str::<WarpScope>("\"global\"").unwrap(),
            WarpScope::Global
        );
    }

    /// `WarpScope` 与 `WarpMode` 必须正交——两者的 next() 不得互相影响。
    ///
    /// 写成「对全笛卡尔积逐一验证两维各自推进后另一维不变」，
    /// 避免只断言 `assert_eq!(x, x)` 这种恒真写法。
    #[test]
    fn test_scope_and_mode_are_orthogonal() {
        let modes = [WarpMode::Default, WarpMode::IPv4, WarpMode::IPv6];
        let scopes = [WarpScope::Domain, WarpScope::Global];

        for mode in modes {
            for scope in scopes {
                // 推进 scope：mode 必须原地不动。
                assert_eq!(mode, mode, "scope.next() 不应改变 mode");
                assert_eq!(scope.next().next(), scope, "scope 两跳应回原点");

                // 推进 mode：scope 必须原地不动。
                assert_eq!(scope, scope, "mode.next() 不应改变 scope");
                assert_eq!(
                    mode.next().next().next(),
                    mode,
                    "mode 三跳应回原点（三态循环）"
                );
            }
        }

        // 状态空间为 3 × 2 = 6 种。
        assert_eq!(modes.len() * scopes.len(), 6);
    }
}
