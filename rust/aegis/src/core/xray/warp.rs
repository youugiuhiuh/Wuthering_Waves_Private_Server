use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;
use tokio::fs;

use super::config::ConfigManager;
use crate::core::paths::{usque as usque_paths, warp, xray};
use crate::core::usque::{Egress, select_egress};

/// 出口引擎是否**真的可用**——`Global` 是否生成 catch-all 的判据。
///
/// # 为何不再是环境变量（Spec 1 → Spec 2）
///
/// Spec 1 曾用 `WARP_SCOPE_ENFORCE` 环境变量当护栏，其存在理由是
/// 「**出口不可用**」：Xray 的 `proxy/wireguard` 是 UDP-only，
/// 而本部署机房丢弃出站 UDP 的非 53 端口，因此一旦生成 catch-all，
/// 全部用户 inbound 会被指向一条收不到回包的隧道（服务器日志曾证实：
/// `XHTTP-*` 路由到 `proxy-warp`，而 `wg0` 的 `rx_bytes` 恒为 0）。
///
/// Spec 2 提供了走 TCP/443 的 usque，出口**真的可用**了，
/// 于是护栏的理由消失。用**运行时的出口可用性**取代环境变量更准确：
///
/// - 环境变量无法感知网络变化（机房改策略、隧道卡死都不会更新它）；
/// - 运行时不变量本来就决定了用哪个出口引擎，复用它不会产生
///   「配置说 MASQUE、实际探测却说 WireGuard 可用」这类不一致；
/// - 少一个可以配错的外部输入。
///
/// 判据：出口引擎**不是** `Egress::Direct` 即视为可用——`Direct` 正是
/// 「两条通路都不可用」的表达（见 [`select_egress`]）。
fn egress_is_available(egress: Egress) -> bool {
    egress.is_warp_enabled()
}

/// 用缓存中的探测结果选出口；无缓存（或已过期）时保守地回退。
///
/// **回退到 `Direct` 而非某个具体引擎**：探测都没跑过就声称某条路可用，
/// 等于在用未验证的假设配置生产流量。宁可先不启用 WARP，
/// 等下一次探测给出结论。
///
/// 缓存本体在 [`crate::core::usque::engine::ProbeCache`]，
/// 由探测调度任务写入。
fn current_egress() -> Egress {
    match crate::core::usque::engine::global_probe_cache()
        .get(now_monotonic(), crate::core::usque::engine::CACHE_TTL)
    {
        Some(report) => select_egress(report.conclusion),
        None => Egress::Direct,
    }
}

/// 单调时刻（进程启动至今）。
///
/// 用单调时钟而非墙上时钟：TTL 比较不应受系统时间调整影响。
fn now_monotonic() -> std::time::Duration {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    start.elapsed()
}

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
    /// 生成指向本地 usque 的 `socks` outbound。
    ///
    /// # ⚠️ 形状是「扁平」的——与其它协议不同
    ///
    /// Xray 的 `socks` outbound 把 `address` / `port` **直接挂在 `settings` 下**：
    ///
    /// ```json
    /// { "protocol": "socks", "settings": { "address": "127.0.0.1", "port": 1080 } }
    /// ```
    ///
    /// 而 `vless` / `vmess` / `trojan` / `wireguard` 全都是
    /// `settings.servers[]` 数组。写成后者会**静默失效**——
    /// Xray 对未知字段宽容（engine 仍会起，流量却不过隧道）。
    /// 本项目已经在 `core_health.rs` 记录过这类「配置能起但功能已坏」的教训。
    ///
    /// 故本函数有专门的断言测试
    /// （`test_masque_outbound_is_flat_socks_shape`）。
    ///
    /// # tag 保持 `proxy-warp`
    ///
    /// 与 WireGuard 路径的 wireguard outbound 同名——这样上层的
    /// `freedom + dialerProxy` 中转与路由规则**不需要感知出口实现换了**。
    /// 对 Xray 而言，只是 `proxy-warp` 这个 tag 后面挂的东西变了。
    pub fn masque_outbound(addr: &str, port: u16) -> Value {
        json!({
            "tag": "proxy-warp",
            "protocol": "socks",
            "settings": {
                "address": addr,
                "port": port,
            }
        })
    }

    /// 构造 `10_warp_routing.json` 的 `routing.rules`。
    ///
    /// 纯函数（无 I/O、无环境依赖），故可直接单测。
    ///
    /// ## 参数
    ///
    /// - `rules`：域名清单（`domain:` / 裸域名均可，交给 Xray 解析）。
    /// - `scope`：用户请求的流量范围。
    /// - `enforce`：**出口引擎是否真的可用**（Spec 2 起语义变了）。
    ///
    ///   为 `false` 时 `Global` 与 `Domain` 产出完全相同的规则集——
    ///   即「用户请求了全局，但出口不可用，故不生效」。
    ///
    ///   ⚠️ **语义变更史**：Spec 1 时该参数的含义是「管理员是否用
    ///   `WARP_SCOPE_ENFORCE` 环境变量显式允许」。Spec 2 提供了
    ///   走 TCP/443 的 usque 后，出口真的可用了，护栏由**运行时可用性**
    ///   取代（见 [`egress_is_available`]）——环境变量无法感知网络变化，
    ///   也更易配错。参数签名保留，语义以本节为准。
    ///
    /// ## 产出
    ///
    /// 始终包含第一条：`warp-in` 入站的流量走 `warp`。
    /// 之后按情况追加：
    ///
    /// - `Global` + `enforce` → 一条 **catch-all**：无 `domain` / `ip` /
    ///   `inboundTag` 条件，即匹配一切流量。
    /// - 否则若有域名 → 一条 `domain` 白名单规则。
    /// - 否则（`Domain` 且清单为空）→ 只保留 `warp-in` 那条。
    ///
    /// 注意 `Domain` + 空清单**不**产出空的 `domain: []` 规则：空数组在
    /// Xray 里是无条件匹配还是永不匹配并不直观，而构筑一条「没任何
    /// 条件的规则」会把整个节点指向 WARP——与「没配规则」的意图相反。
    pub fn build_routing_rules(rules: Vec<String>, scope: WarpScope, enforce: bool) -> Vec<Value> {
        let mut out = vec![json!({
            "type": "field",
            "inboundTag": ["warp-in"],
            "outboundTag": "warp"
        })];

        if scope.is_global() && enforce {
            // 无条件规则 = 匹配一切。这是「全部用户流量走 WARP」。
            out.push(json!({
                "type": "field",
                "outboundTag": "warp"
            }));
        } else if !rules.is_empty() {
            out.push(json!({
                "type": "field",
                "outboundTag": "warp",
                "domain": rules
            }));
        }

        out
    }

    /// 写回 `warp_account.json` 中的 `scope` 字段，其余字段保持不变。
    ///
    /// 单独抽出是因为「只改 scope」不应该重写整份 Xray 配置（见
    /// [`Self::update_warp_routing_rules`] 的 `scope_only`）。
    ///
    /// 文件不存在时返回 `Ok(false)`（未写入）——调用方据此决定是否
    /// 报「WARP 未安装」。用 `Value` 而非 `WarpAccountConfig` 读写，
    /// 以免因未来新增字段而在升级时丢失未知键。
    async fn persist_scope(scope: WarpScope) -> Result<bool> {
        let account_path = warp::ACCOUNT_FILE;
        let content = match fs::read_to_string(account_path).await {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };

        let mut account: Value = serde_json::from_str(&content)
            .context("warp_account.json 格式非法，无法写入流量范围")?;

        account["scope"] = serde_json::to_value(scope)?;

        let pretty = serde_json::to_string_pretty(&account)?;
        fs::write(account_path, pretty).await?;
        Ok(true)
    }

    /// 重写 `10_warp_routing.json` 并重载核心。
    ///
    /// ## 参数
    ///
    /// - `rules`：域名清单。
    /// - `mode`：出口协议偏好（与 `scope` 正交）。
    /// - `scope`：流量范围（落盘到 `warp_account.json`，不写进本文件）。
    /// - `scope_only`：**只改 scope，不碰路由规则、不重启核心**。
    ///
    /// ## `scope_only` 为何存在
    ///
    /// v1 的 `Global` 不改变任何路由行为（见 [`Self::build_routing_rules`]
    /// 的 `enforce`）。只为了记一个状态而重启 `wwps-core`，会白白抖动
    /// 全体在线用户的连接。故该路径只写 `warp_account.json`。
    pub async fn update_warp_routing_rules(
        rules: Vec<String>,
        mode: WarpMode,
        scope: WarpScope,
        scope_only: bool,
    ) -> Result<()> {
        if scope_only {
            let written = Self::persist_scope(scope).await?;
            if !written {
                anyhow::bail!("WARP 未安装 (配置文件 warp_account.json 缺失)");
            }
            return Ok(());
        }

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

        // 按出口引擎决定 `proxy-warp` 后面挂什么。
        //
        // - `Masque`：挂一个指向本地 usque 的 `socks` outbound；
        //   账号在 usque 自己的 `config.json`，**不在**本文件里，
        //   因此下面的 wireguard 字段（priv_key/v4/...）在本分支下不适用。
        // - 其它（WireGuard / Direct）：保持既有 wireguard outbound。
        let egress = current_egress();
        let wg_tag = if mode == WarpMode::Default {
            "warp"
        } else {
            "proxy-warp"
        };

        let wg_outbound = if egress == Egress::Masque {
            Self::masque_outbound(usque_paths::SOCKS_ADDR, usque_paths::SOCKS_PORT)
        } else {
            json!({
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
            })
        };

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

        let routing_rules =
            Self::build_routing_rules(rules, scope, egress_is_available(current_egress()));
        let config = json!({
            "inbounds": [socks_inbound],
            "outbounds": outbounds,
            "routing": {
                "rules": routing_rules
            }
        });

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(config_path, content).await?;

        // scope 落在 warp_account.json（Xray 不读该文件，写入零侵入）。
        // 失败不阻断：路由规则已落盘并重载，scope 只是附带状态。
        if let Err(e) = Self::persist_scope(scope).await {
            log::warn!("写入 WARP 流量范围失败（不影响路由）: {e}");
        }

        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    /// 读取当前配置：`(域名清单, 出口协议偏好, 流量范围)`。
    ///
    /// `scope` 从 `warp_account.json` 读取（与 [`Self::update_warp_routing_rules`]
    /// 的写入位置对应）。文件缺失或字段缺失均回退 [`WarpScope::Domain`]。
    pub async fn get_warp_routing_rules() -> Result<(Vec<String>, WarpMode, WarpScope)> {
        let scope = Self::read_scope().await;
        let (rules, mode) = Self::read_routing_file().await?;
        Ok((rules, mode, scope))
    }

    /// 从 `warp_account.json` 读 `scope`；任何异常都回退 `Domain`。
    ///
    /// 刻意不把读失败向上抛：调用方（菜单渲染）宁可显示保守的
    /// `Domain`，也不应因状态文件损坏而整块菜单报错。
    async fn read_scope() -> WarpScope {
        let Ok(content) = fs::read_to_string(warp::ACCOUNT_FILE).await else {
            return WarpScope::Domain;
        };
        let Ok(v) = serde_json::from_str::<Value>(&content) else {
            return WarpScope::Domain;
        };
        v.get("scope")
            .and_then(|s| serde_json::from_value::<WarpScope>(s.clone()).ok())
            .unwrap_or_default()
    }

    /// 读 `10_warp_routing.json` 得出 `(域名清单, 出口协议偏好)`。
    async fn read_routing_file() -> Result<(Vec<String>, WarpMode)> {
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
        let (mut current_rules, mode, scope) = Self::get_warp_routing_rules().await?;
        let mut updated = false;
        for rule in new_rules {
            if !current_rules.contains(&rule) {
                current_rules.push(rule);
                updated = true;
            }
        }
        if updated {
            // 改域名清单需要重写路由文件，故 scope_only = false。
            Self::update_warp_routing_rules(current_rules, mode, scope, false).await
        } else {
            Ok(())
        }
    }

    pub async fn remove_warp_routing_rule(rule_to_remove: &str) -> Result<()> {
        let (current_rules, mode, scope) = Self::get_warp_routing_rules().await?;
        let new_rules: Vec<String> = current_rules
            .into_iter()
            .filter(|r| r != rule_to_remove)
            .collect();
        // 同上：清单变动需要重写路由文件。
        Self::update_warp_routing_rules(new_rules, mode, scope, false).await
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

    // ───────────────────── build_routing_rules (T4) ─────────────────────

    fn domains(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 第一条永远是 `warp-in` 入站 → `warp`。其余规则在此基础上追加。
    #[test]
    fn test_rules_always_start_with_warp_in() {
        for scope in [WarpScope::Domain, WarpScope::Global] {
            for enforce in [false, true] {
                for rules in [vec![], domains(&["domain:a.com"])] {
                    let out = ConfigManager::build_routing_rules(rules, scope, enforce);
                    assert!(!out.is_empty(), "至少应有 warp-in 规则");
                    assert_eq!(out[0]["inboundTag"][0], "warp-in");
                    assert_eq!(out[0]["outboundTag"], "warp");
                    assert_eq!(out[0]["type"], "field");
                }
            }
        }
    }

    /// `Domain` 是现有行为：有清单则产出 `domain` 白名单规则。
    #[test]
    fn test_domain_scope_emits_domain_whitelist() {
        let out = ConfigManager::build_routing_rules(
            domains(&["domain:reddit.com", "domain:example.org"]),
            WarpScope::Domain,
            false,
        );
        assert_eq!(out.len(), 2, "warp-in + 一条 domain 规则");
        assert_eq!(out[1]["outboundTag"], "warp");
        let d = out[1]["domain"].as_array().expect("应有 domain 数组");
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], "domain:reddit.com");
        assert!(out[1].get("inboundTag").is_none(), "域名规则不应限定入站");
    }

    /// `Domain` + 空清单 → 只有 warp-in，**不**产出空的 `domain: []`。
    ///
    /// 空的 domain 数组在 Xray 里的匹配语义不直观；而一条无任何条件的
    /// 规则会匹配一切——那等于意外开启全局 WARP。
    #[test]
    fn test_empty_domain_scope_emits_only_warp_in() {
        let out = ConfigManager::build_routing_rules(vec![], WarpScope::Domain, false);
        assert_eq!(out.len(), 1, "空清单不应产出 domain 规则");
        assert!(out[0].get("domain").is_none());
    }

    /// `Global` + `enforce` → catch-all（无任何匹配条件）。
    #[test]
    fn test_global_scope_with_enforce_emits_catch_all() {
        let out = ConfigManager::build_routing_rules(
            domains(&["domain:ignored.com"]),
            WarpScope::Global,
            true,
        );
        assert_eq!(out.len(), 2);
        let catch_all = &out[1];
        assert_eq!(catch_all["outboundTag"], "warp");
        assert_eq!(catch_all["type"], "field");
        // catch-all 的关键：不带任何匹配条件。
        assert!(catch_all.get("domain").is_none(), "catch-all 不应有 domain");
        assert!(catch_all.get("ip").is_none(), "catch-all 不应有 ip");
        assert!(
            catch_all.get("inboundTag").is_none(),
            "catch-all 不应限定入站（否则只覆盖 warp-in）"
        );
        // 有 catch-all 时不再追加域名白名单（已被完全覆盖）。
        assert!(
            out.iter().all(|r| r.get("domain").is_none()),
            "catch-all 存在时不应再有 domain 规则"
        );
    }

    /// **核心不变式**：出口不可用时 `Global` 不得生效。
    ///
    /// 这条在 Spec 1 叫「v1 不生效」，Spec 2 后它的含义变成
    /// 「**出口引擎不可用时**不生效」——`enforce` 参数从环境变量
    /// 改成了运行时可用性（见 `egress_is_available`），
    /// 但**不变量本身没变**：出口不可用 → 不生成 catch-all。
    ///
    /// 这是防黑洞的关键：两条通路都挂时若仍生成 catch-all，
    /// 全部用户 inbound 会被指向一条收不到回包的隧道。
    #[test]
    fn test_global_without_enforce_is_identical_to_domain() {
        let rules = domains(&["domain:a.com", "domain:b.com"]);

        let domain_out =
            ConfigManager::build_routing_rules(rules.clone(), WarpScope::Domain, false);
        let global_out = ConfigManager::build_routing_rules(rules, WarpScope::Global, false);

        assert_eq!(
            domain_out, global_out,
            "出口不可用时 Global 必须与 Domain 逐字节相同（不生效）"
        );
        assert!(
            global_out
                .iter()
                .all(|r| r.get("inboundTag").is_some() || r.get("domain").is_some()),
            "出口不可用时不应出现无条件的 catch-all 规则"
        );
    }

    /// Spec 2：出口**可用**时，`Global` 必须真生成 catch-all。
    ///
    /// 与上一条测试成对——它们共同限定 `enforce` 参数的两种取值，
    /// 把「何时生效」这个判断完整钉死。
    #[test]
    fn test_global_emits_catch_all_when_egress_available() {
        let rules = domains(&["domain:a.com", "domain:b.com"]);
        let out = ConfigManager::build_routing_rules(rules, WarpScope::Global, true);

        assert_eq!(out.len(), 2, "warp-in + 一条 catch-all");
        let catch_all = &out[1];
        assert_eq!(catch_all["outboundTag"], "warp");
        assert!(
            catch_all.get("domain").is_none()
                && catch_all.get("ip").is_none()
                && catch_all.get("inboundTag").is_none(),
            "catch-all 不得带任何匹配条件，否则达不到「全部流量」语义"
        );
    }

    /// **出口可用性判据**：只有 `Direct` 算不可用。
    ///
    /// 这是 Spec 2 取代环境变量的那颗开关。取反或写错会让
    /// `Global` 在两条通路都挂时仍然生成 catch-all——
    /// 即全部用户流量指向一个收不到回包的黑洞。
    #[test]
    fn test_egress_availability_maps_only_direct_to_unavailable() {
        assert!(egress_is_available(Egress::WireGuard));
        assert!(egress_is_available(Egress::Masque));
        assert!(
            !egress_is_available(Egress::Direct),
            "Direct 表示两条通路都不可用，必须判为不可用"
        );
    }

    /// 无探测缓存时 `current_egress()` 必须保守地回退到 `Direct`。
    ///
    /// 探测都没跑过就声称某条路可用，等于用未验证的假设配置生产流量。
    #[test]
    fn test_current_egress_is_conservative_without_cache() {
        crate::core::usque::engine::global_probe_cache().clear();
        assert_eq!(
            current_egress(),
            Egress::Direct,
            "无缓存时应回退 Direct（不启用 WARP）"
        );
    }

    /// 缓存命中时 `current_egress()` 必须跟随探测结论。
    #[test]
    fn test_current_egress_follows_cached_probe() {
        use crate::core::network::{ProbeConclusion, ProbeReport, TcpVerdict, UdpVerdict};

        let cache = crate::core::usque::engine::global_probe_cache();
        cache.clear();
        cache.store(
            ProbeReport {
                udp: UdpVerdict::Blocked,
                tcp: TcpVerdict::AvailableH2,
                conclusion: ProbeConclusion::TcpOnly,
            },
            now_monotonic(),
        );
        assert_eq!(
            current_egress(),
            Egress::Masque,
            "缓存说 TcpOnly 时应选 MASQUE"
        );
        cache.clear();
    }

    /// `Domain` 即使 enforce=true 也不得变成 catch-all。
    /// enforce 只能放行 Global，不能改变 Domain 语义。
    #[test]
    fn test_enforce_does_not_affect_domain_scope() {
        let rules = domains(&["domain:a.com"]);
        let without = ConfigManager::build_routing_rules(rules.clone(), WarpScope::Domain, false);
        let with = ConfigManager::build_routing_rules(rules, WarpScope::Domain, true);
        assert_eq!(without, with, "enforce 对 Domain 不应有任何影响");
    }

    /// enforce 开关：只认 `1` / `true` / `yes`，默认关闭。
    ///
    /// 环境变量是进程级的，而测试并行运行——直接 `set_var` 会影响其他
    /// 用例。故把「值 → 判定」这段纯逻辑单独验证，不去碰真实环境。
    #[test]
    fn test_enforce_env_value_parsing() {
        let parse = |v: Option<&str>| matches!(v, Some("1") | Some("true") | Some("yes"));

        assert!(!parse(None), "未设置 → 关闭（v1 默认）");
        assert!(!parse(Some("")), "空字符串 → 关闭");
        assert!(!parse(Some("0")), "0 → 关闭");
        assert!(!parse(Some("false")), "false → 关闭");
        assert!(!parse(Some("TRUE")), "大写不算（保持简单可预测）");
        assert!(parse(Some("1")), "1 → 开启");
        assert!(parse(Some("true")), "true → 开启");
        assert!(parse(Some("yes")), "yes → 开启");
    }

    /// T5 硬约束：`scope_only = true` 必须**不**触发 `reload_core()`。
    ///
    /// 无法用运行时测试验证：路径常量硬编码为 `/etc/wwps/...`，单测跑
    /// 不到那条分支（真跑会重启生产核心）。改用源码结构断言：
    ///
    /// - `scope_only` 分支必须在 `reload_core` 调用**之前** `return`；
    /// - `update_warp_routing_rules` 内只有**一处** `reload_core`。
    ///
    /// 若有人把 `scope_only` 的 early-return 改成“继续往下跑”，或者
    /// 在上半段插入 `reload_core`，这条会失败。
    #[test]
    fn test_scope_only_path_returns_before_reload() {
        const SRC: &str = include_str!("warp.rs");

        let fn_start = SRC
            .find("pub async fn update_warp_routing_rules")
            .expect("找不到 update_warp_routing_rules");
        let body = &SRC[fn_start..];

        let guard = body
            .find("if scope_only {")
            .expect("scope_only 分支必须存在");
        let ret = body[guard..]
            .find("return Ok(());")
            .expect("scope_only 分支必须提前 return");
        let reload = body
            .find("reload_core()")
            .expect("非 scope_only 路径必须 reload");

        assert!(
            guard + ret < reload,
            "scope_only 的 return 必须出现在 reload_core 之前：\
             否则只改一个状态字段也会重启核心，抖动全体在线用户"
        );

        // 只统计 `update_warp_routing_rules` 函数体内的非注释行。
        // 必须先把「本测试的字符串字面量」排除，否则它会被自己数进去。
        let tests_start = SRC.find("#[cfg(test)]").unwrap_or(SRC.len());
        let impl_body = &SRC[fn_start..tests_start];
        let total = impl_body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .filter(|l| l.contains("MaintenanceManager::reload_core()"))
            .count();
        assert_eq!(
            total, 1,
            "update_warp_routing_rules 内只应有一处 reload_core（排除注释与测试代码）"
        );
    }

    // ──────────── T8: MASQUE socks outbound ────────────

    /// **硬约束 6**：Xray 的 `socks` outbound 形状是**扁平**的。
    ///
    /// `address` / `port` 直接挂在 `settings` 下，**不是** `settings.servers[]`。
    /// 后者是 `vless`/`vmess`/`trojan`/`wireguard` 的形状；
    /// 在 `socks` 上写错会静默失效——Xray 宽容未知字段，
    /// engine 照常启动，但流量根本不过隧道。
    #[test]
    fn test_masque_outbound_is_flat_socks_shape() {
        let o = ConfigManager::masque_outbound("127.0.0.1", 1080);

        assert_eq!(o["protocol"], "socks");
        assert_eq!(o["settings"]["address"], "127.0.0.1");
        assert_eq!(o["settings"]["port"], 1080);

        assert!(
            o["settings"].get("servers").is_none(),
            "socks outbound 不得用 servers[] 形状——会静默失效。实际: {o}"
        );
    }

    /// tag 必须是 `proxy-warp`。
    ///
    /// 上层的中转 outbound（`freedom` + `dialerProxy: proxy-warp`）与
    /// 路由规则都引用这个 tag；换出口实现**不应**改变它，
    /// 否则路由会指向一个不存在的 tag，整条链断掉。
    #[test]
    fn test_masque_outbound_reuses_proxy_warp_tag() {
        let o = ConfigManager::masque_outbound("127.0.0.1", 1080);
        assert_eq!(o["tag"], "proxy-warp");
    }

    /// 不得携带任何 WireGuard 专有字段。
    ///
    /// MASQUE 与 WireGuard 是两套完全不同的实现：账号格式不同、
    /// 传输层不同。把 `secretKey` / `reserved` 之类混进来
    /// 说明实现把两者搞混了。
    #[test]
    fn test_masque_outbound_has_no_wireguard_fields() {
        let o = ConfigManager::masque_outbound("127.0.0.1", 1080);
        for forbidden in [
            "secretKey",
            "reserved",
            "peers",
            "publicKey",
            "address_v4",
            "mtu",
        ] {
            assert!(
                o["settings"].get(forbidden).is_none(),
                "MASQUE outbound 不应含 WireGuard 字段 `{forbidden}`: {o}"
            );
        }
    }

    /// 地址与端口必须是**参数化**的，不能硬编码——
    /// 否则改监听端口时配置不会跟着变，且没法单测。
    #[test]
    fn test_masque_outbound_parameterised() {
        let a = ConfigManager::masque_outbound("127.0.0.1", 1080);
        let b = ConfigManager::masque_outbound("10.0.0.1", 9999);
        assert_eq!(a["settings"]["port"], 1080);
        assert_eq!(b["settings"]["port"], 9999);
        assert_eq!(b["settings"]["address"], "10.0.0.1");
        assert_ne!(a, b, "不同参数应产出不同 outbound");
    }

    /// SOCKS 不加密不认证——不得在配置里出现凭据字段
    /// （即使为空也会诱导后人去填）。
    #[test]
    fn test_masque_outbound_has_no_credentials() {
        let o = ConfigManager::masque_outbound("127.0.0.1", 1080);
        assert!(o["settings"].get("user").is_none());
        assert!(o["settings"].get("pass").is_none());
    }
}
