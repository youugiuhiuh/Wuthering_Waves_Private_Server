use crate::core::paths::xray;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde_json::{Value, json};
use tokio::sync::Mutex;

// 拆分后仍从本模块导出，保持既有调用方路径不变（handlers/message.rs 有 6 处引用两种判定）。
pub use super::custom_direct::{
    CustomAddOutcome, CustomDomainError, match_custom_direct, matches_builtin_direct,
    matches_connectivity_check, normalize_custom_domain,
};

pub(super) static CONFIG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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
    // geosite:cn 收录了这些域名，若排在 cn_ip / cn_domain 之后即失效。
    //
    // 注意 id 为历史命名：本规则覆盖三类被 geosite:cn 收录、被 cn_domain
    // blackhole 的关键域名：
    //   1. 连通性探测端点（generate_204）
    //   2. Google 登录必需的静态资源（ssl.gstatic.com 见 ChromeOS sign-in allowlist）
    //   3. Google Fonts 样式表与字体文件（fonts.googleapis.com 发 CSS，fonts.gstatic.com 发字体）
    RuleDef {
        id: "connectivity_check",
        rule_type: "domain",
        targets: &[
            "www.gstatic.com",
            "connectivitycheck.gstatic.com",
            "ssl.gstatic.com",
            "fonts.gstatic.com",
            "fonts.googleapis.com",
        ],
        outbound: "direct",
        default_enabled: true,
    },
    // 外網必需服務直連：geosite:cn 误收的外网必需服务端点（Google/Apple/Microsoft），
    // 必须早于 cn_ip / cn_domain，否则被 blackhole。
    // 硬约束：不放行任何广告/追踪域名（由 test_essential_direct_excludes_ads_and_tracking
    // 的 14 域名 + 8 模式断言固化）——故不整包引用 geosite:google-cn。
    RuleDef {
        id: "essential_direct",
        rule_type: "domain",
        targets: &[
            // ── Google / YouTube 功能必需（29）──
            "domain:recaptcha.net",
            "domain:safebrowsing.googleapis.com",
            "domain:safebrowsing-cache.google.com",
            "domain:update.googleapis.com",
            "domain:dl.google.com",
            "domain:dl.l.google.com",
            "domain:tools.google.com",
            "domain:clientservices.googleapis.com",
            "domain:performanceparameters.googleapis.com",
            "domain:tac.googleapis.com",
            "domain:crashlyticsreports-pa.googleapis.com",
            "domain:firebase-settings.crashlytics.com",
            "domain:update.crashlytics.com",
            "domain:checkin.gstatic.com",
            "domain:csi.gstatic.com",
            "domain:g0.gstatic.com",
            "domain:g1.gstatic.com",
            "domain:g2.gstatic.com",
            "domain:g3.gstatic.com",
            "domain:fontfiles.googleapis.com",
            "domain:redirector.gvt1.com",
            "domain:redirector.gcpcdn.gvt1.com",
            "domain:redirector.offline-maps.gvt1.com",
            "domain:redirector.snap.gvt1.com",
            "domain:beacons.gvt2.com",
            "domain:beacons2.gvt2.com",
            "domain:beacons3.gvt2.com",
            // 覆盖 geosite:cn 的 YouTube CDN regex，避免枚举轮换节点
            "domain:googlevideo.com",
            "domain:youtube-dubbing.com",
            // ── Apple（2）──
            // 165 条，覆盖 ocsp/crl/mesu/swscan/swdist/swcdn/gs-loc/cl2-cl5/init.ess/guzzoni/...
            "geosite:apple-cn",
            // apple-cn 唯一漏项
            "domain:init.itunes.apple.com",
            // ── Microsoft（8）──
            // crl/ocsp.microsoft.com 等 6 条
            "geosite:microsoft-pki",
            "domain:download.microsoft.com",
            "domain:download.visualstudio.microsoft.com",
            "domain:officecdn.microsoft.com",
            "domain:storeedge.microsoft.com",
            "domain:storeedgefd.dsx.mp.microsoft.com",
            "domain:dcg.microsoft.com",
            "domain:sdx.microsoft.com",
        ],
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

    pub(super) async fn read_base_json() -> Result<(Value, String)> {
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

    /// 纯函数：确保 routing.rules 满足「direct 规则不变量」并返回是否发生变更。
    /// 不触碰文件系统，便于单测。
    ///
    /// 不变量（Xray routing 顺序匹配、首条命中即停）：
    /// 所有 `outbound == "direct"` 的规则必须按 `ROUTING_RULES` 顺序排在
    /// 所有 `blocked` 规则之前；否则它们落在 cn_ip / cn_domain 之后而完全失效。
    ///
    /// 迁移是唯一修复路径（`toggle()` 用 push 追加到末尾，不改），因此：
    ///   - 缺失的 `default_enabled` direct 规则（connectivity_check、essential_direct）会被插入；
    ///   - 已存在的 direct 规则会被同步为 `rule_def_to_json(定义)`（修过时内容）并前置；
    ///   - `default_enabled == false` 的 direct 规则（openai）**不主动插入**
    ///     （插入等于默认打开它，违反其定义），仅在已存在时校正内容与位置；
    ///   - 其余规则保持原有相对顺序。
    pub fn ensure_direct_rules_value(v: &mut Value) -> bool {
        // 规范化 routing 与 routing.rules 的存在性
        if v.get("routing").map(|r| r.is_null()).unwrap_or(true) {
            v["routing"] = Value::Object(serde_json::Map::new());
        }
        if v["routing"]["rules"].as_array().is_none() {
            v["routing"]["rules"] = Value::Array(Vec::new());
        }

        let original = v["routing"]["rules"].as_array().unwrap().clone();

        let direct_ids: Vec<&'static str> = ROUTING_RULES
            .iter()
            .filter(|r| r.outbound == "direct")
            .map(|r| r.id)
            .collect();

        let mut reordered: Vec<Value> = Vec::with_capacity(original.len());

        // 1) direct 规则按 ROUTING_RULES 顺序置于最前，内容取当前定义（canonical）
        for def in ROUTING_RULES.iter().filter(|r| r.outbound == "direct") {
            let present = original
                .iter()
                .any(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some(def.id));
            if present || def.default_enabled {
                reordered.push(Self::rule_def_to_json(def));
            }
        }

        // 2) 其余（非 direct）规则保持原有相对顺序
        for rule in &original {
            let tag = rule.get("ruleTag").and_then(|t| t.as_str());
            let is_direct = tag.is_some_and(|t| direct_ids.contains(&t));
            if !is_direct {
                reordered.push(rule.clone());
            }
        }

        if reordered == original {
            return false;
        }
        v["routing"]["rules"] = Value::Array(reordered);
        true
    }

    /// 迁移：确保 00_base.json 的 routing.rules 满足 direct 规则不变量（幂等）。
    ///
    /// **仅在发生变更时**写盘并 reload 核心（与 `write_rules()` 对齐）；
    /// 无变更时零副作用（不写盘、不 reload，避免打断现有连接）。由
    /// get_all_with_status() 在用户打开路由菜单时触发。
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
        crate::core::system::maintenance::MaintenanceManager::reload_core().await
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
        assert_eq!(ROUTING_RULES.len(), 9);
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

    /// 外網必需服務直連：独立规则，直连出站、默认启用。
    #[test]
    fn test_essential_direct_rule_shape() {
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        assert_eq!(rule.rule_type, "domain");
        assert_eq!(rule.outbound, "direct");
        assert!(rule.default_enabled, "essential_direct 应默认启用");
    }

    /// 外網必需服務直連必须紧接 connectivity_check 之后、早于 cn_ip / cn_domain。
    /// Xray routing 顺序匹配、首条命中即停；排在 cn 规则之后就完全不生效，
    /// 而 cn_domain（geosite:cn）会把这些外網必需域名 blackhole。
    #[test]
    fn test_essential_direct_precedes_cn_rules() {
        let pos = |id: &str| {
            ROUTING_RULES
                .iter()
                .position(|r| r.id == id)
                .unwrap_or_else(|| panic!("规则 {} 不存在", id))
        };
        assert!(
            pos("connectivity_check") < pos("essential_direct"),
            "essential_direct 必须排在 connectivity_check 之后"
        );
        assert!(
            pos("essential_direct") < pos("cn_ip"),
            "essential_direct 必须排在 cn_ip 之前"
        );
        assert!(
            pos("essential_direct") < pos("cn_domain"),
            "essential_direct 必须排在 cn_domain 之前"
        );
    }

    /// 清单条目一律带显式前缀：`domain:`（apex＋子域语义）或白名单 `geosite:`。
    /// Xray 裸字符串是关键字**子字符串**匹配，会误命中 `www.gstatic.com.evil.com`
    /// 这类域名，且与自检函数的语义不一致；故禁止裸域名。
    #[test]
    fn test_essential_direct_targets_use_explicit_prefix() {
        // 39 = 37 條 domain: + 2 條 geosite:；SPEC 標題的「37 條」經編排者裁定為筆誤（只數了 domain: 行），將於文檔提交更正為 39。
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        assert_eq!(
            rule.targets.len(),
            39,
            "清单必须恰为 39 条（SPEC 逐字清单：29 Google + 2 Apple + 8 Microsoft）"
        );
        const ALLOWED_GEOSITE: &[&str] = &["geosite:apple-cn", "geosite:microsoft-pki"];
        for &t in rule.targets {
            let ok = t.starts_with("domain:") || ALLOWED_GEOSITE.contains(&t);
            assert!(
                ok,
                "条目必须带 domain: 前缀或为白名单 geosite 条目，禁止裸域名: {}",
                t
            );
        }
    }

    /// 硬约束：不得放行广告/追踪域名。以 14 个具体域名 + 8 个模式双重断言固化，
    /// 防止日后手滑把 `geosite:google-cn` 那 28 条广告/追踪条目加回。
    #[test]
    fn test_essential_direct_excludes_ads_and_tracking() {
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");

        const DENIED_DOMAINS: &[&str] = &[
            "app-measurement.com",
            "imasdk.googleapis.com",
            "adservice.google.com",
            "pagead-googlehosted.l.google.com",
            "ssl-google-analytics.l.google.com",
            "www-google-analytics.l.google.com",
            "www-googletagmanager.l.google.com",
            "google-analytics.com",
            "googletagmanager.com",
            "googleadservices.com",
            "googlesyndication.com",
            "googletagservices.com",
            "doubleclick.net",
            "googleoptimize.com",
        ];
        const DENIED_SUBSTRINGS: &[&str] = &[
            "pagead",
            "doubleclick",
            "adservices",
            "syndication",
            "googletagmanager",
            "-analytics",
            "app-measurement",
            "imasdk",
        ];

        for &t in rule.targets {
            let host = t
                .strip_prefix("domain:")
                .or_else(|| t.strip_prefix("geosite:"))
                .unwrap_or(t);
            for d in DENIED_DOMAINS {
                assert_ne!(host, *d, "不得放行广告/追踪域名: {}", t);
                assert!(
                    !host.ends_with(&format!(".{}", d)),
                    "不得放行广告/追踪域名的子域: {}",
                    t
                );
            }
            for p in DENIED_SUBSTRINGS {
                assert!(!host.contains(p), "条目 {} 命中广告/追踪模式 {}", t, p);
            }
        }
    }

    /// 实测证据支撑的必需端點必须在场（否则登录 / YouTube CDN / 安全浏览
    /// 仍被 cn_domain blackhole）。
    #[test]
    fn test_essential_direct_contains_evidence_backed_hosts() {
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        for required in [
            "domain:recaptcha.net",
            "domain:googlevideo.com",
            "domain:safebrowsing.googleapis.com",
            "geosite:apple-cn",
            "geosite:microsoft-pki",
            "domain:init.itunes.apple.com",
        ] {
            assert!(
                rule.targets.contains(&required),
                "必需端點 {} 必须在场（实测证据支撑）",
                required
            );
        }
    }

    /// 清单卫生：无重复、全小写、无 scheme、无路径、无空格、无 IP、无 regexp:。
    #[test]
    fn test_essential_direct_targets_unique_lowercase_no_scheme() {
        let rule = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        let mut seen = std::collections::HashSet::new();
        for &t in rule.targets {
            assert!(seen.insert(t), "条目重复: {}", t);
            assert_eq!(t, t.to_lowercase(), "条目必须全小写: {}", t);
            assert!(!t.contains("://"), "条目不得含 scheme: {}", t);
            assert!(!t.contains('/'), "条目不得含路径: {}", t);
            assert!(!t.contains(' '), "条目不得含空格: {}", t);
            assert!(!t.starts_with("regexp:"), "条目不得用 regexp: {}", t);
            let host = t
                .strip_prefix("domain:")
                .or_else(|| t.strip_prefix("geosite:"))
                .unwrap_or(t);
            assert!(
                host.parse::<std::net::IpAddr>().is_err(),
                "条目不得是 IP: {}",
                t
            );
        }
    }

    /// 域名清单必须恰为这 5 项（连通性探测 + Google 登录必需静态资源 +
    /// Google Fonts 样式表），且不得混入其余被 geosite:cn 收录的 gstatic/
    /// googleapis 资源域名。
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
            &[
                "www.gstatic.com",
                "connectivitycheck.gstatic.com",
                "ssl.gstatic.com",
                "fonts.gstatic.com",
                "fonts.googleapis.com",
            ],
            "域名清单必须恰为这 5 项"
        );
        // 不得出现 scheme 或路径 —— Xray 只匹配 SNI / Host
        for t in rule.targets {
            assert!(
                !t.contains("://") && !t.contains('/'),
                "targets 必须是纯主机名，不能含 scheme 或路径: {}",
                t
            );
        }
        // 不得混入其余被 geosite:cn 收录的资源 CDN / 签到 / 遥测域名
        for t in rule.targets {
            assert!(
                !t.starts_with("csi.")
                    && !t.starts_with("g0.")
                    && !t.starts_with("g1.")
                    && !t.starts_with("g2.")
                    && !t.starts_with("g3.")
                    && !t.starts_with("checkin.")
                    && !t.starts_with("fontfiles.")
                    && !t.starts_with("update.")
                    && !t.starts_with("tac.")
                    && !t.starts_with("clientservices.")
                    && !t.starts_with("safebrowsing.")
                    && !t.starts_with("wear."),
                "不应放行其余资源 CDN / 遥测域名: {}",
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
        // 经 Ruling 10 授权：迁移泛化后同时插入 default_enabled 的 essential_direct
        // （行为源自 SPEC Success Criteria #4），故长度 2→3。
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
        assert_eq!(rules[0]["outboundTag"], "direct");
        assert_eq!(rules[1]["ruleTag"], "essential_direct");
        assert_eq!(rules[2]["ruleTag"], "cn_ip");
    }

    /// 迁移是错位的唯一修复路径：toggle() 用 push 把规则追加到末尾（cn_domain
    /// 之后），那里因首条命中即停而完全失效。迁移必须把它提到首位。
    #[test]
    fn test_ensure_direct_rules_hoists_misplaced_rule_to_index_zero() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]},
            {"type": "field", "ruleTag": "connectivity_check", "outboundTag": "direct", "domain": ["www.gstatic.com"]}
        ]));
        assert!(
            RoutingManager::ensure_direct_rules_value(&mut v),
            "错位的规则应被移动，视为有变更"
        );
        let rules = v["routing"]["rules"].as_array().unwrap();
        // 经 Ruling 11（同类预先授权）：插入 default_enabled 的 essential_direct 使长度 2→3。
        assert_eq!(rules.len(), 3, "移动不得产生重复条目");
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(
            tags,
            vec!["connectivity_check", "essential_direct", "cn_ip"]
        );
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
        // 经 Ruling 10 授权：空 base 现插入 cc + essential_direct（SPEC Success Criteria #4），长度 1→2。
        assert_eq!(v["routing"]["rules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_ensure_direct_rules_handles_missing_rules_key() {
        let mut v = serde_json::json!({"routing": {"domainStrategy": "IPIfNonMatch"}});
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        // 经 Ruling 10 授权：长度 1→2 并补 essential_direct（SPEC Success Criteria #4）。
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
        assert_eq!(rules[1]["ruleTag"], "essential_direct");
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
        // 经 Ruling 10 授权：保留精确有序比对，仅插入 essential_direct（SPEC Success Criteria #4）。
        assert_eq!(
            tags,
            vec![
                "connectivity_check",
                "essential_direct",
                "private_ip",
                "cn_ip",
                "cn_domain"
            ]
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
            serde_json::json!([
                "www.gstatic.com",
                "connectivitycheck.gstatic.com",
                "ssl.gstatic.com",
                "fonts.gstatic.com",
                "fonts.googleapis.com"
            ])
        );
        assert!(rule.get("ip").is_none(), "domain 规则不应带 ip 键");
    }

    /// 存量机器上规则已在首位但内容过时（缺新增域名）时必须被更新。
    /// 旧迁移只看 tag 是否存在就早退，导致新增域名永远进不去。
    #[test]
    fn test_ensure_direct_rules_updates_stale_targets_at_index_zero() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "connectivity_check", "outboundTag": "direct",
             "domain": ["www.gstatic.com"]},
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]}
        ]));
        assert!(
            RoutingManager::ensure_direct_rules_value(&mut v),
            "内容过时应视为有变更"
        );
        let rules = v["routing"]["rules"].as_array().unwrap();
        // 经 Ruling 10 授权：essential_direct 插入使长度 2→3，cn_ip 索引由 1 位移到 2；
        // cc 的 domain 期望维持裸串（Ruling C 仍有效，E6 才做前缀正規化）。
        assert_eq!(rules.len(), 3, "不得重复插入");
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
        assert_eq!(
            rules[0]["domain"],
            serde_json::json!([
                "www.gstatic.com",
                "connectivitycheck.gstatic.com",
                "ssl.gstatic.com",
                "fonts.gstatic.com",
                "fonts.googleapis.com"
            ]),
            "过时内容应被当前定义覆盖"
        );
        assert_eq!(rules[2]["ruleTag"], "cn_ip", "其余规则不得受影响");
    }

    /// 旧存量 base（仅 cc + 阻塞规则）经迁移后应插入 essential_direct，
    /// 其内容 = rule_def_to_json(当前定义)；direct 规则按 ROUTING_RULES 顺序排在
    /// 所有 blocked 规则之前。
    #[test]
    fn test_ensure_direct_rules_inserts_essential_direct() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "connectivity_check", "outboundTag": "direct",
             "domain": ["www.gstatic.com", "connectivitycheck.gstatic.com", "ssl.gstatic.com",
                        "fonts.gstatic.com", "fonts.googleapis.com"]},
            {"type": "field", "ruleTag": "private_ip", "outboundTag": "blocked", "ip": ["geoip:private"]},
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]},
            {"type": "field", "ruleTag": "cn_domain", "outboundTag": "blocked", "domain": ["geosite:cn"]}
        ]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(
            tags,
            vec![
                "connectivity_check",
                "essential_direct",
                "private_ip",
                "cn_ip",
                "cn_domain"
            ]
        );
        // canonical 一律取 rule_def_to_json(定义)，不硬编码前缀字符串
        let def = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        assert_eq!(rules[1], RoutingManager::rule_def_to_json(def));
    }

    /// 错位的 direct 规则（openai 与 essential_direct 被 toggle 的 push 追加到末尾，
    /// 落在 blocked 之后而成为死规则）必须被移到所有 blocked 之前，并同步为 canonical。
    /// 这同时回归修复既有 openai 死规则问题。
    #[test]
    fn test_ensure_direct_rules_moves_misplaced_direct_before_blocked() {
        let mut v = base_with_rules(serde_json::json!([
            {"type": "field", "ruleTag": "connectivity_check", "outboundTag": "direct",
             "domain": ["www.gstatic.com", "connectivitycheck.gstatic.com", "ssl.gstatic.com",
                        "fonts.gstatic.com", "fonts.googleapis.com"]},
            {"type": "field", "ruleTag": "private_ip", "outboundTag": "blocked", "ip": ["geoip:private"]},
            {"type": "field", "ruleTag": "cn_ip", "outboundTag": "blocked", "ip": ["geoip:cn"]},
            {"type": "field", "ruleTag": "cn_domain", "outboundTag": "blocked", "domain": ["geosite:cn"]},
            {"type": "field", "ruleTag": "essential_direct", "outboundTag": "direct",
             "domain": ["domain:recaptcha.net"]},
            {"type": "field", "ruleTag": "openai", "outboundTag": "direct",
             "domain": ["geosite:openai"]}
        ]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        // 两者都被移到所有 blocked 之前，且按 ROUTING_RULES 顺序（essential_direct 先于 openai）
        assert_eq!(
            tags,
            vec![
                "connectivity_check",
                "essential_direct",
                "openai",
                "private_ip",
                "cn_ip",
                "cn_domain"
            ]
        );
        let ed = ROUTING_RULES
            .iter()
            .find(|r| r.id == "essential_direct")
            .expect("essential_direct 规则必须存在");
        let oa = ROUTING_RULES
            .iter()
            .find(|r| r.id == "openai")
            .expect("openai 规则必须存在");
        assert_eq!(rules[1], RoutingManager::rule_def_to_json(ed));
        assert_eq!(rules[2], RoutingManager::rule_def_to_json(oa));
    }

    /// 全 canonical 后再次迁移必须零变更：返回 false 且 JSON 完全不变。
    #[test]
    fn test_ensure_direct_rules_idempotent_all_canonical() {
        let mut v = base_with_rules(serde_json::json!([]));
        assert!(
            RoutingManager::ensure_direct_rules_value(&mut v),
            "首次应完成迁移"
        );
        let after_first = v["routing"]["rules"].clone();
        assert!(
            !RoutingManager::ensure_direct_rules_value(&mut v),
            "全 canonical 后二次调用应返回 false"
        );
        assert_eq!(v["routing"]["rules"], after_first, "二次调用不得改动 JSON");
    }

    /// 迁移只插入 default_enabled 的 direct 规则（connectivity_check、essential_direct）；
    /// default_disabled 的 openai 不得被主动插入（插入等于默认打开它，违反其定义），
    /// 也不得因缺失 openai 而把返回值误报为 true。
    #[test]
    fn test_ensure_direct_rules_does_not_insert_default_disabled_direct() {
        let mut v = base_with_rules(serde_json::json!([]));
        assert!(RoutingManager::ensure_direct_rules_value(&mut v));
        let rules = v["routing"]["rules"].as_array().unwrap();
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(tags, vec!["connectivity_check", "essential_direct"]);
        assert!(
            !tags.contains(&"openai"),
            "不得插入 default_disabled 的 openai"
        );
        assert!(
            !RoutingManager::ensure_direct_rules_value(&mut v),
            "缺少 openai 不应被视作变更"
        );
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
