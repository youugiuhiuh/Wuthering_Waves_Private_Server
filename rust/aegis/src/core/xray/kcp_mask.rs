use rand::Rng;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub enum KcpMask {
    MkcpLegacy {
        header: Option<String>,
        value: Option<String>,
    },
    Noise,
    Salamander {
        password: String,
        packet_size: Option<(i32, i32)>,
    },
    Sudoku {
        password: String,
    },
    Xdns {
        domains: Vec<String>,
        resolvers: Vec<String>,
    },
    Xicmp {
        dgram: bool,
        ips: Vec<String>,
    },
    Realm {
        url: String,
        stun_servers: Vec<String>,
    },
}

pub(crate) fn generate_aes_password() -> String {
    let rng_len = rand::thread_rng().gen_range(16..32);
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(rng_len)
        .map(char::from)
        .collect()
}

impl KcpMask {
    pub fn type_str(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy { .. } => "mkcp-legacy",
            KcpMask::Noise => "noise",
            KcpMask::Salamander { .. } => "salamander",
            KcpMask::Sudoku { .. } => "sudoku",
            KcpMask::Xdns { .. } => "xdns",
            KcpMask::Xicmp { .. } => "xicmp",
            KcpMask::Realm { .. } => "realm",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            } => "🔀 mKCP Original (XOR)",
            KcpMask::MkcpLegacy { header: None, .. } => "🔐 mKCP AES-128-GCM",
            KcpMask::MkcpLegacy {
                header: Some(h), ..
            } => match h.as_str() {
                "dns" => "🌐 mKCP + DNS伪装",
                "wechat" => "💬 mKCP + 微信伪装",
                "srtp" => "🎬 mKCP + SRTP伪装",
                "utp" => "🔗 mKCP + uTP伪装",
                "dtls" => "🔒 mKCP + DTLS伪装",
                "wireguard" => "🛡️ mKCP + WireGuard伪装",
                _ => "🔐 mKCP",
            },
            KcpMask::Noise => "📊 Noise",
            KcpMask::Salamander { .. } => "🦎 Salamander",
            KcpMask::Sudoku { .. } => "🔢 Sudoku",
            KcpMask::Xdns { .. } => "📡 XDNS 扩展DNS",
            KcpMask::Xicmp { .. } => "💓 XICMP",
            KcpMask::Realm { .. } => "🕳️ Realm",
        }
    }

    pub fn detail(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            } => "无加密，仅FNV1a校验。建议配合混淆层使用",
            KcpMask::MkcpLegacy { header: None, .. } => {
                "AES-128-GCM端到端认证加密，密码经SHA256派生为128位密钥"
            }
            KcpMask::MkcpLegacy {
                header: Some(h), ..
            } => match h.as_str() {
                "dns" => "DNS查询流量伪装，使用指定域名构建DNS请求头部",
                "wechat" => "微信视频通话流量伪装，数据包模拟微信VoIP协议格式",
                "srtp" => "安全实时传输协议(SRTP)流量伪装，看起来像音视频流媒体",
                "utp" => "BitTorrent uTP协议伪装，数据包模拟uTP格式",
                "dtls" => "DTLS 1.2加密数据包伪装，流量看起来像正常UDP通信",
                "wireguard" => "WireGuard VPN流量伪装，数据包模拟WireGuard协议格式",
                _ => "mKCP混淆协议",
            },
            KcpMask::Noise => {
                "随机噪声填充。在数据包中注入随机长度的噪声数据，有效抵抗基于包大小的流量分析。不提供加密功能，建议与加密层叠加使用"
            }
            KcpMask::Salamander { .. } => {
                "蝾螈混淆协议。使用密码派生的混淆变换，可抵抗深度包检测(DPI)。与Hysteria2的Salamander混淆采用相同算法。建议与加密层叠加使用，支持packetSize(Gecko)"
            }
            KcpMask::Sudoku { .. } => {
                "数独混淆算法。基于密码派生的混淆，包含ASCII混淆和随机填充，混淆强度高于Salamander，性能开销略大"
            }
            KcpMask::Xdns { .. } => {
                "扩展DNS伪装。支持AAAA/A记录类型，自定义域名列表和DNS解析器，提供比HeaderDns更灵活的DNS流量模拟。适合需要精确控制DNS伪装行为的场景"
            }
            KcpMask::Xicmp { .. } => {
                "ICMP数据包伪装。将数据包封装为ICMP回显请求/应答格式。适合仅允许ping流量通过的极端限制网络。支持dgram模式和multi-ips"
            }
            KcpMask::Realm { .. } => "UDP打洞(Hysteria)。通过STUN服务器建立UDP隧道，实现NAT穿透",
        }
    }

    pub fn brief(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            } => "XOR混淆，仅FNV1a校验",
            KcpMask::MkcpLegacy { header: None, .. } => "AES-128-GCM认证加密",
            KcpMask::MkcpLegacy {
                header: Some(h), ..
            } => match h.as_str() {
                "dns" => "加密+DNS头部伪装",
                "wechat" => "加密+微信头部伪装",
                "srtp" => "加密+SRTP头部伪装",
                "utp" => "加密+uTP头部伪装",
                "dtls" => "加密+DTLS头部伪装",
                "wireguard" => "加密+WireGuard头部伪装",
                _ => "mKCP混淆",
            },
            KcpMask::Noise => "随机噪声填充，抗流量分析",
            KcpMask::Salamander { .. } => "蝾螈混淆+可选packetSize",
            KcpMask::Sudoku { .. } => "数独混淆算法，强度更高",
            KcpMask::Xdns { .. } => "扩展DNS，AAAA/A记录支持",
            KcpMask::Xicmp { .. } => "ICMP伪装，dgram模式支持",
            KcpMask::Realm { .. } => "UDP打洞，STUN服务器",
        }
    }

    pub fn category_code(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy { .. } => "enc",
            KcpMask::Noise | KcpMask::Salamander { .. } | KcpMask::Sudoku { .. } => "obf",
            KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } | KcpMask::Realm { .. } => "ext",
        }
    }

    pub fn category(&self) -> &'static str {
        match self.category_code() {
            "enc" => "🔐 加密层",
            "obf" => "🌀 混淆层",
            "ext" => "⚡ 扩展层",
            _ => "未知",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            } => "ml",
            KcpMask::MkcpLegacy { header: None, .. } => "mla",
            KcpMask::MkcpLegacy {
                header: Some(h), ..
            } => match h.as_str() {
                "dns" => "mld",
                "wechat" => "mlw",
                "srtp" => "mls",
                "utp" => "mlu",
                "dtls" => "mldt",
                "wireguard" => "mlg",
                _ => "ml",
            },
            KcpMask::Noise => "no",
            KcpMask::Salamander { .. } => "sa",
            KcpMask::Sudoku { .. } => "su",
            KcpMask::Xdns { .. } => "xd",
            KcpMask::Xicmp { .. } => "xi",
            KcpMask::Realm { .. } => "rl",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "ml" => Some(KcpMask::MkcpLegacy {
                header: None,
                value: None,
            }),
            "mla" => Some(KcpMask::MkcpLegacy {
                header: None,
                value: Some(generate_aes_password()),
            }),
            "mld" => Some(KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: Some("www.baidu.com".into()),
            }),
            "mlw" => Some(KcpMask::MkcpLegacy {
                header: Some("wechat".into()),
                value: None,
            }),
            "mls" => Some(KcpMask::MkcpLegacy {
                header: Some("srtp".into()),
                value: None,
            }),
            "mlu" => Some(KcpMask::MkcpLegacy {
                header: Some("utp".into()),
                value: None,
            }),
            "mldt" => Some(KcpMask::MkcpLegacy {
                header: Some("dtls".into()),
                value: None,
            }),
            "mlg" => Some(KcpMask::MkcpLegacy {
                header: Some("wireguard".into()),
                value: None,
            }),
            "no" => Some(KcpMask::Noise),
            "sa" => Some(KcpMask::Salamander {
                password: generate_aes_password(),
                packet_size: None,
            }),
            "su" => Some(KcpMask::Sudoku {
                password: generate_aes_password(),
            }),
            "xd" => Some(KcpMask::Xdns {
                domains: vec!["www.baidu.com".into()],
                resolvers: vec!["www.baidu.com+udp://1.1.1.1:53".into()],
            }),
            "xi" => Some(KcpMask::Xicmp {
                dgram: false,
                ips: vec![],
            }),
            "rl" => Some(KcpMask::Realm {
                url: String::new(),
                stun_servers: vec![],
            }),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Value {
        match self {
            KcpMask::MkcpLegacy { header, value } => {
                if header.is_none() && value.is_none() {
                    json!({"type": "mkcp-legacy"})
                } else {
                    let mut settings = serde_json::Map::new();
                    if let Some(h) = header {
                        settings.insert("header".to_string(), Value::String(h.clone()));
                    }
                    if let Some(v) = value {
                        settings.insert("value".to_string(), Value::String(v.clone()));
                    }
                    json!({"type": "mkcp-legacy", "settings": Value::Object(settings)})
                }
            }
            KcpMask::Noise => json!({"type": "noise"}),
            KcpMask::Salamander {
                password,
                packet_size,
            } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), Value::String("salamander".to_string()));
                let mut settings = serde_json::Map::new();
                settings.insert("password".to_string(), Value::String(password.clone()));
                if let Some((from, to)) = packet_size {
                    settings.insert("packetSize".to_string(), json!({"from": from, "to": to}));
                }
                map.insert("settings".to_string(), Value::Object(settings));
                Value::Object(map)
            }
            KcpMask::Sudoku { password } => {
                json!({"type": "sudoku", "settings": { "password": password }})
            }
            KcpMask::Xdns { domains, resolvers } => {
                json!({"type": "xdns", "settings": { "domains": domains, "resolvers": resolvers }})
            }
            KcpMask::Xicmp { dgram, ips } => {
                if *dgram || !ips.is_empty() {
                    json!({"type": "xicmp", "settings": { "dgram": dgram, "ips": ips }})
                } else {
                    json!({"type": "xicmp"})
                }
            }
            KcpMask::Realm { url, stun_servers } => {
                json!({"type": "realm", "settings": { "url": url, "stunServers": stun_servers }})
            }
        }
    }

    pub fn all_variants() -> Vec<Self> {
        vec![
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            },
            KcpMask::MkcpLegacy {
                header: None,
                value: Some("default".into()),
            },
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: Some("www.baidu.com".into()),
            },
            KcpMask::MkcpLegacy {
                header: Some("wechat".into()),
                value: None,
            },
            KcpMask::MkcpLegacy {
                header: Some("srtp".into()),
                value: None,
            },
            KcpMask::MkcpLegacy {
                header: Some("utp".into()),
                value: None,
            },
            KcpMask::MkcpLegacy {
                header: Some("dtls".into()),
                value: None,
            },
            KcpMask::MkcpLegacy {
                header: Some("wireguard".into()),
                value: None,
            },
            KcpMask::Noise,
            KcpMask::Salamander {
                password: String::new(),
                packet_size: None,
            },
            KcpMask::Sudoku {
                password: String::new(),
            },
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![],
            },
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![],
            },
            KcpMask::Realm {
                url: String::new(),
                stun_servers: vec![],
            },
        ]
    }

    pub fn parse_codes(mask_codes: &[&str]) -> Result<Vec<Self>, String> {
        let mut masks = Vec::new();
        for code in mask_codes {
            let mask =
                Self::from_code(code).ok_or_else(|| format!("Invalid mask code: {}", code))?;
            masks.push(mask);
        }
        Ok(masks)
    }

    pub fn variants_by_category(code: &str) -> Vec<Self> {
        Self::all_variants()
            .into_iter()
            .filter(|m| m.category_code() == code)
            .collect()
    }

    pub fn category_from_code(code: &str) -> Option<&'static str> {
        match code {
            "enc" => Some("🔐 加密层"),
            "obf" => Some("🌀 混淆层"),
            "ext" => Some("⚡ 扩展层"),
            _ => None,
        }
    }

    pub fn is_encryption(&self) -> bool {
        matches!(self, KcpMask::MkcpLegacy { .. })
    }

    pub fn is_sudoku(&self) -> bool {
        matches!(self, KcpMask::Sudoku { .. })
    }

    pub fn is_transport_replacement(&self) -> bool {
        matches!(
            self,
            KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } | KcpMask::Realm { .. }
        )
    }

    pub fn is_xdns(&self) -> bool {
        matches!(self, KcpMask::Xdns { .. })
    }

    pub fn is_xicmp(&self) -> bool {
        matches!(self, KcpMask::Xicmp { .. })
    }

    pub fn header_size(&self) -> Option<usize> {
        match self {
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            } => Some(6),
            KcpMask::MkcpLegacy { header: None, .. } => Some(28),
            KcpMask::MkcpLegacy {
                header: Some(h),
                value,
            } => match h.as_str() {
                "dns" => Some(Self::dns_header_size(
                    value.as_deref().unwrap_or("www.baidu.com"),
                )),
                "wechat" => Some(13),
                "srtp" => Some(4),
                "utp" => Some(4),
                "dtls" => Some(13),
                "wireguard" => Some(4),
                _ => Some(4),
            },
            KcpMask::Salamander { .. } => Some(8),
            KcpMask::Noise => None,
            KcpMask::Sudoku { .. } => None,
            KcpMask::Xdns { .. } => None,
            KcpMask::Xicmp { .. } => None,
            KcpMask::Realm { .. } => None,
        }
    }

    fn dns_header_size(domain: &str) -> usize {
        let mut size = 12;
        for label in domain.split('.') {
            size += 1 + label.len();
        }
        size += 1;
        size += 4;
        size
    }

    fn sort_priority(&self) -> u8 {
        match self {
            KcpMask::Sudoku { .. } => 0,
            KcpMask::MkcpLegacy { header: None, .. } => 10,
            KcpMask::MkcpLegacy { .. } => 20,
            KcpMask::Salamander { .. } => 30,
            KcpMask::Noise => 40,
            KcpMask::Xdns { .. } => 50,
            KcpMask::Xicmp { .. } => 60,
            KcpMask::Realm { .. } => 70,
        }
    }

    pub fn canonical_order(masks: &[KcpMask]) -> Vec<KcpMask> {
        let mut ordered: Vec<KcpMask> = masks.to_vec();
        ordered.sort_by_key(|m| m.sort_priority());
        ordered
    }

    pub fn is_compatible_with(&self, existing: &[KcpMask]) -> Result<(), String> {
        if self.is_transport_replacement() && existing.iter().any(|m| m.is_transport_replacement())
        {
            let name = if self.is_xdns() {
                "XDNS"
            } else if matches!(self, KcpMask::Xicmp { .. }) {
                "XICMP"
            } else {
                "Realm"
            };
            return Err(format!("{}和其他传输层不能同时使用", name));
        }

        if self.is_encryption() && existing.iter().any(|m| m.is_encryption()) {
            return Err("重复的加密层".to_string());
        }

        if self.is_sudoku() && existing.iter().any(|m| m.is_sudoku()) {
            return Err("重复的Sudoku".to_string());
        }

        if existing.iter().any(|m| m.code() == self.code()) {
            return Err(format!("重复的{}", self.display_name()));
        }

        let total_header: usize = existing
            .iter()
            .filter_map(|m| m.header_size())
            .sum::<usize>()
            + self.header_size().unwrap_or(0);
        let sudoku_reserve = if self.is_sudoku() || existing.iter().any(|m| m.is_sudoku()) {
            2400
        } else {
            0
        };
        if total_header + sudoku_reserve > 3800 {
            return Err(format!(
                "header总大小{}字节过大，可能超出UDP包限制(4096字节)",
                total_header
            ));
        }

        Ok(())
    }

    pub fn validate_stack(masks: &[KcpMask]) -> Result<(), String> {
        if masks.is_empty() {
            return Err("请至少选择1层遮罩".to_string());
        }
        let has_xdns = masks.iter().any(|m| m.is_xdns());
        let has_xicmp = masks.iter().any(|m| matches!(m, KcpMask::Xicmp { .. }));
        let has_realm = masks.iter().any(|m| matches!(m, KcpMask::Realm { .. }));

        if has_xdns && has_xicmp {
            return Err("XDNS和XICMP不能同时使用".to_string());
        }
        if has_xdns && has_realm {
            return Err("XDNS和Realm不能同时使用".to_string());
        }
        if has_xicmp && has_realm {
            return Err("XICMP和Realm不能同时使用".to_string());
        }
        if masks.iter().filter(|m| m.is_encryption()).count() > 1 {
            return Err("重复的加密层".to_string());
        }
        if masks.iter().filter(|m| m.is_sudoku()).count() > 1 {
            return Err("重复的Sudoku".to_string());
        }
        let total_header: usize = masks.iter().filter_map(|m| m.header_size()).sum();
        let sudoku_reserve = if masks.iter().any(|m| m.is_sudoku()) {
            2400
        } else {
            0
        };
        if total_header + sudoku_reserve > 3800 {
            return Err(format!(
                "header总大小{}字节过大，可能超出UDP包限制(4096字节)",
                total_header
            ));
        }
        Ok(())
    }

    pub fn get_stack_warnings(masks: &[KcpMask]) -> Vec<String> {
        let mut warnings = Vec::new();
        if masks.len() == 1
            && matches!(
                masks[0],
                KcpMask::MkcpLegacy {
                    header: None,
                    value: None
                }
            )
        {
            warnings.push("⚠️ mKCP Original 单独使用安全性低，建议配合混淆层使用".to_string());
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mkcp_legacy_as_json() {
        let json = KcpMask::MkcpLegacy {
            header: None,
            value: None,
        }
        .as_json();
        assert_eq!(json["type"], "mkcp-legacy");
        assert!(json.get("settings").is_none());

        let json = KcpMask::MkcpLegacy {
            header: None,
            value: Some("pwd".into()),
        }
        .as_json();
        assert_eq!(json["type"], "mkcp-legacy");
        assert_eq!(json["settings"]["value"], "pwd");
        assert!(json["settings"].get("header").is_none());

        let json = KcpMask::MkcpLegacy {
            header: Some("dns".into()),
            value: Some("example.com".into()),
        }
        .as_json();
        assert_eq!(json["type"], "mkcp-legacy");
        assert_eq!(json["settings"]["header"], "dns");
        assert_eq!(json["settings"]["value"], "example.com");

        let json = KcpMask::MkcpLegacy {
            header: Some("wechat".into()),
            value: None,
        }
        .as_json();
        assert_eq!(json["type"], "mkcp-legacy");
        assert_eq!(json["settings"]["header"], "wechat");
        assert!(json["settings"].get("value").is_none());
    }

    #[test]
    fn test_salamander_with_packet_size() {
        let json = KcpMask::Salamander {
            password: "obfs".into(),
            packet_size: Some((512, 1200)),
        }
        .as_json();
        assert_eq!(json["type"], "salamander");
        assert_eq!(json["settings"]["password"], "obfs");
        assert_eq!(json["settings"]["packetSize"]["from"], 512);
        assert_eq!(json["settings"]["packetSize"]["to"], 1200);

        let json_no_ps = KcpMask::Salamander {
            password: "obfs".into(),
            packet_size: None,
        }
        .as_json();
        assert_eq!(json_no_ps["type"], "salamander");
        assert_eq!(json_no_ps["settings"]["password"], "obfs");
        assert!(json_no_ps["settings"].get("packetSize").is_none());
    }

    #[test]
    fn test_xdns_new_format() {
        let json = KcpMask::Xdns {
            domains: vec!["example.com:aaaa".into()],
            resolvers: vec!["example.com:aaaa+udp://1.1.1.1:53".into()],
        }
        .as_json();
        assert_eq!(json["type"], "xdns");
        assert_eq!(json["settings"]["domains"][0], "example.com:aaaa");
        assert_eq!(
            json["settings"]["resolvers"][0],
            "example.com:aaaa+udp://1.1.1.1:53"
        );
    }

    #[test]
    fn test_xicmp_new_format() {
        let json = KcpMask::Xicmp {
            dgram: false,
            ips: vec![],
        }
        .as_json();
        assert_eq!(json["type"], "xicmp");
        assert!(json.get("settings").is_none());

        let json = KcpMask::Xicmp {
            dgram: true,
            ips: vec!["1.2.3.4".into(), "5.6.7.8".into()],
        }
        .as_json();
        assert_eq!(json["type"], "xicmp");
        assert_eq!(json["settings"]["dgram"], true);
        assert_eq!(json["settings"]["ips"][0], "1.2.3.4");
    }

    #[test]
    fn test_realm_as_json() {
        let json = KcpMask::Realm {
            url: "realm://example.com:1234".into(),
            stun_servers: vec!["stun:stun.l.google.com:19302".into()],
        }
        .as_json();
        assert_eq!(json["type"], "realm");
        assert_eq!(json["settings"]["url"], "realm://example.com:1234");
        assert_eq!(
            json["settings"]["stunServers"][0],
            "stun:stun.l.google.com:19302"
        );
    }

    #[test]
    fn test_display_name_spot_check() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .display_name(),
            "🔀 mKCP Original (XOR)"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: Some("x".into())
            }
            .display_name(),
            "🔐 mKCP AES-128-GCM"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: None
            }
            .display_name(),
            "🌐 mKCP + DNS伪装"
        );
        assert_eq!(KcpMask::Noise.display_name(), "📊 Noise");
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .display_name(),
            "📡 XDNS 扩展DNS"
        );
        assert_eq!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .display_name(),
            "💓 XICMP"
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .display_name(),
            "🕳️ Realm"
        );
    }

    #[test]
    fn test_code_all_variants() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .code(),
            "ml"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: Some("x".into())
            }
            .code(),
            "mla"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: None
            }
            .code(),
            "mld"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("wechat".into()),
                value: None
            }
            .code(),
            "mlw"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("srtp".into()),
                value: None
            }
            .code(),
            "mls"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("utp".into()),
                value: None
            }
            .code(),
            "mlu"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dtls".into()),
                value: None
            }
            .code(),
            "mldt"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("wireguard".into()),
                value: None
            }
            .code(),
            "mlg"
        );
        assert_eq!(KcpMask::Noise.code(), "no");
        assert_eq!(
            KcpMask::Salamander {
                password: "x".into(),
                packet_size: None
            }
            .code(),
            "sa"
        );
        assert_eq!(
            KcpMask::Sudoku {
                password: "x".into()
            }
            .code(),
            "su"
        );
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .code(),
            "xd"
        );
        assert_eq!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .code(),
            "xi"
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .code(),
            "rl"
        );
    }

    #[test]
    fn test_from_code() {
        assert!(matches!(
            KcpMask::from_code("ml"),
            Some(KcpMask::MkcpLegacy {
                header: None,
                value: None
            })
        ));
        assert!(matches!(
            KcpMask::from_code("mla"),
            Some(KcpMask::MkcpLegacy {
                header: None,
                value: Some(_)
            })
        ));
        assert!(
            matches!(KcpMask::from_code("mld"), Some(KcpMask::MkcpLegacy { header: Some(h), value: Some(v) }) if h == "dns" && v == "www.baidu.com")
        );
        assert!(
            matches!(KcpMask::from_code("mlw"), Some(KcpMask::MkcpLegacy { header: Some(h), value: None }) if h == "wechat")
        );
        assert!(matches!(KcpMask::from_code("no"), Some(KcpMask::Noise)));
        assert!(matches!(
            KcpMask::from_code("sa"),
            Some(KcpMask::Salamander { .. })
        ));
        assert!(matches!(
            KcpMask::from_code("su"),
            Some(KcpMask::Sudoku { .. })
        ));
        assert!(matches!(
            KcpMask::from_code("xd"),
            Some(KcpMask::Xdns { .. })
        ));
        assert!(
            matches!(KcpMask::from_code("xi"), Some(KcpMask::Xicmp { dgram: false, ips } ) if ips.is_empty())
        );
        assert!(
            matches!(KcpMask::from_code("rl"), Some(KcpMask::Realm { url, stun_servers }) if url.is_empty() && stun_servers.is_empty())
        );
        assert!(KcpMask::from_code("invalid").is_none());
    }

    #[test]
    fn test_from_code_generates_password() {
        let mask = KcpMask::from_code("mla").unwrap();
        assert!(matches!(
            mask,
            KcpMask::MkcpLegacy {
                header: None,
                value: Some(_)
            }
        ));
        let mask = KcpMask::from_code("sa").unwrap();
        assert!(matches!(mask, KcpMask::Salamander { password: p, .. } if !p.is_empty()));
    }

    #[test]
    fn test_xdns_from_code_new_resolver_format() {
        let mask = KcpMask::from_code("xd").unwrap();
        assert!(
            matches!(mask, KcpMask::Xdns { domains, resolvers } if domains == vec!["www.baidu.com"] && resolvers == vec!["www.baidu.com+udp://1.1.1.1:53"])
        );
    }

    #[test]
    fn test_category_code_all_categories() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .category_code(),
            "enc"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: None
            }
            .category_code(),
            "enc"
        );
        assert_eq!(KcpMask::Noise.category_code(), "obf");
        assert_eq!(
            KcpMask::Salamander {
                password: "x".into(),
                packet_size: None
            }
            .category_code(),
            "obf"
        );
        assert_eq!(
            KcpMask::Sudoku {
                password: "x".into()
            }
            .category_code(),
            "obf"
        );
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .category_code(),
            "ext"
        );
        assert_eq!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .category_code(),
            "ext"
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .category_code(),
            "ext"
        );
    }

    #[test]
    fn test_category_spot_check() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .category(),
            "🔐 加密层"
        );
        assert_eq!(KcpMask::Noise.category(), "🌀 混淆层");
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .category(),
            "⚡ 扩展层"
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .category(),
            "⚡ 扩展层"
        );
    }

    #[test]
    fn test_category_from_code() {
        assert_eq!(KcpMask::category_from_code("enc"), Some("🔐 加密层"));
        assert_eq!(KcpMask::category_from_code("obf"), Some("🌀 混淆层"));
        assert_eq!(KcpMask::category_from_code("ext"), Some("⚡ 扩展层"));
        assert_eq!(KcpMask::category_from_code("dis"), None);
        assert_eq!(KcpMask::category_from_code("xyz"), None);
    }

    #[test]
    fn test_is_encryption() {
        assert!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .is_encryption()
        );
        assert!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: None
            }
            .is_encryption()
        );
        assert!(!KcpMask::Noise.is_encryption());
        assert!(
            !KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .is_encryption()
        );
    }

    #[test]
    fn test_is_sudoku() {
        assert!(
            KcpMask::Sudoku {
                password: "x".into()
            }
            .is_sudoku()
        );
        assert!(
            !KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .is_sudoku()
        );
        assert!(!KcpMask::Noise.is_sudoku());
    }

    #[test]
    fn test_is_transport_replacement() {
        assert!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .is_transport_replacement()
        );
        assert!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .is_transport_replacement()
        );
        assert!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .is_transport_replacement()
        );
        assert!(
            !KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .is_transport_replacement()
        );
        assert!(!KcpMask::Noise.is_transport_replacement());
    }

    #[test]
    fn test_header_size_all_variants() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .header_size(),
            Some(6)
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: Some("x".into())
            }
            .header_size(),
            Some(28)
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("wechat".into()),
                value: None
            }
            .header_size(),
            Some(13)
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: Some("www.baidu.com".into())
            }
            .header_size(),
            Some(31)
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("srtp".into()),
                value: None
            }
            .header_size(),
            Some(4)
        );
        assert_eq!(
            KcpMask::Salamander {
                password: "x".into(),
                packet_size: None
            }
            .header_size(),
            Some(8)
        );
        assert_eq!(KcpMask::Noise.header_size(), None);
        assert_eq!(
            KcpMask::Sudoku {
                password: "x".into()
            }
            .header_size(),
            None
        );
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .header_size(),
            None
        );
        assert_eq!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .header_size(),
            None
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .header_size(),
            None
        );
    }

    #[test]
    fn test_all_variants_length() {
        assert_eq!(KcpMask::all_variants().len(), 14);
    }

    #[test]
    fn test_variants_by_category() {
        let enc = KcpMask::variants_by_category("enc");
        let obf = KcpMask::variants_by_category("obf");
        let ext = KcpMask::variants_by_category("ext");
        assert_eq!(enc.len(), 8);
        assert_eq!(obf.len(), 3);
        assert_eq!(ext.len(), 3);
    }

    #[test]
    fn test_is_compatible_with_empty() {
        let mask = KcpMask::MkcpLegacy {
            header: None,
            value: None,
        };
        assert!(mask.is_compatible_with(&[]).is_ok());
    }

    #[test]
    fn test_is_compatible_with_duplicate() {
        let mask = KcpMask::MkcpLegacy {
            header: None,
            value: None,
        };
        assert!(
            mask.is_compatible_with(&[KcpMask::MkcpLegacy {
                header: None,
                value: None
            }])
            .is_err()
        );
    }

    #[test]
    fn test_is_compatible_with_double_encryption() {
        let mask = KcpMask::MkcpLegacy {
            header: Some("dns".into()),
            value: None,
        };
        assert!(
            mask.is_compatible_with(&[KcpMask::MkcpLegacy {
                header: None,
                value: None
            }])
            .is_err()
        );
    }

    #[test]
    fn test_is_compatible_with_transport_replacement() {
        let xdns = KcpMask::Xdns {
            domains: vec![],
            resolvers: vec![],
        };
        let xicmp = KcpMask::Xicmp {
            dgram: false,
            ips: vec![],
        };
        assert!(
            xdns.is_compatible_with(std::slice::from_ref(&xicmp))
                .is_err()
        );

        let realm = KcpMask::Realm {
            url: "".into(),
            stun_servers: vec![],
        };
        assert!(
            xdns.is_compatible_with(std::slice::from_ref(&realm))
                .is_err()
        );
        assert!(xicmp.is_compatible_with(&[realm]).is_err());
    }

    #[test]
    fn test_validate_stack_empty() {
        assert!(KcpMask::validate_stack(&[]).is_err());
    }

    #[test]
    fn test_validate_stack_valid() {
        assert!(
            KcpMask::validate_stack(&[
                KcpMask::MkcpLegacy {
                    header: None,
                    value: None
                },
                KcpMask::Noise
            ])
            .is_ok()
        );
    }

    #[test]
    fn test_validate_stack_xdns_xicmp_conflict() {
        let masks = [
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![],
            },
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![],
            },
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_validate_stack_xdns_realm_conflict() {
        let masks = [
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![],
            },
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![],
            },
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_canonical_order() {
        let masks = [
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![],
            },
            KcpMask::MkcpLegacy {
                header: None,
                value: None,
            },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        assert!(matches!(&ordered[0], KcpMask::MkcpLegacy { .. }));
        assert!(matches!(&ordered[1], KcpMask::Xdns { .. }));
    }

    #[test]
    fn test_get_stack_warnings_alone() {
        let warnings = KcpMask::get_stack_warnings(&[KcpMask::MkcpLegacy {
            header: None,
            value: None,
        }]);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_get_stack_warnings_no_warning_for_encrypted() {
        let warnings = KcpMask::get_stack_warnings(&[KcpMask::MkcpLegacy {
            header: None,
            value: Some("pwd".into()),
        }]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_generate_aes_password() {
        let password = generate_aes_password();
        assert!(!password.is_empty());
        assert!(password.len() >= 16 && password.len() <= 32);
    }

    #[test]
    fn test_parse_codes_valid() {
        let masks = KcpMask::parse_codes(&["ml", "no"]).unwrap();
        assert_eq!(masks.len(), 2);
    }

    #[test]
    fn test_parse_codes_invalid() {
        assert!(KcpMask::parse_codes(&["ml", "invalid"]).is_err());
    }

    #[test]
    fn test_type_str_all_variants() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .type_str(),
            "mkcp-legacy"
        );
        assert_eq!(KcpMask::Noise.type_str(), "noise");
        assert_eq!(
            KcpMask::Salamander {
                password: "x".into(),
                packet_size: None
            }
            .type_str(),
            "salamander"
        );
        assert_eq!(
            KcpMask::Sudoku {
                password: "x".into()
            }
            .type_str(),
            "sudoku"
        );
        assert_eq!(
            KcpMask::Xdns {
                domains: vec![],
                resolvers: vec![]
            }
            .type_str(),
            "xdns"
        );
        assert_eq!(
            KcpMask::Xicmp {
                dgram: false,
                ips: vec![]
            }
            .type_str(),
            "xicmp"
        );
        assert_eq!(
            KcpMask::Realm {
                url: "".into(),
                stun_servers: vec![]
            }
            .type_str(),
            "realm"
        );
    }

    #[test]
    fn test_brief_spot_check() {
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: None,
                value: None
            }
            .brief(),
            "XOR混淆，仅FNV1a校验"
        );
        assert_eq!(
            KcpMask::MkcpLegacy {
                header: Some("dns".into()),
                value: None
            }
            .brief(),
            "加密+DNS头部伪装"
        );
        assert_eq!(KcpMask::Noise.brief(), "随机噪声填充，抗流量分析");
    }
}
