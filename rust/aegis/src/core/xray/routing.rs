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

/// 自定义放行域名规范化中的拒绝原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomDomainError {
    Empty,
    HasSchemeOrPath,
    HasPort,
    IpNotSupported,
    SingleLabel,
    InvalidLabel,
    TooLong,
    UnsupportedPrefix,
    NonAscii,
}

/// 判断是否为纯 IP 字面量或 CIDR（如 192.168.1.1、::1、10.0.0.0/8）。
/// 用标准库解析而非正则：`1.2.3` 这类形似 IP 的域名不会被误判。
fn is_ip_or_cidr(s: &str) -> bool {
    let (addr, mask) = match s.split_once('/') {
        Some((a, m)) => (a, Some(m)),
        None => (s, None),
    };
    if let Some(m) = mask {
        // 掩码必须是合法的 0-128 数字，否则 `youtube.com/8` 会被误判成 CIDR
        if !m.parse::<u8>().is_ok_and(|n| n <= 128) {
            return false;
        }
    }
    addr.parse::<std::net::Ipv4Addr>().is_ok() || addr.parse::<std::net::Ipv6Addr>().is_ok()
}

/// 规范化并校验用户输入的放行域名，返回可直接放进 Xray domain 数组的条目。
///
/// 顺序是刻意的：先做形态归一（小写/尾点/`*.`/`.`），再判 IP 与 scheme/路径/端口，
/// 最后才校验 label。反过来会把 `192.168.1.1`（label 合法）误报为非法 label，
/// 把 `10.0.0.0/8` 误报为带路径，把 `2001:db8::1` 误报为未知前缀。
pub fn normalize_custom_domain(input: &str) -> Result<String, CustomDomainError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CustomDomainError::Empty);
    }

    // 域名大小写不敏感，Xray 也只做小写匹配；统一小写避免用户输入大写导致静默漏匹配
    let lowered = trimmed.to_lowercase();
    // 尾点表示 FQDN，Xray domain 数组不接受，必须去掉
    let without_dot = lowered.trim_end_matches('.');
    if without_dot.is_empty() {
        return Err(CustomDomainError::Empty);
    }

    // v1 不做 punycode：转换需要新增依赖，已推迟；先明确拒绝而不是放行一个永远不匹配的域名
    if !without_dot.is_ascii() {
        return Err(CustomDomainError::NonAscii);
    }

    // `*.` 与 `.` 都表示「该域及其所有子域」，与 Xray 的 `domain:` 前缀语义重复，剥掉
    let body = without_dot.strip_prefix("*.").unwrap_or(without_dot);
    let body = body.strip_prefix('.').unwrap_or(body);

    // IP/CIDR 必须先判：IPv6 形如 2001:db8::1，若先按 “xxx:” 前缀判会被误报为未知前缀；
    // IPv4 与 CIDR 的 label 恰好合法，若先判 label 会被当成普通域名放行
    if is_ip_or_cidr(body) {
        return Err(CustomDomainError::IpNotSupported);
    }

    // scheme 判定要早于 “:” 前缀判定：`https://x` 冒号前是 https，否则会被当成未知前缀
    if body.contains("://") || body.starts_with("http:") || body.starts_with("https:") {
        return Err(CustomDomainError::HasSchemeOrPath);
    }
    // 含路径的输入不可能是纯主机名
    if body.contains('/') {
        return Err(CustomDomainError::HasSchemeOrPath);
    }

    // 前缀处理：domain: / full: 是 Xray 认可的写法，原样保留；
    // 其余 “冒号前不是主机名” 的写法（regexp:、keyword:、geosite: 等）v1 不支持，明确拒绝；
    // 冒号前含点说明是主机名带端口（youtube.com:443），归为 HasPort
    let (prefix, host) = match body.split_once(':') {
        None => ("domain:", body),
        Some(("domain", rest)) => ("domain:", rest),
        Some(("full", rest)) => ("full:", rest),
        Some((token, _)) if token.contains('.') => return Err(CustomDomainError::HasPort),
        Some(_) => return Err(CustomDomainError::UnsupportedPrefix),
    };

    // 单个 label（无点）无法用 domain: 表达放行范围，多半是用户漏了后缀（cn、localhost）
    if !host.contains('.') {
        return Err(CustomDomainError::SingleLabel);
    }

    // 逐 label 白名单校验：下划线（_dmarc）、首尾连字符、空格等一律非法
    for label in host.split('.') {
        if label.len() > 63 {
            return Err(CustomDomainError::TooLong);
        }
        if label.is_empty()
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(CustomDomainError::InvalidLabel);
        }
    }

    // 超长主机名会被 Xray 拒绝，进而导致整份配置加载失败，必须前置拦截
    if host.len() > 253 {
        return Err(CustomDomainError::TooLong);
    }

    Ok(format!("{}{}", prefix, host))
}

pub struct RoutingManager;

/// 自定义放行列表的硬上限。
/// 上限存在是因为整条规则要落进 Xray 配置：域名越多，每次 reload 的解析与内存成本越高，
/// 而这份列表只能靠用户手动逐个增删；100 条足够覆盖误伤场景，也避免配置被无限膨胀。
pub(crate) const CUSTOM_DIRECT_LIMIT: usize = 100;

/// `add_custom_direct_entry` 的结果。调用方据此区分「该写盘」与「无变更」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomAddOutcome {
    Added(String),
    AlreadyExists(String),
    LimitReached,
}

/// 追加条目（幂等）。已达上限、或条目已存在（无变更）返回 None。
///
/// 纯函数：不碰文件系统，调用方负责把 Some 的结果写盘。
/// 返回 None 而非「原样返回」，是为了让调用方能把「无变更」与「有变更」区分开，
/// 只在真正变更时才 reload 核心 —— reload 会打断用户连接，不能拿它当廉价操作。
pub(crate) fn append_unique(list: &[String], entry: &str, limit: usize) -> Option<Vec<String>> {
    if list.len() >= limit || list.iter().any(|d| d == entry) {
        return None;
    }
    let mut out = list.to_vec();
    out.push(entry.to_string());
    Some(out)
}

/// 按序号移除。越界返回 None。
///
/// 越界必须返回 None 而不是空列表：空列表在调用方那里意味着「整体移除规则」，
/// 误把越界当成清空就会静默删掉用户的全部自定义域名。
pub(crate) fn remove_at(list: &[String], idx: usize) -> Option<Vec<String>> {
    if idx >= list.len() {
        return None;
    }
    let mut out = list.to_vec();
    out.remove(idx);
    Some(out)
}

/// 主机名归一：去首尾空白 + 小写。
/// 域名匹配大小写不敏感（DNS 与 Xray 均如此），且 host 来自 HTTP 请求头、
/// 条目来自 JSON，两侧都可能带空白，不归一就会出现「配置写了却连不上」的静默漏匹配。
fn normalize_host(s: &str) -> String {
    s.trim().to_lowercase()
}

/// 单个条目与已归一 host 的匹配。
///
/// `domain:X`（以及未知形态）命中 X 与 X 的所有子域；`full:X` 仅命中 X。
fn entry_matches_host(entry: &str, host: &str) -> bool {
    let (exact_only, pattern) = match entry.split_once(':') {
        Some(("full", rest)) => (true, rest),
        Some(("domain", rest)) => (false, rest),
        // 未知形态按 domain: 处理：Xray 里裸主机名就是「含子域」语义，
        // 且 v1 已拒绝其余前缀，此处只需与 Xray 行为对齐而不是再报错
        _ => (false, entry),
    };
    let pattern = pattern.trim();
    if pattern.is_empty() {
        // `domain:` / `full:` 的空前缀若不拦，会退化成「匹配一切」，把全部流量放行
        return false;
    }
    if host == pattern {
        return true;
    }
    if exact_only {
        return false;
    }
    // 必须比 "." 边界：裸 ends_with 会让 notdecodo.cn 被 decodo.cn 命中
    host.len() > pattern.len()
        && host.ends_with(pattern)
        && host.as_bytes()[host.len() - pattern.len() - 1] == b'.'
}

/// 判断 host 是否命中 custom_direct 列表；命中则返回其下标。
/// 匹配语义：条目 domain:X 命中 X 与其所有子域；full:X 仅命中 X。
/// 同时存在两种条目时按列表顺序返回第一个命中的下标，与 Xray 首条命中即停一致。
///
/// 纯函数：不读文件、不访问网络、无副作用（host 为空返回 None）。
pub fn match_custom_direct(domains: &[String], host: &str) -> Option<usize> {
    let host = normalize_host(host);
    if host.is_empty() {
        return None;
    }
    domains
        .iter()
        .position(|entry| entry_matches_host(&normalize_host(entry), &host))
}

/// 判断 host 是否命中 connectivity_check 的 5 个静态主机名（裸主机名 = 含子域语义）。
///
/// 主机名清单从 ROUTING_RULES 现取而不另抄一份：抄两份的话，日后改规则域名时
/// 自检会静默用过时清单判断，恰恰在排查「放行了却仍被拦」时给出误导结论。
pub fn matches_connectivity_check(host: &str) -> bool {
    let host = normalize_host(host);
    if host.is_empty() {
        return false;
    }
    ROUTING_RULES
        .iter()
        .find(|r| r.id == "connectivity_check")
        .is_some_and(|rule| {
            rule.targets
                .iter()
                .any(|t| entry_matches_host(&normalize_host(t), &host))
        })
}

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

    /// 纯函数：确保 routing.rules 含 connectivity_check（位于首位，幂等）。
    /// 返回是否发生变更。不触碰文件系统，便于单测。
    ///
    /// 插在首位是必须的：Xray routing 顺序匹配、首条命中即停，
    /// 排在 cn_ip / cn_domain 之后则完全不生效。
    /// 因此规则已存在但错位时也会被提到首位（视为有变更），而不是原样放过。
    pub fn ensure_direct_rules_value(v: &mut Value) -> bool {
        const RULE_ID: &str = "connectivity_check";

        // 规范化 routing 与 routing.rules 的存在性
        if v.get("routing").map(|r| r.is_null()).unwrap_or(true) {
            v["routing"] = Value::Object(serde_json::Map::new());
        }
        if v["routing"]["rules"].as_array().is_none() {
            v["routing"]["rules"] = Value::Array(Vec::new());
        }

        let def = ROUTING_RULES
            .iter()
            .find(|r| r.id == RULE_ID)
            .expect("ROUTING_RULES 必须包含 connectivity_check");
        let canonical = Self::rule_def_to_json(def);

        let rules = v["routing"]["rules"].as_array_mut().unwrap();
        let pos = rules
            .iter()
            .position(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some(RULE_ID));

        match pos {
            // 已在首位且内容与当前定义一致：无变更，无需写盘
            Some(0) if rules[0] == canonical => false,
            // 已在首位但内容过时（例如新增了域名）：用当前定义覆盖。
            // 旧版本的迁移只插一次就不再更新，存量机器永远拿不到新增域名，
            // 因此这里必须比对内容而非只看 tag 是否存在。
            Some(0) => {
                rules[0] = canonical;
                true
            }
            // 错位：提到首位。toggle() 用 push 追加到末尾（cn_domain 之后），
            // 那里因首条命中即停而完全失效；迁移是唯一的修复路径。
            // 同时用当前定义覆盖，顺带修正过时内容。
            Some(i) => {
                rules.remove(i);
                rules.insert(0, canonical);
                true
            }
            None => {
                rules.insert(0, canonical);
                true
            }
        }
    }

    /// 构造 custom_direct 规则的 JSON（domain 数组承载用户归一化输入）。
    pub(crate) fn custom_direct_rule_json(domains: &[String]) -> Value {
        json!({
            "type": "field",
            "ruleTag": "custom_direct",
            "outboundTag": "direct",
            "domain": Value::Array(domains.iter().map(|d| Value::String(d.clone())).collect()),
        })
    }
    /// 删除指定 ruleTag 的规则（含重复条目）；返回是否发生变更。
    pub(crate) fn remove_rule_by_tag(v: &mut Value, rule_id: &str) -> bool {
        let Some(rules) = v
            .get_mut("routing")
            .and_then(|r| r.get_mut("rules"))
            .and_then(|r| r.as_array_mut())
        else {
            return false;
        };
        let before = rules.len();
        rules.retain(|r| r.get("ruleTag").and_then(|t| t.as_str()) != Some(rule_id));
        rules.len() != before
    }
    /// 把 rule 插到 anchor_tag 之后；若 rule_id 已存在则先移除再插入。
    /// anchor 不存在时插到首位。返回是否发生变更。
    pub(crate) fn upsert_after(
        v: &mut Value,
        anchor_tag: &str,
        rule_id: &str,
        rule: Value,
    ) -> bool {
        let original = v.clone();
        // 规范化容器，调用方（含迁移路径）可能拿到缺键的旧 base
        if v.get("routing").map(|r| r.is_null()).unwrap_or(true) {
            v["routing"] = Value::Object(serde_json::Map::new());
        }
        if v["routing"]["rules"].as_array().is_none() {
            v["routing"]["rules"] = Value::Array(Vec::new());
        }
        let rules = v["routing"]["rules"].as_array_mut().unwrap();
        // 先清掉同名旧规则：错位/过时内容只被「移动+覆盖」，不产生重复条目
        rules.retain(|r| r.get("ruleTag").and_then(|t| t.as_str()) != Some(rule_id));
        // 移除后重新定位 anchor：anchor 与 rule_id 相邻时索引会位移
        let insert_at = rules
            .iter()
            .position(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some(anchor_tag))
            .map(|i| i + 1)
            .unwrap_or(0);
        rules.insert(insert_at, rule);
        *v != original
    }
    /// 纯函数：确保 routing.rules 含 custom_direct（位于 connectivity_check 之后，幂等）。
    /// 空列表 = 移除既存规则。返回是否发生变更。
    ///
    /// 为什么必须紧跟 connectivity_check：Xray 顺序匹配、首条命中即停，
    /// 用户自定义域名（多为 geosite:cn 收录）排在 cn_domain 之后会被抢先 blackhole。
    /// 与 connectivity_check 同为 direct，故插在其后不扰动既有顺序断言。
    pub fn ensure_custom_direct_value(v: &mut Value, domains: &[String]) -> bool {
        const RULE_ID: &str = "custom_direct";
        const ANCHOR: &str = "connectivity_check";
        if domains.is_empty() {
            return Self::remove_rule_by_tag(v, RULE_ID);
        }
        Self::upsert_after(v, ANCHOR, RULE_ID, Self::custom_direct_rule_json(domains))
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

    /// 从 base JSON 中取出 custom_direct 规则的 domain 列表；规则不存在则为空。
    fn custom_direct_domains_from(v: &Value) -> Vec<String> {
        v["routing"]["rules"]
            .as_array()
            .and_then(|rules| {
                rules
                    .iter()
                    .find(|r| r.get("ruleTag").and_then(|t| t.as_str()) == Some("custom_direct"))
            })
            .and_then(|r| r.get("domain"))
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| d.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 把域名列表写回 base JSON 的 custom_direct 规则。
    /// 空列表由 ensure_custom_direct_value 负责整体移除规则，不在此重复实现。
    async fn persist_base_json(v: &Value, base_path: &str) -> Result<()> {
        let new_content = serde_json::to_string_pretty(v).context("序列化配置失败")?;
        tokio::fs::write(base_path, new_content)
            .await
            .context("写入 00_base.json 失败")
    }

    /// 追加一个已规范化的条目（调用方先用 normalize_custom_domain 校验）。写盘并重载核心。
    ///
    /// 仅在有真实变更时才写盘 + reload：reload 会重建核心进程、打断现有连接，
    /// 因此「重复添加」与「已达上限」都必须是无副作用的早退。
    pub async fn add_custom_direct_entry(entry: &str) -> anyhow::Result<CustomAddOutcome> {
        // 读改写全程持锁：菜单迁移（ensure_direct_rules_in_base）也在写同一个文件，
        // 交错执行会让后写的一方覆盖前者的变更。
        let _lock = CONFIG_LOCK.lock().await;
        let (mut v, base_path) = Self::read_base_json().await?;
        let current = Self::custom_direct_domains_from(&v);
        if current.iter().any(|d| d == entry) {
            return Ok(CustomAddOutcome::AlreadyExists(entry.to_string()));
        }
        let Some(next) = append_unique(&current, entry, CUSTOM_DIRECT_LIMIT) else {
            return Ok(CustomAddOutcome::LimitReached);
        };
        Self::ensure_custom_direct_value(&mut v, &next);
        Self::persist_base_json(&v, &base_path).await?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(CustomAddOutcome::Added(entry.to_string()))
    }

    /// 按序号移除；越界返回 Err 且【不写盘】。返回被移除的条目。
    pub async fn remove_custom_direct_at(idx: usize) -> anyhow::Result<String> {
        let _lock = CONFIG_LOCK.lock().await;
        let (mut v, base_path) = Self::read_base_json().await?;
        let current = Self::custom_direct_domains_from(&v);
        let Some(next) = remove_at(&current, idx) else {
            // 越界在任何写操作之前返回：菜单里的序号来自上一次渲染，
            // 期间可能已被另一个会话删空，此时按越界处理远好过删错条目。
            anyhow::bail!("序号 {} 越界（当前 {} 条）", idx, current.len());
        };
        let removed = current[idx].clone();
        Self::ensure_custom_direct_value(&mut v, &next);
        Self::persist_base_json(&v, &base_path).await?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(removed)
    }

    /// 只读：当前 custom_direct 的 domain 列表（规则不存在则返回空 vec）。不写盘、不重载。
    pub async fn list_custom_direct_domains() -> anyhow::Result<Vec<String>> {
        // 持锁只为避免与写盘并发：写盘不是原子的，无锁读可能读到被截断的半个文件。
        let _lock = CONFIG_LOCK.lock().await;
        let (v, _) = Self::read_base_json().await?;
        Ok(Self::custom_direct_domains_from(&v))
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
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["ruleTag"], "connectivity_check");
        assert_eq!(rules[0]["outboundTag"], "direct");
        assert_eq!(rules[1]["ruleTag"], "cn_ip");
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
        assert_eq!(rules.len(), 2, "移动不得产生重复条目");
        let tags: Vec<&str> = rules.iter().filter_map(|r| r["ruleTag"].as_str()).collect();
        assert_eq!(tags, vec!["connectivity_check", "cn_ip"]);
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
        assert_eq!(rules.len(), 2, "不得重复插入");
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
        assert_eq!(rules[1]["ruleTag"], "cn_ip", "其余规则不得受影响");
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

    // ── normalize_custom_domain ───────────────────────────────

    fn norm_ok(input: &str) -> String {
        normalize_custom_domain(input).unwrap_or_else(|e| panic!("{} 应通过，实际 {:?}", input, e))
    }

    fn norm_err(input: &str) -> CustomDomainError {
        normalize_custom_domain(input).expect_err(input)
    }

    /// 6 种接受形态，外加大小写/空白/尾点归一化。
    #[test]
    fn test_normalize_custom_domain_accepts() {
        for (input, expected) in [
            ("youtube.com", "domain:youtube.com"),
            ("www.youtube.com", "domain:www.youtube.com"),
            ("*.youtube.com", "domain:youtube.com"),
            (".youtube.com", "domain:youtube.com"),
            ("domain:youtube.com", "domain:youtube.com"),
            ("full:youtube.com", "full:youtube.com"),
            ("YOUTUBE.COM", "domain:youtube.com"),
            ("youtube.com.", "domain:youtube.com"),
            ("  YOUTUBE.COM  ", "domain:youtube.com"),
        ] {
            assert_eq!(norm_ok(input), expected, "输入 {input}");
        }
    }

    /// 9 类拒绝，逐条对齐验收标准。
    #[test]
    fn test_normalize_custom_domain_rejects() {
        for (input, expected) in [
            (
                "https://youtube.com/path",
                CustomDomainError::HasSchemeOrPath,
            ),
            ("youtube.com/path", CustomDomainError::HasSchemeOrPath),
            ("youtube.com:443", CustomDomainError::HasPort),
            ("192.168.1.1", CustomDomainError::IpNotSupported),
            ("10.0.0.0/8", CustomDomainError::IpNotSupported),
            ("", CustomDomainError::Empty),
            ("   ", CustomDomainError::Empty),
            ("a b.com", CustomDomainError::InvalidLabel),
            ("_dmarc.youtube.com", CustomDomainError::InvalidLabel),
            ("-foo.com", CustomDomainError::InvalidLabel),
            ("foo-.com", CustomDomainError::InvalidLabel),
            ("localhost", CustomDomainError::SingleLabel),
            ("cn", CustomDomainError::SingleLabel),
            ("regexp:x", CustomDomainError::UnsupportedPrefix),
            ("keyword:x", CustomDomainError::UnsupportedPrefix),
            ("geosite:cn", CustomDomainError::UnsupportedPrefix),
            ("中文.com", CustomDomainError::NonAscii),
        ] {
            assert_eq!(norm_err(input), expected, "输入 {input}");
        }
    }

    /// 254 字符整体与 64 字符单 label 都必须以 TooLong 拒绝。
    #[test]
    fn test_normalize_custom_domain_rejects_length_overflow() {
        let too_long = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(62)
        );
        assert_eq!(too_long.len(), 254, "构造的输入必须正好 254 字符");
        assert_eq!(norm_err(&too_long), CustomDomainError::TooLong);
        let overlong_label = format!("{}.com", "a".repeat(64));
        assert_eq!(norm_err(&overlong_label), CustomDomainError::TooLong);
    }
    // ── custom_direct 插入 / 迁移 / 删除（纯函数） ───────────────
    fn cd_domains() -> Vec<String> {
        vec!["domain:decodo.cn".to_string()]
    }
    fn cd_rule(domains: &[String]) -> Value {
        RoutingManager::custom_direct_rule_json(domains)
    }
    // 短名转调：长签名调用会被 rustfmt 拆行，收敛为短调用以保持测试紧凑。
    fn ensure_cd(v: &mut Value, domains: &[String]) -> bool {
        RoutingManager::ensure_custom_direct_value(v, domains)
    }
    fn remove_cd(v: &mut Value) -> bool {
        RoutingManager::remove_rule_by_tag(v, "custom_direct")
    }
    fn upsert_cd(v: &mut Value, anchor: &str, rule: Value) -> bool {
        RoutingManager::upsert_after(v, anchor, "custom_direct", rule)
    }
    fn tags_of(v: &Value) -> Vec<String> {
        v["routing"]["rules"].as_array().map_or(Vec::new(), |rs| {
            rs.iter()
                .filter_map(|r| r["ruleTag"].as_str().map(str::to_string))
                .collect()
        })
    }
    fn tag_index(v: &Value, tag: &str) -> usize {
        tags_of(v)
            .iter()
            .position(|t| t == tag)
            .unwrap_or_else(|| panic!("缺少规则 {tag}"))
    }
    fn direct_chain() -> Value {
        base_with_rules(json!([
            {"type":"field","ruleTag":"connectivity_check","outboundTag":"direct","domain":["www.gstatic.com"]},
            {"type":"field","ruleTag":"private_ip","outboundTag":"blocked","ip":["geoip:private"]},
            {"type":"field","ruleTag":"cn_ip","outboundTag":"blocked","ip":["geoip:cn"]},
            {"type":"field","ruleTag":"cn_domain","outboundTag":"blocked","domain":["geosite:cn"]}
        ]))
    }
    #[test]
    fn test_custom_direct_rule_json_shape() {
        let j = cd_rule(&cd_domains());
        assert_eq!(j["type"], "field");
        assert_eq!(j["ruleTag"], "custom_direct");
        assert_eq!(j["outboundTag"], "direct");
        assert_eq!(j["domain"], json!(["domain:decodo.cn"]));
        let multi = cd_rule(&["a.cn".into(), "b.cn".into()]);
        assert_eq!(multi["domain"], json!(["a.cn", "b.cn"]));
    }
    // 索引 + 显式回归：必须紧跟 connectivity_check，且早于 private_ip / cn_ip / cn_domain。
    // 顺序颠倒（尤其排在 cn_domain 之后）会被 geosite:cn 抢先 blackhole，规则形同不存在。
    #[test]
    fn test_custom_direct_index_precedes_cn_domain_regression() {
        let mut v = direct_chain();
        assert!(ensure_cd(&mut v, &cd_domains()));
        let cd = tag_index(&v, "custom_direct");
        assert_eq!(cd, tag_index(&v, "connectivity_check") + 1);
        assert!(cd < tag_index(&v, "private_ip"));
        assert!(cd < tag_index(&v, "cn_ip"));
        assert!(cd < tag_index(&v, "cn_domain"));
    }
    // 幂等 + 空列表移除；缺失 routing / routing.rules / routing=null 不 panic，空输入不建容器。
    #[test]
    fn test_custom_direct_idempotent_empty_list_and_missing_containers() {
        let mut v = direct_chain();
        assert!(!ensure_cd(&mut v, &[]));
        assert!(ensure_cd(&mut v, &cd_domains()));
        let snapshot = v.clone();
        assert!(!ensure_cd(&mut v, &cd_domains()));
        assert_eq!(v, snapshot);
        assert!(ensure_cd(&mut v, &[]));
        assert!(!tags_of(&v).iter().any(|t| t == "custom_direct"));
        for mut m in [json!({}), json!({"routing": {}}), json!({"routing": null})] {
            assert!(ensure_cd(&mut m, &cd_domains()));
            let tag = m["routing"]["rules"][0]["ruleTag"].clone();
            assert_eq!(tag, "custom_direct");
        }
        let mut empty = json!({});
        assert!(!ensure_cd(&mut empty, &[]));
        assert_eq!(empty, json!({}));
    }
    // 过时覆盖 + 错位移动（不重复）+ 既有规则相对顺序不变 + connectivity_check 不被改动。
    #[test]
    fn test_custom_direct_overwrites_moves_and_preserves_order() {
        let mut v = base_with_rules(json!([
            {"type":"field","ruleTag":"connectivity_check","outboundTag":"direct","domain":["old.example.com"]},
            {"type":"field","ruleTag":"custom_direct","outboundTag":"direct","domain":["domain:stale.cn"]},
            {"type":"field","ruleTag":"private_ip","outboundTag":"blocked","ip":["geoip:private"]},
            {"type":"field","ruleTag":"cn_ip","outboundTag":"blocked","ip":["geoip:cn"]},
            {"type":"field","ruleTag":"cn_domain","outboundTag":"blocked","domain":["geosite:cn"]}
        ]));
        assert!(ensure_cd(&mut v, &cd_domains()));
        let rules = v["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 5);
        let overwritten = rules[1]["domain"].clone();
        assert_eq!(overwritten, json!(["domain:decodo.cn"]));
        assert_eq!(rules[0]["domain"], json!(["old.example.com"]));
        let others: Vec<String> = tags_of(&v)
            .into_iter()
            .filter(|t| t != "custom_direct")
            .collect();
        let want = "connectivity_check,private_ip,cn_ip,cn_domain";
        assert_eq!(others.join(","), want);
        let mut mis = base_with_rules(json!([
            {"type":"field","ruleTag":"connectivity_check","outboundTag":"direct","domain":["www.gstatic.com"]},
            {"type":"field","ruleTag":"cn_domain","outboundTag":"blocked","domain":["geosite:cn"]},
            {"type":"field","ruleTag":"custom_direct","outboundTag":"direct","domain":["domain:old.cn"]}
        ]));
        assert!(ensure_cd(&mut mis, &cd_domains()));
        assert_eq!(tag_index(&mis, "custom_direct"), 1);
        let count = tags_of(&mis)
            .iter()
            .filter(|t| *t == "custom_direct")
            .count();
        assert_eq!(count, 1);
    }
    #[test]
    fn test_remove_and_upsert_after_report_change() {
        let mut v = direct_chain();
        assert!(!remove_cd(&mut v));
        let mut empty = json!({});
        assert!(!remove_cd(&mut empty));
        let rule = cd_rule(&cd_domains());
        let r2 = rule.clone();
        assert!(upsert_cd(&mut v, "connectivity_check", r2));
        assert_eq!(tag_index(&v, "custom_direct"), 1);
        assert!(!upsert_cd(&mut v, "connectivity_check", rule));
        assert!(remove_cd(&mut v));
        assert!(!tags_of(&v).iter().any(|t| t == "custom_direct"));
        let mut orphan = base_with_rules(json!([
            {"type":"field","ruleTag":"cn_domain","outboundTag":"blocked","domain":["geosite:cn"]}
        ]));
        let r3 = cd_rule(&cd_domains());
        assert!(upsert_cd(&mut orphan, "connectivity_check", r3));
        assert_eq!(tag_index(&orphan, "custom_direct"), 0);
    }

    // ── append_unique / remove_at（I/O 薄封装的纯逻辑层） ───────────
    //
    // 只测这两个纯函数：00_base.json 是硬编码生产路径，单测写它会污染部署机。
    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }
    fn many(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("domain:d{i}.cn")).collect()
    }

    #[test]
    fn test_append_unique_grows_list_without_mutating_input() {
        let base = strs(&["domain:a.cn"]);
        let out = append_unique(&base, "domain:b.cn", 100).expect("未达上限应返回新列表");
        assert_eq!(out, strs(&["domain:a.cn", "domain:b.cn"]));
        assert_eq!(base, strs(&["domain:a.cn"]), "不得就地修改入参");
    }

    #[test]
    fn test_append_unique_is_idempotent_for_duplicate() {
        let base = strs(&["domain:a.cn", "domain:b.cn"]);
        assert_eq!(
            append_unique(&base, "domain:a.cn", 100),
            None,
            "重复条目应返回 None（无变更，调用方据此不写盘）"
        );
    }

    #[test]
    fn test_append_unique_limit_boundary() {
        let out = append_unique(&many(99), "domain:last.cn", 100).expect("第 100 条应允许追加");
        assert_eq!(out.len(), 100);
        assert_eq!(
            out[99], "domain:last.cn",
            "追加必须落在末尾以保持用户可见顺序"
        );
        assert_eq!(
            append_unique(&many(100), "domain:overflow.cn", 100),
            None,
            "第 101 条必须被上限拒绝"
        );
    }

    #[test]
    fn test_append_unique_honours_passed_limit() {
        let base = strs(&["domain:a.cn"]);
        assert_eq!(append_unique(&base, "domain:b.cn", 1), None);
        assert!(append_unique(&[], "domain:a.cn", 1).is_some());
    }

    #[test]
    fn test_remove_at_removes_and_keeps_remaining_order() {
        let base = strs(&["domain:a.cn", "domain:b.cn", "domain:c.cn"]);
        assert_eq!(
            remove_at(&base, 1),
            Some(strs(&["domain:a.cn", "domain:c.cn"]))
        );
        assert_eq!(
            remove_at(&base, 0),
            Some(strs(&["domain:b.cn", "domain:c.cn"]))
        );
        assert_eq!(
            remove_at(&base, 2),
            Some(strs(&["domain:a.cn", "domain:b.cn"]))
        );
        assert_eq!(base.len(), 3, "不得就地修改入参");
    }

    #[test]
    fn test_remove_at_last_element_yields_empty_list() {
        // 空列表是「整体移除规则」的信号，必须能表示出来而不是当成越界
        assert_eq!(remove_at(&strs(&["domain:a.cn"]), 0), Some(Vec::new()));
    }

    #[test]
    fn test_remove_at_out_of_range_returns_none() {
        let base = strs(&["domain:a.cn"]);
        assert_eq!(remove_at(&base, 1), None);
        assert_eq!(remove_at(&base, usize::MAX), None);
        assert_eq!(remove_at(&[], 0), None, "空列表任何序号都越界");
    }

    // ── match_custom_direct / matches_connectivity_check（纯函数自检判定） ──

    /// domain:X 命中 X 本身与所有子域；断言同时覆盖真正的边界
    /// （X 出现在串尾但不是子域时必须不命中）。
    #[test]
    fn test_match_custom_direct_domain_prefix_matches_apex_and_subdomains() {
        let list = strs(&["domain:decodo.cn"]);
        assert_eq!(match_custom_direct(&list, "decodo.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "dashboard.decodo.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "a.b.decodo.cn"), Some(0));
        // 关键边界：后缀相同但不是子域，必须比 "." 边界而非裸 ends_with
        assert_eq!(match_custom_direct(&list, "notdecodo.cn"), None);
        assert_eq!(match_custom_direct(&list, "decodo.cn.evil.com"), None);
        assert_eq!(match_custom_direct(&list, "decodo.com"), None);
    }

    #[test]
    fn test_match_custom_direct_full_prefix_matches_exact_only() {
        let list = strs(&["full:decodo.cn"]);
        assert_eq!(match_custom_direct(&list, "decodo.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "dashboard.decodo.cn"), None);
    }

    /// 同时存在 domain:/full: 时，按列表顺序返回第一个命中的下标。
    #[test]
    fn test_match_custom_direct_returns_first_matching_index() {
        let list = strs(&[
            "full:other.cn",
            "domain:decodo.cn",
            "full:decodo.cn",
            "domain:decodo.cn",
        ]);
        assert_eq!(match_custom_direct(&list, "decodo.cn"), Some(1));
        assert_eq!(match_custom_direct(&list, "x.decodo.cn"), Some(1));
        assert_eq!(match_custom_direct(&list, "other.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "none.cn"), None);
    }

    #[test]
    fn test_match_custom_direct_empty_list_and_empty_host() {
        assert_eq!(match_custom_direct(&[], "decodo.cn"), None);
        let list = strs(&["domain:decodo.cn"]);
        assert_eq!(match_custom_direct(&list, ""), None);
        assert_eq!(match_custom_direct(&list, "   "), None);
    }

    /// host 与条目两侧的空白/大小写都要归一；条目出自 JSON，形态不受本函数控制。
    #[test]
    fn test_match_custom_direct_normalizes_case_and_whitespace() {
        let list = strs(&["  DOMAIN:Decodo.CN  "]);
        assert_eq!(match_custom_direct(&list, "DECODO.CN"), Some(0));
        assert_eq!(
            match_custom_direct(&list, "  Dashboard.Decodo.cn "),
            Some(0)
        );
        assert_eq!(match_custom_direct(&list, "NOTDECODO.CN"), None);
    }

    /// 未知形态（裸主机名）按 domain: 语义处理，与 Xray 裸主机名条目一致。
    #[test]
    fn test_match_custom_direct_bare_entry_uses_domain_semantics() {
        let list = strs(&["decodo.cn", "full:", "domain:"]);
        assert_eq!(match_custom_direct(&list, "decodo.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "dashboard.decodo.cn"), Some(0));
        assert_eq!(match_custom_direct(&list, "notdecodo.cn"), None);
        // 空前缀（"full:" / "domain:"）不得匹配任何 host，避免空串通配
        assert_eq!(match_custom_direct(&list, "anything.cn"), None);
    }

    #[test]
    fn test_matches_connectivity_check_hosts_and_subdomains() {
        for host in [
            "www.gstatic.com",
            "connectivitycheck.gstatic.com",
            "ssl.gstatic.com",
            "fonts.gstatic.com",
            "fonts.googleapis.com",
        ] {
            assert!(matches_connectivity_check(host), "{host} 必须命中");
            assert!(
                matches_connectivity_check(&host.to_uppercase()),
                "{host} 大写形态必须命中"
            );
        }
        // 裸主机名等价于 domain: —— 子域也要命中
        assert!(matches_connectivity_check("sub.www.gstatic.com"));
        assert!(matches_connectivity_check("a.b.fonts.googleapis.com"));
    }

    #[test]
    fn test_matches_connectivity_check_rejects_other_hosts() {
        for host in [
            "decodo.cn",
            "",
            "   ",
            "notwww.gstatic.com",
            "gstatic.com",
            "example.com",
        ] {
            assert!(!matches_connectivity_check(host), "{host} 不应命中");
        }
    }
}
