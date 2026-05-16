use rand::Rng;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub enum KcpMask {
    MkcpOriginal,
    MkcpAes128Gcm {
        password: String,
    },
    Noise,
    Salamander {
        password: String,
    },
    Sudoku {
        password: String,
    },
    HeaderDns {
        domain: String,
    },
    HeaderWechat,
    HeaderSrtp,
    HeaderUtp,
    HeaderDtls,
    HeaderWireguard,
    Xdns {
        domains: Vec<String>,
        resolvers: Vec<String>,
    },
    Xicmp {
        listen_ip: String,
        id: u32,
    },
    HeaderCustom,
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
            KcpMask::MkcpOriginal => "mkcp-original",
            KcpMask::MkcpAes128Gcm { .. } => "mkcp-aes128gcm",
            KcpMask::Noise => "noise",
            KcpMask::Salamander { .. } => "salamander",
            KcpMask::Sudoku { .. } => "sudoku",
            KcpMask::HeaderDns { .. } => "header-dns",
            KcpMask::HeaderWechat => "header-wechat",
            KcpMask::HeaderSrtp => "header-srtp",
            KcpMask::HeaderUtp => "header-utp",
            KcpMask::HeaderDtls => "header-dtls",
            KcpMask::HeaderWireguard => "header-wireguard",
            KcpMask::Xdns { .. } => "xdns",
            KcpMask::Xicmp { .. } => "xicmp",
            KcpMask::HeaderCustom => "header-custom",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal => "🔀 mKCP Original",
            KcpMask::MkcpAes128Gcm { .. } => "🔐 mKCP AES-128-GCM",
            KcpMask::Noise => "📊 Noise",
            KcpMask::Salamander { .. } => "🦎 Salamander",
            KcpMask::Sudoku { .. } => "🔢 Sudoku",
            KcpMask::HeaderDns { .. } => "🌐 DNS 伪装",
            KcpMask::HeaderWechat => "💬 微信视频 伪装",
            KcpMask::HeaderSrtp => "🎬 SRTP 伪装",
            KcpMask::HeaderUtp => "🔗 uTP 伪装",
            KcpMask::HeaderDtls => "🔒 DTLS 伪装",
            KcpMask::HeaderWireguard => "🛡️ WireGuard 伪装",
            KcpMask::Xdns { .. } => "📡 XDNS 扩展DNS",
            KcpMask::Xicmp { .. } => "💓 XICMP",
            KcpMask::HeaderCustom => "✏️ Custom 自定义",
        }
    }

    pub fn detail(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal => {
                "轻量级XOR混淆传输。仅提供FNV1a完整性校验，不含真正加密，仅能抵御被动检测。性能开销最低，安全性最低。建议至少配合一个伪装层使用"
            }
            KcpMask::MkcpAes128Gcm { .. } => {
                "AES-128-GCM端到端认证加密。密码经SHA256派生为128位密钥，提供加密+认证双重保护。推荐首选加密层，安全性高，性能开销适中"
            }
            KcpMask::Noise => {
                "随机噪声填充。在数据包中注入随机长度的噪声数据，有效抵抗基于包大小的流量分析。不提供加密功能，建议与加密层叠加使用"
            }
            KcpMask::Salamander { .. } => {
                "蝾螈混淆协议。使用密码派生的混淆变换，可抵抗深度包检测(DPI)。与Hysteria2的Salamander混淆采用相同算法。建议与加密层叠加使用"
            }
            KcpMask::Sudoku { .. } => {
                "数独混淆算法。基于密码派生的混淆，包含ASCII混淆和随机填充。混淆强度高于Salamander，性能开销略大"
            }
            KcpMask::HeaderDns { .. } => {
                "伪装为DNS查询流量。每个数据包添加DNS查询头部，默认域名www.baidu.com。适合仅允许DNS流量通过的严格网络环境"
            }
            KcpMask::HeaderWechat => {
                "伪装为微信视频通话流量。数据包头部模拟微信VoIP协议格式，适合允许微信通信的网络环境"
            }
            KcpMask::HeaderSrtp => {
                "伪装为安全实时传输协议(SRTP)流量。数据包看起来像音视频流媒体传输，适合允许视频通话的网络"
            }
            KcpMask::HeaderUtp => {
                "伪装为BitTorrent uTP协议流量。数据包头部模拟uTP格式，可能绕过允许P2P流量的限制策略"
            }
            KcpMask::HeaderDtls => {
                "伪装为DTLS 1.2加密数据包。使流量看起来像正常的加密UDP通信(TLS的UDP版本)，具有较好的伪装效果"
            }
            KcpMask::HeaderWireguard => {
                "伪装为WireGuard VPN流量。数据包头部模拟WireGuard协议格式，可能混入VPN流量中，适合允许VPN使用的网络"
            }
            KcpMask::Xdns { .. } => {
                "扩展DNS伪装。支持自定义域名列表和DNS解析器(默认1.1.1.1 UDP)，提供比HeaderDns更灵活的DNS流量模拟。适合需要精确控制DNS伪装行为的场景"
            }
            KcpMask::Xicmp { .. } => {
                "ICMP数据包伪装。将数据包封装为ICMP回显请求/应答格式。适合仅允许ping流量通过的极端限制网络"
            }
            KcpMask::HeaderCustom => {
                "自定义UDP头部伪装。允许高级用户定义自定义的UDP包头部格式。适合有特殊伪装需求的场景"
            }
        }
    }

    pub fn brief(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal => "轻量级XOR混淆，仅FNV1a校验",
            KcpMask::MkcpAes128Gcm { .. } => "AES-128-GCM认证加密，推荐首选",
            KcpMask::Noise => "随机噪声填充，抗流量分析",
            KcpMask::Salamander { .. } => "蝾螈混淆协议，抗深度包检测",
            KcpMask::Sudoku { .. } => "数独混淆算法，强度更高",
            KcpMask::HeaderDns { .. } => "DNS查询流量伪装",
            KcpMask::HeaderWechat => "微信视频通话流量伪装",
            KcpMask::HeaderSrtp => "SRTP音视频流媒体伪装",
            KcpMask::HeaderUtp => "BitTorrent uTP协议伪装",
            KcpMask::HeaderDtls => "DTLS 1.2加密数据包伪装",
            KcpMask::HeaderWireguard => "WireGuard VPN流量伪装",
            KcpMask::Xdns { .. } => "扩展DNS，支持自定义域名和解析器",
            KcpMask::Xicmp { .. } => "ICMP数据包伪装，极端限制网络适用",
            KcpMask::HeaderCustom => "自定义UDP头部格式",
        }
    }

    pub fn category_code(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. } => "enc",
            KcpMask::Noise | KcpMask::Salamander { .. } | KcpMask::Sudoku { .. } => "obf",
            KcpMask::HeaderDns { .. }
            | KcpMask::HeaderWechat
            | KcpMask::HeaderSrtp
            | KcpMask::HeaderUtp
            | KcpMask::HeaderDtls
            | KcpMask::HeaderWireguard => "dis",
            KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } | KcpMask::HeaderCustom => "ext",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. } => "🔐 加密层",
            KcpMask::Noise | KcpMask::Salamander { .. } | KcpMask::Sudoku { .. } => "🌀 混淆层",
            KcpMask::HeaderDns { .. }
            | KcpMask::HeaderWechat
            | KcpMask::HeaderSrtp
            | KcpMask::HeaderUtp
            | KcpMask::HeaderDtls
            | KcpMask::HeaderWireguard => "🎭 伪装层",
            KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } | KcpMask::HeaderCustom => "⚡ 扩展层",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            KcpMask::MkcpOriginal => "mo",
            KcpMask::MkcpAes128Gcm { .. } => "ma",
            KcpMask::Noise => "no",
            KcpMask::Salamander { .. } => "sa",
            KcpMask::Sudoku { .. } => "su",
            KcpMask::HeaderDns { .. } => "hd",
            KcpMask::HeaderWechat => "hw",
            KcpMask::HeaderSrtp => "hs",
            KcpMask::HeaderUtp => "hu",
            KcpMask::HeaderDtls => "hdt",
            KcpMask::HeaderWireguard => "hwg",
            KcpMask::Xdns { .. } => "xd",
            KcpMask::Xicmp { .. } => "xi",
            KcpMask::HeaderCustom => "hc",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "mo" => Some(KcpMask::MkcpOriginal),
            "ma" => Some(KcpMask::MkcpAes128Gcm {
                password: generate_aes_password(),
            }),
            "no" => Some(KcpMask::Noise),
            "sa" => Some(KcpMask::Salamander {
                password: generate_aes_password(),
            }),
            "su" => Some(KcpMask::Sudoku {
                password: generate_aes_password(),
            }),
            "hd" => Some(KcpMask::HeaderDns {
                domain: "www.baidu.com".to_string(),
            }),
            "hw" => Some(KcpMask::HeaderWechat),
            "hs" => Some(KcpMask::HeaderSrtp),
            "hu" => Some(KcpMask::HeaderUtp),
            "hdt" => Some(KcpMask::HeaderDtls),
            "hwg" => Some(KcpMask::HeaderWireguard),
            "xd" => Some(KcpMask::Xdns {
                domains: vec!["www.baidu.com".to_string()],
                resolvers: vec!["+udp://1.1.1.1".to_string()],
            }),
            "xi" => {
                let id = rand::thread_rng().gen_range(1..=65535);
                Some(KcpMask::Xicmp {
                    listen_ip: "0.0.0.0".to_string(),
                    id,
                })
            }
            "hc" => Some(KcpMask::HeaderCustom),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Value {
        match self {
            KcpMask::MkcpOriginal => json!({
                "type": "mkcp-original"
            }),
            KcpMask::MkcpAes128Gcm { password } => json!({
                "type": "mkcp-aes128gcm",
                "settings": { "password": password }
            }),
            KcpMask::Noise => json!({
                "type": "noise"
            }),
            KcpMask::Salamander { password } => json!({
                "type": "salamander",
                "settings": { "password": password }
            }),
            KcpMask::Sudoku { password } => json!({
                "type": "sudoku",
                "settings": { "password": password }
            }),
            KcpMask::HeaderDns { domain } => json!({
                "type": "header-dns",
                "settings": { "domain": domain }
            }),
            KcpMask::HeaderWechat => json!({
                "type": "header-wechat"
            }),
            KcpMask::HeaderSrtp => json!({
                "type": "header-srtp"
            }),
            KcpMask::HeaderUtp => json!({
                "type": "header-utp"
            }),
            KcpMask::HeaderDtls => json!({
                "type": "header-dtls"
            }),
            KcpMask::HeaderWireguard => json!({
                "type": "header-wireguard"
            }),
            KcpMask::Xdns { domains, resolvers } => json!({
                "type": "xdns",
                "settings": {
                    "domains": domains,
                    "resolvers": resolvers
                }
            }),
            KcpMask::Xicmp { listen_ip, id } => json!({
                "type": "xicmp",
                "settings": {
                    "listenIp": listen_ip,
                    "id": id
                }
            }),
            KcpMask::HeaderCustom => json!({
                "type": "header-custom"
            }),
        }
    }

    pub fn all_variants() -> Vec<Self> {
        vec![
            KcpMask::MkcpOriginal,
            KcpMask::MkcpAes128Gcm {
                password: String::new(),
            },
            KcpMask::Noise,
            KcpMask::Salamander {
                password: String::new(),
            },
            KcpMask::Sudoku {
                password: String::new(),
            },
            KcpMask::HeaderDns {
                domain: String::new(),
            },
            KcpMask::HeaderWechat,
            KcpMask::HeaderSrtp,
            KcpMask::HeaderUtp,
            KcpMask::HeaderDtls,
            KcpMask::HeaderWireguard,
            KcpMask::Xdns {
                domains: Vec::new(),
                resolvers: Vec::new(),
            },
            KcpMask::Xicmp {
                listen_ip: String::new(),
                id: 0,
            },
            KcpMask::HeaderCustom,
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
            "dis" => Some("🎭 伪装层"),
            "ext" => Some("⚡ 扩展层"),
            _ => None,
        }
    }

    pub fn is_encryption(&self) -> bool {
        matches!(self, KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. })
    }

    pub fn is_sudoku(&self) -> bool {
        matches!(self, KcpMask::Sudoku { .. })
    }

    pub fn is_transport_replacement(&self) -> bool {
        matches!(self, KcpMask::Xdns { .. } | KcpMask::Xicmp { .. })
    }

    pub fn is_xdns(&self) -> bool {
        matches!(self, KcpMask::Xdns { .. })
    }

    pub fn is_xicmp(&self) -> bool {
        matches!(self, KcpMask::Xicmp { .. })
    }

    pub fn is_disguise_header(&self) -> bool {
        matches!(
            self,
            KcpMask::HeaderDns { .. }
                | KcpMask::HeaderWechat
                | KcpMask::HeaderSrtp
                | KcpMask::HeaderUtp
                | KcpMask::HeaderDtls
                | KcpMask::HeaderWireguard
                | KcpMask::HeaderCustom
        )
    }

    pub fn is_header_conn(&self) -> bool {
        matches!(
            self,
            KcpMask::MkcpOriginal
                | KcpMask::MkcpAes128Gcm { .. }
                | KcpMask::Salamander { .. }
                | KcpMask::HeaderDns { .. }
                | KcpMask::HeaderWechat
                | KcpMask::HeaderSrtp
                | KcpMask::HeaderUtp
                | KcpMask::HeaderDtls
                | KcpMask::HeaderWireguard
                | KcpMask::HeaderCustom
        )
    }

    pub fn header_size(&self) -> Option<usize> {
        match self {
            KcpMask::MkcpOriginal => Some(6),
            KcpMask::MkcpAes128Gcm { .. } => Some(28),
            KcpMask::Salamander { .. } => Some(8),
            KcpMask::HeaderDns { domain } => Some(Self::dns_header_size(domain)),
            KcpMask::HeaderWechat => Some(13),
            KcpMask::HeaderSrtp => Some(4),
            KcpMask::HeaderUtp => Some(4),
            KcpMask::HeaderDtls => Some(13),
            KcpMask::HeaderWireguard => Some(4),
            KcpMask::HeaderCustom => Some(4),
            KcpMask::Noise => None,
            KcpMask::Sudoku { .. } => None,
            KcpMask::Xdns { .. } => None,
            KcpMask::Xicmp { .. } => None,
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
            KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. } => 10,
            KcpMask::Salamander { .. } => 20,
            KcpMask::HeaderDns { .. }
            | KcpMask::HeaderWechat
            | KcpMask::HeaderSrtp
            | KcpMask::HeaderUtp
            | KcpMask::HeaderDtls
            | KcpMask::HeaderWireguard
            | KcpMask::HeaderCustom => 30,
            KcpMask::Noise => 40,
            KcpMask::Xdns { .. } => 50,
            KcpMask::Xicmp { .. } => 60,
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
            let name = if self.is_xdns() { "XDNS" } else { "XICMP" };
            let other = if self.is_xdns() { "XICMP" } else { "XDNS" };
            return Err(format!("{}和{}不能同时使用", name, other));
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
        if masks.iter().any(|m| m.is_xdns()) && masks.iter().any(|m| m.is_xicmp()) {
            return Err("XDNS和XICMP不能同时使用".to_string());
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
        if masks.len() == 1 && matches!(masks[0], KcpMask::MkcpOriginal) {
            warnings.push("⚠️ mKCP Original 单独使用安全性低，建议配合伪装层使用".to_string());
        }
        warnings
    }
}
