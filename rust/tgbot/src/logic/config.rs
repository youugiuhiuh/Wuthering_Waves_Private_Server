use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use once_cell::sync::Lazy;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio::fs;

use crate::core::paths::{xray, warp};
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::logic::cmd_async::run_cmd_output;

/// 服务端 mldsa65Seed（32 字节 seed 的 base64url），来自 xray/wwps-core mldsa65 输出。
/// 优先环境变量 `TGBOT_REALITY_PQ_SEED`，否则 `/etc/wwps/reality_pq.seed`。
static REALITY_PQ_SEED: Lazy<String> = Lazy::new(|| {
    if let Ok(v) = std::env::var("TGBOT_REALITY_PQ_SEED") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    if let Ok(c) = std::fs::read_to_string("/etc/wwps/reality_pq.seed") {
        let t = c.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    String::new()
});

/// 客户端 mldsa65Verify / pqv（公钥 base64url），来自 xray/wwps-core mldsa65 输出。
/// 优先环境变量 `TGBOT_REALITY_PQ_VERIFY` 或 `TGBOT_REALITY_PQ_PUB`，否则 `/etc/wwps/reality_pq.pub`。
static REALITY_PQ_VERIFY: Lazy<String> = Lazy::new(|| {
    if let Ok(v) = std::env::var("TGBOT_REALITY_PQ_VERIFY") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    if let Ok(v) = std::env::var("TGBOT_REALITY_PQ_PUB") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    if let Ok(c) = std::fs::read_to_string("/etc/wwps/reality_pq.pub") {
        let t = c.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    String::new()
});

/// 将 PQ verify（Standard 或 URL-safe Base64）转为 URL-safe 输出，兼容链接与 JSON。
fn reality_pq_verify_as_base64url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = general_purpose::STANDARD
        .decode(s)
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(s))
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(general_purpose::URL_SAFE_NO_PAD.encode(&bytes))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Vision,
    XHTTP,
    Kcp,
}

#[derive(Debug, Clone)]
pub enum KcpMask {
    MkcpOriginal,
    MkcpAes128Gcm { password: String },
    Noise,
    Salamander { password: String },
    Sudoku { password: String },
    HeaderDns { domain: String },
    HeaderWechat,
    HeaderSrtp,
    HeaderUtp,
    HeaderDtls,
    HeaderWireguard,
    Xdns { domains: Vec<String>, resolvers: Vec<String> },
    Xicmp { listen_ip: String, id: u32 },
    HeaderCustom,
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
            KcpMask::MkcpOriginal => "轻量级XOR混淆传输。仅提供FNV1a完整性校验，不含真正加密，仅能抵御被动检测。性能开销最低，安全性最低。建议至少配合一个伪装层使用",
            KcpMask::MkcpAes128Gcm { .. } => "AES-128-GCM端到端认证加密。密码经SHA256派生为128位密钥，提供加密+认证双重保护。推荐首选加密层，安全性高，性能开销适中",
            KcpMask::Noise => "随机噪声填充。在数据包中注入随机长度的噪声数据，有效抵抗基于包大小的流量分析。不提供加密功能，建议与加密层叠加使用",
            KcpMask::Salamander { .. } => "蝾螈混淆协议。使用密码派生的混淆变换，可抵抗深度包检测(DPI)。与Hysteria2的Salamander混淆采用相同算法。建议与加密层叠加使用",
            KcpMask::Sudoku { .. } => "数独混淆算法。基于密码派生的混淆，包含ASCII混淆和随机填充。混淆强度高于Salamander，性能开销略大",
            KcpMask::HeaderDns { .. } => "伪装为DNS查询流量。每个数据包添加DNS查询头部，默认域名www.baidu.com。适合仅允许DNS流量通过的严格网络环境",
            KcpMask::HeaderWechat => "伪装为微信视频通话流量。数据包头部模拟微信VoIP协议格式，适合允许微信通信的网络环境",
            KcpMask::HeaderSrtp => "伪装为安全实时传输协议(SRTP)流量。数据包看起来像音视频流媒体传输，适合允许视频通话的网络",
            KcpMask::HeaderUtp => "伪装为BitTorrent uTP协议流量。数据包头部模拟uTP格式，可能绕过允许P2P流量的限制策略",
            KcpMask::HeaderDtls => "伪装为DTLS 1.2加密数据包。使流量看起来像正常的加密UDP通信(TLS的UDP版本)，具有较好的伪装效果",
            KcpMask::HeaderWireguard => "伪装为WireGuard VPN流量。数据包头部模拟WireGuard协议格式，可能混入VPN流量中，适合允许VPN使用的网络",
            KcpMask::Xdns { .. } => "扩展DNS伪装。支持自定义域名列表和DNS解析器(默认1.1.1.1 UDP)，提供比HeaderDns更灵活的DNS流量模拟。适合需要精确控制DNS伪装行为的场景",
            KcpMask::Xicmp { .. } => "ICMP数据包伪装。将数据包封装为ICMP回显请求/应答格式。适合仅允许ping流量通过的极端限制网络",
            KcpMask::HeaderCustom => "自定义UDP头部伪装。允许高级用户定义自定义的UDP包头部格式。适合有特殊伪装需求的场景",
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
            KcpMask::HeaderDns { .. } | KcpMask::HeaderWechat | KcpMask::HeaderSrtp
            | KcpMask::HeaderUtp | KcpMask::HeaderDtls | KcpMask::HeaderWireguard => "🎭 伪装层",
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
                password: ConfigManager::generate_aes_password(),
            }),
            "no" => Some(KcpMask::Noise),
            "sa" => Some(KcpMask::Salamander {
                password: ConfigManager::generate_aes_password(),
            }),
            "su" => Some(KcpMask::Sudoku {
                password: ConfigManager::generate_aes_password(),
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
            KcpMask::MkcpAes128Gcm { password: String::new() },
            KcpMask::Noise,
            KcpMask::Salamander { password: String::new() },
            KcpMask::Sudoku { password: String::new() },
            KcpMask::HeaderDns { domain: String::new() },
            KcpMask::HeaderWechat,
            KcpMask::HeaderSrtp,
            KcpMask::HeaderUtp,
            KcpMask::HeaderDtls,
            KcpMask::HeaderWireguard,
            KcpMask::Xdns { domains: Vec::new(), resolvers: Vec::new() },
            KcpMask::Xicmp { listen_ip: String::new(), id: 0 },
            KcpMask::HeaderCustom,
        ]
    }

    pub fn parse_codes(mask_codes: &[&str]) -> Result<Vec<Self>, String> {
        let mut masks = Vec::new();
        for code in mask_codes {
            let mask = Self::from_code(code)
                .ok_or_else(|| format!("Invalid mask code: {}", code))?;
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
            KcpMask::MkcpOriginal
            | KcpMask::MkcpAes128Gcm { .. } => 10,
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
        if self.is_xicmp() && !existing.is_empty() {
            return Err("XICMP必须是最外层(最后添加的遮罩)".to_string());
        }

        if self.is_transport_replacement() {
            if existing.iter().any(|m| m.is_transport_replacement()) {
                let name = if self.is_xdns() { "XDNS" } else { "XICMP" };
                let other = if self.is_xdns() { "XICMP" } else { "XDNS" };
                return Err(format!("{}和{}不能同时使用", name, other));
            }
        }

        if self.is_encryption() {
            if existing.iter().any(|m| m.is_encryption()) {
                return Err("重复的加密层".to_string());
            }
        }

        if self.is_sudoku() {
            if existing.iter().any(|m| m.is_sudoku()) {
                return Err("重复的Sudoku".to_string());
            }
        }

        if existing.iter().any(|m| m.code() == self.code()) {
            return Err(format!("重复的{}", self.display_name()));
        }

        if matches!(self, KcpMask::MkcpOriginal) && existing.is_empty() {
            return Err("mKCP Original单独使用安全性低，建议配合伪装层使用".to_string());
        }

        let total_header: usize = existing.iter()
            .filter_map(|m| m.header_size())
            .sum::<usize>()
            + self.header_size().unwrap_or(0);
        let sudoku_reserve = if self.is_sudoku() || existing.iter().any(|m| m.is_sudoku()) {
            2400
        } else {
            0
        };
        if total_header + sudoku_reserve > 3800 {
            return Err(format!("header总大小{}字节过大，可能超出UDP包限制(4096字节)", total_header));
        }

        Ok(())
    }

    pub fn validate_stack(masks: &[KcpMask]) -> Result<(), String> {
        if masks.is_empty() {
            return Err("请至少选择1层遮罩".to_string());
        }
        if masks.iter().any(|m| m.is_xicmp()) {
            if !masks.last().map(|m| m.is_xicmp()).unwrap_or(false) {
                return Err("XICMP必须是最外层(最后添加的遮罩)".to_string());
            }
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
        if masks.iter().any(|m| m.is_sudoku()) {
            if !masks.last().map(|m| m.is_sudoku()).unwrap_or(false) {
                return Err("Sudoku必须是最后一层(最内侧)".to_string());
            }
        }
        if let Some(enc_idx) = masks.iter().position(|m| m.is_encryption()) {
            for m in &masks[enc_idx + 1..] {
                if m.is_disguise_header() || matches!(m, KcpMask::Salamander { .. }) {
                    return Err("加密层之后不能有伪装/混淆层(加密层应紧贴数据)".to_string());
                }
            }
        }
        if masks.len() == 1 && matches!(masks[0], KcpMask::MkcpOriginal) {
            return Err("mKCP Original单独使用安全性低，建议配合伪装层使用".to_string());
        }
        let total_header: usize = masks.iter().filter_map(|m| m.header_size()).sum();
        let sudoku_reserve = if masks.iter().any(|m| m.is_sudoku()) { 2400 } else { 0 };
        if total_header + sudoku_reserve > 3800 {
            return Err(format!(
                "header总大小{}字节过大，可能超出UDP包限制(4096字节)",
                total_header
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConfigManager;

impl ConfigManager {
    const CONFIG_BASE_PATH: &'static str = xray::DIR;
    const TIMEOUT_WWPS_CORE: Duration = Duration::from_secs(5);

    fn generate_aes_password() -> String {
        let rng_len = rand::thread_rng().gen_range(16..32);
        rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(rng_len)
            .map(char::from)
            .collect()
    }

    pub async fn get_clients_from_config(file_path: &str) -> Result<Vec<Value>> {
        let content = fs::read_to_string(file_path)
            .await
            .context("❌ 读取配置文件失败")?;
        let v: Value = serde_json::from_str(&content).context("❌ 解析 JSON 失败")?;

        let clients = v["inbounds"][0]["settings"]["clients"]
            .as_array()
            .or_else(|| v["inbounds"][0]["users"].as_array())
            .or_else(|| v["inbounds"][1]["settings"]["clients"].as_array())
            .cloned()
            .unwrap_or_default();

        Ok(clients)
    }

    pub async fn list_all_inbound_files() -> Result<Vec<String>> {
        let mut out = Vec::new();
        
        if let Ok(mut rd) = fs::read_dir(xray::CONF_DIR).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && name.ends_with("_inbounds.json")
                {
                    out.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
        
        Ok(out)
    }

    pub async fn list_inbound_files_by_proto(proto: Proto) -> Result<Vec<String>> {
        let all = Self::list_all_inbound_files().await?;
        let prefix = match proto {
            Proto::Vision => "batch_reality",
            Proto::XHTTP => "batch_xhttp",
            Proto::Kcp => "batch_kcp",
        };
        let filtered: Vec<String> = all
            .into_iter()
            .filter(|p| {
                if let Some(name) = p.split('/').next_back() {
                    name.starts_with(prefix)
                } else {
                    false
                }
            })
            .collect();
        Ok(filtered)
    }

    /// 是否已配置 ML-DSA-65（Reality PQ）：seed 或 verify 的环境变量/文件存在即视为已配置。
    pub fn is_reality_pq_configured() -> bool {
        if std::env::var("TGBOT_REALITY_PQ_SEED")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if std::env::var("TGBOT_REALITY_PQ_VERIFY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if std::env::var("TGBOT_REALITY_PQ_PUB")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        Path::new("/etc/wwps/reality_pq.seed").exists()
            || Path::new("/etc/wwps/reality_pq.pub").exists()
    }

    /// 删除 ML-DSA-65 相关文件（禁用）。删除后需重启 Bot 或重新生成配置后生效。
    pub async fn delete_reality_pq() -> Result<()> {
        const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
        const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
        const PQ_KEY_PATH: &str = "/etc/wwps/reality_pq.key";
        for path in [PQ_SEED_PATH, PQ_PUB_PATH, PQ_KEY_PATH] {
            if Path::new(path).exists() {
                let _ = fs::remove_file(path).await;
            }
        }
        Ok(())
    }

    /// 通过执行 wwps-core mldsa65（或 xray mldsa65）生成 seed/verify 并写入文件，与 Xray 完全兼容。
    pub async fn generate_reality_pq_keys() -> Result<()> {
        const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
        const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
        let stdout = match run_wwps_core_cmd(&["mldsa65"]).await {
            Ok(out) => out,
            Err(_) => {
                let (status, out, err) =
                    run_cmd_output("xray", &["mldsa65"], Self::TIMEOUT_WWPS_CORE).await?;
                if !status.success() {
                    anyhow::bail!("xray mldsa65 执行失败: {}", err);
                }
                out
            }
        };
        let seed = stdout
            .lines()
            .find(|l| l.starts_with("Seed:"))
            .and_then(|l| l.strip_prefix("Seed:").map(|s| s.trim().to_string()))
            .ok_or_else(|| anyhow!("❌ mldsa65 输出未包含 Seed"))?;
        let verify = stdout
            .lines()
            .find(|l| l.starts_with("Verify:"))
            .and_then(|l| l.strip_prefix("Verify:").map(|s| s.trim().to_string()))
            .ok_or_else(|| anyhow!("❌ mldsa65 输出未包含 Verify"))?;
        if seed.is_empty() || verify.is_empty() {
            anyhow::bail!("❌ mldsa65 输出 Seed/Verify 为空");
        }
        let dir = Path::new(PQ_SEED_PATH)
            .parent()
            .unwrap_or(Path::new("/etc/wwps"));
        if !dir.exists() {
            tokio::fs::create_dir_all(dir)
                .await
                .context("创建 /etc/wwps 失败")?;
        }
        fs::write(PQ_SEED_PATH, seed.as_bytes())
            .await
            .context("写入 reality_pq.seed 失败")?;
        fs::write(PQ_PUB_PATH, verify.as_bytes())
            .await
            .context("写入 reality_pq.pub 失败")?;
        Ok(())
    }

    async fn generate_wwps_uuid() -> Result<String> {
        let stdout = run_wwps_core_cmd(&["uuid"]).await?;
        Ok(stdout.trim().to_string())
    }

    async fn generate_wwps_x25519() -> Result<(String, String)> {
        let stdout = run_wwps_core_cmd(&["x25519"]).await?;

        let priv_key = stdout
            .lines()
            .find(|l| l.contains("PrivateKey"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .ok_or_else(|| anyhow!("❌ 未找到 PrivateKey"))?;

        let pub_key = stdout
            .lines()
            .find(|l| l.contains("Password") || l.contains("PublicKey"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .ok_or_else(|| anyhow!("❌ 未找到 PublicKey"))?;

        Ok((priv_key, pub_key))
    }

    fn generate_random_short_id() -> String {
        let mut rng = StdRng::from_entropy();
        format!("{:016x}", rng.r#gen::<u64>())
    }

    fn generate_random_path() -> String {
        let mut rng = StdRng::from_entropy();
        let suffix: String = (0..5)
            .map(|_| {
                let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        format!("/xhttp_{}", suffix)
    }

    pub async fn generate_secure_batch_filename(proto: Proto) -> Result<String> {
        let uuid = Self::generate_wwps_uuid().await?;
        let uuid_short = Self::uuid_short_prefix(&uuid);
        let prefix = match proto {
            Proto::Vision => "batch_reality",
            Proto::XHTTP => "batch_xhttp",
            Proto::Kcp => "batch_kcp",
        };
        Ok(format!("{}_{}_inbounds.json", prefix, uuid_short))
    }

    fn uuid_short_prefix(uuid: &str) -> String {
        uuid.split('-')
            .next()
            .unwrap_or(uuid)
            .chars()
            .take(8)
            .collect::<String>()
    }

    fn build_reality_vless_inbound(
        tag: &str,
        port: i32,
        uuid: &str,
        email: &str,
        sni: &str,
        _pub_key: &str,
        priv_key: &str,
        short_id: &str,
        ip_version: IpVersion,
        proto: Proto,
        path: Option<&str>,
        enable_pq: bool,
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 => "0.0.0.0",
            // 双栈分离需要同时服务 IPv4/IPv6 上下行，优先使用 IPv6 wildcard。
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary | IpVersion::SplitStackV4Primary => {
                "::"
            }
        };

        let client = if proto == Proto::Vision {
            json!({
                "id": uuid,
                "email": email,
                "flow": "xtls-rprx-vision"
            })
        } else {
            json!({
                "id": uuid,
                "email": email
            })
        };

        let mut stream_settings = json!({
            "network": match proto {
                Proto::Vision => "tcp",
                Proto::XHTTP => "xhttp",
                Proto::Kcp => {
                    unreachable!("Kcp should use build_kcp_inbound")
                }
            },
            "security": "reality",
            "realitySettings": {
                "show": false,
                "dest": format!("{}:443", sni),
                "xver": 0,
                "serverNames": [sni],
                "privateKey": priv_key,
                "shortIds": ["", short_id]
            }
        });

        // 服务端：仅在当前 SNI 通过 TLS 探测且存在 PQ seed 时，下发 mldsa65Seed。
        if enable_pq && !REALITY_PQ_SEED.is_empty() {
            stream_settings["realitySettings"]["mldsa65Seed"] =
                serde_json::Value::String(REALITY_PQ_SEED.clone());
        }

        if proto == Proto::XHTTP {
            let actual_path = path.unwrap_or("/xhttp_client_upload");
            stream_settings["xhttpSettings"] = json!({
                "host": "", // 显式设置 host 以符合 #4118 建议
                "path": actual_path,
                "mode": "auto"
            });
        }

        json!({
            "listen": listen_ip,
            "port": port,
            "protocol": "vless",
            "tag": tag,
            "settings": {
                "clients": [client],
                "decryption": "none"
            },
            "streamSettings": stream_settings,
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "metadataOnly": false
            }
        })
    }

    pub(crate) fn build_kcp_inbound(
        tag: &str,
        port: i32,
        uuid: &str,
        email: &str,
        ip_version: IpVersion,
        masks: &[KcpMask],
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => "0.0.0.0",
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => "::",
        };

        let client = json!({
            "id": uuid,
            "email": email
        });

        let udp_array: Vec<Value> = masks.iter().map(|m| m.as_json()).collect();

        json!({
            "listen": listen_ip,
            "port": port,
            "protocol": "vless",
            "tag": tag,
            "settings": {
                "clients": [client],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "kcp",
                "security": "none",
                "finalmask": {
                    "udp": udp_array
                },
                "kcpSettings": {
                    "mtu": 1350,
                    "tti": 50,
                    "uplinkCapacity": 5,
                    "downlinkCapacity": 20,
                    "cwndMultiplier": 1,
                    "maxSendingWindow": 2097152
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "metadataOnly": false
            }
        })
    }

    pub(crate) fn generate_kcp_client_link(
        uuid: &str,
        host: &str,
        port: i32,
        email: &str,
        ip_version: IpVersion,
        masks: &[KcpMask],
    ) -> String {
        let udp_array: Vec<Value> = masks.iter().map(|m| m.as_json()).collect();
        let finalmask_json = json!({
            "udp": udp_array
        });
        let fm_str = serde_json::to_string(&finalmask_json).unwrap();
        let fm_encoded = utf8_percent_encode(&fm_str, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();

        format!(
            "vless://{}@{}:{}?encryption=none&type=kcp&security=none&fm={}#{}",
            uuid, fmt_host, port, fm_encoded, encoded_email
        )
    }

    pub async fn batch_create_kcp(
        count: usize,
        standalone: bool,
        ip_version: IpVersion,
        mask_codes: &[&str],
    ) -> Result<BatchCreationResult> {
        let masks = KcpMask::parse_codes(mask_codes)
            .map_err(|e| anyhow!("{}", e))?;

        let mask_types: Vec<&str> = masks.iter().map(|m| m.type_str()).collect();
        let mask_label = mask_types.join("+");

        let (host, _) = Self::resolve_public_hosts(
            ip_version,
            crate::logic::system::SystemMonitor::get_public_ip().await,
            crate::logic::system::SystemMonitor::get_public_ipv6().await,
        )?;

        let mut rng = StdRng::from_entropy();

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..count {
            let port = loop {
                let p = rng.gen_range(10000..60000);
                if crate::logic::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                    continue;
                }
                if crate::logic::maintenance::MaintenanceManager::is_port_available(p).await {
                    break p as i32;
                }
            };

            let uuid = Self::generate_wwps_uuid().await?;
            let uuid_short = Self::uuid_short_prefix(&uuid);

let email = format!("{}-vless-kcp-{}", uuid_short, mask_label);
        let tag = format!("KCP-{}-{}", i + 1, uuid_short);

        let config = Self::build_kcp_inbound(
            &tag, port, &uuid, &email, ip_version, &masks,
        );
        batch_configs.push(config);

        let link = Self::generate_kcp_client_link(
            &uuid, &host, port, &email, ip_version, &masks,
        );
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        if standalone {
            Self::create_standalone_config(batch_configs, links, Proto::Kcp).await
        } else {
            Self::update_existing_config(batch_configs, links).await
        }
    }

    pub async fn batch_create_reality_vision_enhanced(
        count: usize,
        standalone: bool,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let (host, _) = Self::resolve_public_hosts(
            ip_version,
            crate::logic::system::SystemMonitor::get_public_ip().await,
            crate::logic::system::SystemMonitor::get_public_ipv6().await,
        )?;

        let mut rng = StdRng::from_entropy();
        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = crate::logic::sni_selector::SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        let port_443_available =
            crate::logic::maintenance::MaintenanceManager::is_port_available(443).await;

        for i in 0..count {
            let sni = selector.next();

            // 判断当前 SNI 是否适合启用 PQ（证书链长度 + 公钥算法）。
            let pq_ok = crate::logic::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, Proto::Vision, preferred)
                    .await?;

            let config = Self::build_reality_vless_inbound(
                &tag,
                port,
                &uuid,
                &email,
                &sni,
                &pub_key,
                &priv_key,
                &short_id,
                ip_version,
                Proto::Vision,
                path.as_deref(),
                pq_ok,
            );

            batch_configs.push(config);

            let link = Self::generate_client_link(
                &uuid,
                &host,
                port,
                &sni,
                &pub_key,
                &short_id,
                &email,
                ip_version,
                Proto::Vision,
                path.as_deref(),
                None,
                pq_ok,
            );
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        if standalone {
            Self::create_standalone_config(batch_configs, links, Proto::Vision).await
        } else {
            Self::update_existing_config(batch_configs, links).await
        }
    }

    pub async fn batch_create_xhttp_reality_enhanced(
        count: usize,
        standalone: bool,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let (host, host_secondary) = Self::resolve_public_hosts(
            ip_version,
            crate::logic::system::SystemMonitor::get_public_ip().await,
            crate::logic::system::SystemMonitor::get_public_ipv6().await,
        )?;

        let mut rng = StdRng::from_entropy();
        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = crate::logic::sni_selector::SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        let port_443_available =
            crate::logic::maintenance::MaintenanceManager::is_port_available(443).await;

        for i in 0..count {
            let sni = selector.next();

            let pq_ok = crate::logic::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, Proto::XHTTP, preferred)
                    .await?;

            let config = Self::build_reality_vless_inbound(
                &tag,
                port,
                &uuid,
                &email,
                &sni,
                &pub_key,
                &priv_key,
                &short_id,
                ip_version,
                Proto::XHTTP,
                path.as_deref(),
                pq_ok,
            );

            batch_configs.push(config);

            let link = Self::generate_client_link(
                &uuid,
                &host,
                port,
                &sni,
                &pub_key,
                &short_id,
                &email,
                ip_version,
                Proto::XHTTP,
                path.as_deref(),
                host_secondary.as_deref(),
                pq_ok,
            );
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        if standalone {
            Self::create_standalone_config(batch_configs, links, Proto::XHTTP).await
        } else {
            Self::update_existing_config(batch_configs, links).await
        }
    }

    fn resolve_public_hosts(
        ip_version: IpVersion,
        ipv4: Result<String>,
        ipv6: Result<String>,
    ) -> Result<(String, Option<String>)> {
        match ip_version {
            IpVersion::IPv4 => Ok((ipv4?, None)),
            IpVersion::IPv6 => Ok((ipv6?, None)),
            IpVersion::SplitStackV6Primary => Ok((ipv6?, Some(ipv4?))),
            IpVersion::SplitStackV4Primary => Ok((ipv4?, Some(ipv6?))),
        }
    }

    async fn generate_enhanced_config(
        rng: &mut StdRng,
        sni: String,
        index: usize,
        proto: Proto,
        preferred_port: Option<u16>,
    ) -> Result<(
        i32,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    )> {
        let port: i32 = if let Some(pp) = preferred_port {
            if crate::logic::maintenance::MaintenanceManager::is_port_available(pp).await {
                pp as i32
            } else {
                loop {
                    let p = rng.gen_range(10000..60000);
                    if crate::logic::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                        continue;
                    }
                    if crate::logic::maintenance::MaintenanceManager::is_port_available(p).await {
                        break p as i32;
                    }
                }
            }
        } else {
            loop {
                let p = rng.gen_range(10000..60000);
                if crate::logic::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                    continue;
                }
                if crate::logic::maintenance::MaintenanceManager::is_port_available(p).await {
                    break p as i32;
                }
            }
        };

        // 生成唯一参数
        let uuid = Self::generate_wwps_uuid().await?;
        let (priv_key, pub_key) = Self::generate_wwps_x25519().await?;
        let short_id = Self::generate_random_short_id();
        let uuid_short = Self::uuid_short_prefix(&uuid);

        let suffix = match proto {
            Proto::Vision => "vless_reality_vision",
            Proto::XHTTP => "vless_xhttp_reality",
            Proto::Kcp => "vless_kcp",
        };
        let email = format!("{}-{}", uuid_short, suffix);
        let tag = format!(
            "{}-{}-{}",
            match proto {
                Proto::Vision => "VLESS",
                Proto::XHTTP => "XHTTP",
                Proto::Kcp => "KCP",
            },
            uuid_short,
            index
        );

        let path = if proto == Proto::XHTTP {
            Some(Self::generate_random_path())
        } else {
            None
        };

        Ok((
            port as i32,
            uuid,
            priv_key,
            pub_key,
            short_id,
            sni,
            email,
            tag,
            path,
        ))
    }

    fn generate_client_link(
        uuid: &str,
        host: &str,
        port: i32,
        sni: &str,
        pub_key: &str,
        short_id: &str,
        email: &str,
        ip_version: IpVersion,
        proto: Proto,
        path: Option<&str>,
        host_secondary: Option<&str>,
        enable_pq: bool,
    ) -> String {
        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };

        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_pbk = utf8_percent_encode(pub_key, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();

        match proto {
            Proto::Vision => {
                let mut link = format!(
                    "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=tcp&headerType=none",
                    uuid, fmt_host, port, encoded_sni, encoded_pbk, short_id,
                );

                if enable_pq {
                    if let Some(pqv) = reality_pq_verify_as_base64url(&REALITY_PQ_VERIFY) {
                        let encoded_pqv = utf8_percent_encode(&pqv, NON_ALPHANUMERIC).to_string();
                        link.push_str(&format!("&pqv={}", encoded_pqv));
                    }
                }

                format!("{}#{}", link, encoded_email)
            }
            Proto::XHTTP => {
                // 参考 GitHub #716 标准提案
                let actual_path = path.unwrap_or("/xhttp_client_upload");
                let encoded_path = utf8_percent_encode(actual_path, NON_ALPHANUMERIC).to_string();
                let mut link = format!(
                    "vless://{}@{}:{}?encryption=none&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=xhttp&path={}&mode=auto",
                    uuid, fmt_host, port, encoded_sni, encoded_pbk, short_id, encoded_path
                );

                if let Some(secondary) = host_secondary {
                    // 构建 extra.downloadSettings JSON 并进行 URL 编码
                    let extra_json = json!({
                        "downloadSettings": {
                            "address": secondary,
                            "port": port,
                            "network": "xhttp",
                            "security": "reality",
                            "realitySettings": {
                                "serverName": sni,
                                "fingerprint": "chrome",
                                "publicKey": pub_key,
                                "shortId": short_id
                            },
                            "xhttpSettings": {
                                "host": "",
                                "path": actual_path,
                                "mode": "auto"
                            }
                        }
                    });
                    if let Ok(extra_str) = serde_json::to_string(&extra_json) {
                        let encoded_extra =
                            utf8_percent_encode(&extra_str, NON_ALPHANUMERIC).to_string();
                        link.push_str(&format!("&extra={}", encoded_extra));
                    }
                }

                if enable_pq {
                    if let Some(pqv) = reality_pq_verify_as_base64url(&REALITY_PQ_VERIFY) {
                        let encoded_pqv = utf8_percent_encode(&pqv, NON_ALPHANUMERIC).to_string();
                        link.push_str(&format!("&pqv={}", encoded_pqv));
                    }
                }

                format!("{}#{}", link, encoded_email)
            }
            Proto::Kcp => {
                unreachable!("Kcp should use generate_kcp_client_link instead")
            }
        }
    }

    async fn create_standalone_config(
        configs: Vec<Value>,
        links: Vec<String>,
        proto: Proto,
    ) -> Result<BatchCreationResult> {
        // 生成独立文件名
        let filename = Self::generate_secure_batch_filename(proto).await?;
        let config_path = format!("{}/{}", xray::CONF_DIR, filename);

        let created_count = configs.len();

        // 创建完整配置结构
        let config = json!({
            "log": {
                "loglevel": "warning"
            },
            "dns": {
                "servers": [
                    "https+local://1.1.1.1/dns-query",
                    "https+local://8.8.8.8/dns-query"
                ],
                "tag": "dns"
            },
            "inbounds": configs,
            "outbounds": [
                {
                    "protocol": "freedom",
                    "settings": {},
                    "tag": "direct"
                },
                {
                    "protocol": "blackhole",
                    "settings": {},
                    "tag": "blocked"
                }
            ],
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": []
            }
        });

        // 保存文件
        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_path, content).await?;
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count,
        })
    }

    async fn update_existing_config(
        configs: Vec<Value>,
        links: Vec<String>,
    ) -> Result<BatchCreationResult> {
        let created_count = configs.len();
        // 备份原配置
        let existing_path = format!("{}/07_VLESS_vision_reality_inbounds.json", xray::CONF_DIR);
        let backup_path = Self::backup_config_file(&existing_path).await?;

        // 更新现有配置
        let mut v: Value = serde_json::from_str(&fs::read_to_string(&existing_path).await?)?;

        // 清理旧配置并添加新配置
        if let Some(inbounds) = v["inbounds"].as_array_mut() {
            inbounds.retain(|ib| {
                let tag = ib["tag"].as_str().unwrap_or("");
                !tag.starts_with("VLESS-")
            });
            for config in configs {
                inbounds.push(config);
            }
        }

        // 保存配置
        fs::write(&existing_path, serde_json::to_string_pretty(&v)?).await?;
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;

        Ok(BatchCreationResult {
            links,
            config_file: None,
            backup_file: Some(backup_path),
            created_count,
        })
    }

    async fn backup_config_file(path: &str) -> Result<String> {
        let timestamp = chrono::Utc::now().timestamp();
        let backup_path = format!("{}.backup.{}", path, timestamp);
        fs::copy(path, &backup_path).await?;
        Ok(backup_path)
    }

    pub async fn delete_all_configurations() -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        let count = files.len();
        for file in &files {
            let _ = fs::remove_file(file).await;
        }
        if count > 0 {
            crate::logic::maintenance::MaintenanceManager::reload_core().await?;
        }
        Ok(count)
    }

    pub async fn delete_configurations_by_count(count: usize) -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        if files.is_empty() {
            return Ok(0);
        }

        // 按修改时间排序（从旧到新）
        let mut file_with_time = Vec::new();
        for f in files {
            if let Ok(meta) = std::fs::metadata(&f) {
                if let Ok(time) = meta.modified() {
                    file_with_time.push((f, time));
                }
            }
        }
        file_with_time.sort_by(|a, b| a.1.cmp(&b.1));

        let to_delete = file_with_time.iter().take(count);
        let mut deleted_count = 0;
        for (f, _) in to_delete {
            if fs::remove_file(f).await.is_ok() {
                deleted_count += 1;
            }
        }

        if deleted_count > 0 {
            crate::logic::maintenance::MaintenanceManager::reload_core().await?;
        }
        Ok(deleted_count)
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        fs::remove_file(path).await.context("❌ 删除配置文件失败")?;
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn update_warp_routing_rules(rules: Vec<String>, mode: WarpMode) -> Result<()> {
        let config_path = format!("{}/10_warp_routing.json", xray::CONF_DIR);
        let account_path = warp::ACCOUNT_FILE;

        // Read account config
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

        // Standard Cloudflare WARP Endpoint & PublicKey
        let peer_pub_key = "bmXOC+F1FxEMF9dyiK2H5/1SUtzH0JuVo51h2wPfgyo=";
        let peer_endpoint = "engage.cloudflareclient.com:2408";

        // Define WireGuard outbound
        // If mode is Default -> tag: "warp", no extra freedom outbound
        // If mode is IPv4/IPv6 -> tag: "proxy-warp", add extra freedom outbound "warp" -> dialerProxy "proxy-warp"
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

        // If specific IP version required, add Freedom outbound with dialerProxy
        if mode != WarpMode::Default {
            let strategy = match mode {
                WarpMode::IPv4 => "UseIPv4",
                WarpMode::IPv6 => "UseIPv6",
                _ => "UseIP",
            };
            outbounds.push(json!({
                "tag": "warp", // The tag used by routing rules
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

        // SOCKS5 Inbound (Listening on 127.0.0.1:40000)
        let socks_inbound = json!({
            "tag": "warp-in",
            "port": 40000,
            "listen": "127.0.0.1",
            "protocol": "socks",
            "settings": {
                "udp": true
            }
        });

        // Routing Rules
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
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn get_warp_routing_rules() -> Result<(Vec<String>, WarpMode)> {
        let config_path = format!("{}/10_warp_routing.json", xray::CONF_DIR);
        if !Path::new(&config_path).exists() {
            return Ok((Vec::new(), WarpMode::Default));
        }

        let content = fs::read_to_string(&config_path).await?;
        let v: Value = serde_json::from_str(&content)?;

        // Extract rules: Find the rule with "domain" field
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

        // Extract IP mode
        // Logic: Check if there is a "freedom" outbound with tag "warp".
        // If yes, check its domainStrategy. If no, it's Default.
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

async fn run_wwps_core_cmd(args: &[&str]) -> Result<String> {
    let (status, stdout, stderr) = run_cmd_output(
        xray::BIN,
        args,
        ConfigManager::TIMEOUT_WWPS_CORE,
    )
    .await?;

    if status.success() {
        Ok(stdout)
    } else {
        anyhow::bail!("wwps-core {:?} 执行失败: {}", args, stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use base64::engine::general_purpose;
    use percent_encoding::percent_decode_str;

    #[test]
    fn test_header_size_values() {
        assert_eq!(KcpMask::MkcpOriginal.header_size(), Some(6));
        assert_eq!(KcpMask::MkcpAes128Gcm { password: "test".to_string() }.header_size(), Some(28));
        assert_eq!(KcpMask::Salamander { password: "test".to_string() }.header_size(), Some(8));
        assert_eq!(KcpMask::HeaderDns { domain: "example.com".to_string() }.header_size(), Some(29));
        assert_eq!(KcpMask::HeaderWechat.header_size(), Some(13));
        assert_eq!(KcpMask::HeaderSrtp.header_size(), Some(4));
        assert_eq!(KcpMask::HeaderUtp.header_size(), Some(4));
        assert_eq!(KcpMask::HeaderDtls.header_size(), Some(13));
        assert_eq!(KcpMask::HeaderWireguard.header_size(), Some(4));
        assert_eq!(KcpMask::HeaderCustom.header_size(), Some(4));

        assert_eq!(KcpMask::Noise.header_size(), None);
        assert_eq!(KcpMask::Sudoku { password: "test".to_string() }.header_size(), None);
        assert_eq!(KcpMask::Xdns { domains: vec![], resolvers: vec![] }.header_size(), None);
        assert_eq!(KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }.header_size(), None);
    }

    #[test]
    fn test_dns_header_size_dynamic() {
        assert_eq!(KcpMask::HeaderDns { domain: "a.io".to_string() }.header_size(), Some(22));
        assert_eq!(KcpMask::HeaderDns { domain: "example.com".to_string() }.header_size(), Some(29));
        assert_eq!(KcpMask::HeaderDns { domain: "sub.domain.example.com".to_string() }.header_size(), Some(40));
    }

    #[test]
    fn test_reality_pq_verify_as_base64url() {
        // Standard base64 含 +/ 的输入应转为 URL-safe 输出
        let bytes_with_special = b"\xfc\xfd\xfe\xff";
        let std_b64 = general_purpose::STANDARD.encode(bytes_with_special);
        let out = reality_pq_verify_as_base64url(&std_b64).expect("应成功转换");
        assert!(!out.contains('+'));
        assert!(!out.contains('/'));
        assert_eq!(
            general_purpose::URL_SAFE_NO_PAD.decode(&out).ok(),
            Some(bytes_with_special.to_vec())
        );

        // URL-safe 输入应保持 URL-safe
        let url_b64 = general_purpose::URL_SAFE_NO_PAD.encode(b"world");
        let out2 = reality_pq_verify_as_base64url(&url_b64).expect("应成功转换");
        assert_eq!(out2, url_b64);

        // 空或无效输入应返回 None
        assert!(reality_pq_verify_as_base64url("").is_none());
        assert!(reality_pq_verify_as_base64url("!!!").is_none());
    }

    #[test]
    fn test_build_reality_vless_inbound_architecture() {
        let tag = "TEST_TAG";
        let port = 443;
        let uuid = "test-uuid";
        let email = "test-email";
        let sni = "example.com";
        let pub_key = "pub";
        let priv_key = "priv";
        let short_id = "sid";

        let vless = ConfigManager::build_reality_vless_inbound(
            tag,
            port,
            uuid,
            email,
            sni,
            pub_key,
            priv_key,
            short_id,
            IpVersion::IPv4,
            Proto::Vision,
            None,
            false,
        );

        // 验证架构合规性
        assert_eq!(vless["listen"], "0.0.0.0", "必须使用 Direct Listen 0.0.0.0");
        assert_eq!(vless["protocol"], "vless");
        assert_eq!(vless["streamSettings"]["security"], "reality");

        // 验证没有 Dokodemo 相关的残留
        assert!(
            vless.get("settings").unwrap().get("address").is_none(),
            "不能包含 dokodemo address 设置"
        );

        // 验证关键参数
        assert_eq!(vless["settings"]["clients"][0]["id"], "test-uuid");
        assert_eq!(
            vless["streamSettings"]["realitySettings"]["dest"],
            "example.com:443"
        );
        assert_eq!(
            vless["streamSettings"]["realitySettings"]["serverNames"][0],
            "example.com"
        );
    }

    #[test]
    fn test_xhttp_dynamic_path() {
        let tag = "XHTTP_TAG";
        let port = 8443;
        let uuid = "xhttp-uuid";
        let email = "xhttp-email";
        let sni = "google.com";
        let pub_key = "pbk";
        let priv_key = "prk";
        let short_id = "sid";
        let path = "/xhttp_random123";

        let vless = ConfigManager::build_reality_vless_inbound(
            tag,
            port,
            uuid,
            email,
            sni,
            pub_key,
            priv_key,
            short_id,
            IpVersion::IPv4,
            Proto::XHTTP,
            Some(path),
            false,
        );

        assert_eq!(vless["streamSettings"]["network"], "xhttp");
        assert_eq!(vless["streamSettings"]["xhttpSettings"]["path"], path);
        assert_eq!(vless["streamSettings"]["xhttpSettings"]["mode"], "auto");
        // 验证 XHTTP 没有 flow
        assert!(vless["settings"]["clients"][0].get("flow").is_none());
    }

    #[test]
    fn test_resolve_public_hosts_rejects_missing_ipv4_for_ipv4_mode() {
        let result = ConfigManager::resolve_public_hosts(
            IpVersion::IPv4,
            Err(anyhow!("missing ipv4")),
            Ok("::1".to_string()),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_public_hosts_requires_both_families_for_split_stack() {
        let result = ConfigManager::resolve_public_hosts(
            IpVersion::SplitStackV6Primary,
            Err(anyhow!("missing ipv4")),
            Ok("2001:db8::1".to_string()),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_public_hosts_requires_both_families_for_split_stack_v4_primary() {
        let result = ConfigManager::resolve_public_hosts(
            IpVersion::SplitStackV4Primary,
            Ok("198.51.100.1".to_string()),
            Err(anyhow!("missing ipv6")),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_generate_client_link_xhttp_split_v6_primary_formats_remote_and_download_address() {
        let uuid = "11111111-1111-1111-1111-111111111111";
        let host_v6 = "2001:db8::10";
        let port = 443;
        let sni = "example.com";
        let pbk = "pub_key";
        let sid = "abcd1234";
        let email = "test-user";
        let path = "/xhttp_path";
        let host_v4_secondary = "198.51.100.10";

        let link = ConfigManager::generate_client_link(
            uuid,
            host_v6,
            port,
            sni,
            pbk,
            sid,
            email,
            IpVersion::SplitStackV6Primary,
            Proto::XHTTP,
            Some(path),
            Some(host_v4_secondary),
            false,
        );

        assert!(
            link.contains(&format!("vless://{}@[{}]:{}", uuid, host_v6, port)),
            "remote-host 应为方括号包裹的 IPv6"
        );
        assert!(link.contains("type=xhttp"));
        assert!(link.contains("security=reality"));
        assert!(link.contains("mode=auto"));
        assert!(link.contains("&extra="));

        // 提取 extra 参数值（可能其后有 &pqv=，需只取到下一个 &）
        let extra_encoded = link
            .split("&extra=")
            .nth(1)
            .and_then(|s| s.split('#').next())
            .and_then(|s| s.split('&').next())
            .expect("应存在 extra 参数");
        let extra_decoded = percent_decode_str(extra_encoded)
            .decode_utf8()
            .expect("extra 应可解码");
        let extra_json: Value = serde_json::from_str(&extra_decoded).expect("extra 应为合法 JSON");

        assert_eq!(
            extra_json["downloadSettings"]["address"], host_v4_secondary,
            "v6上v4下时，downloadSettings.address 应为 IPv4"
        );
        assert_eq!(extra_json["downloadSettings"]["port"], port);
        assert_eq!(extra_json["downloadSettings"]["network"], "xhttp");
        assert_eq!(extra_json["downloadSettings"]["security"], "reality");
        assert_eq!(
            extra_json["downloadSettings"]["realitySettings"]["serverName"],
            sni
        );
    }

    #[test]
    fn test_generate_client_link_xhttp_split_v4_primary_formats_remote_and_download_address() {
        let uuid = "22222222-2222-2222-2222-222222222222";
        let host_v4 = "198.51.100.20";
        let port = 443;
        let sni = "example.org";
        let pbk = "pub_key_2";
        let sid = "ef567890";
        let email = "test-user-2";
        let path = "/xhttp_path2";
        let host_v6_secondary = "2001:db8::20";

        let link = ConfigManager::generate_client_link(
            uuid,
            host_v4,
            port,
            sni,
            pbk,
            sid,
            email,
            IpVersion::SplitStackV4Primary,
            Proto::XHTTP,
            Some(path),
            Some(host_v6_secondary),
            false,
        );

        assert!(
            link.contains(&format!("vless://{}@{}:{}", uuid, host_v4, port)),
            "remote-host 应为 IPv4 且不带方括号"
        );
        assert!(!link.contains(&format!("@[{}]:", host_v4)));
        assert!(link.contains("&extra="));

        // 提取 extra 参数值（可能其后有 &pqv=，需只取到下一个 &）
        let extra_encoded = link
            .split("&extra=")
            .nth(1)
            .and_then(|s| s.split('#').next())
            .and_then(|s| s.split('&').next())
            .expect("应存在 extra 参数");
        let extra_decoded = percent_decode_str(extra_encoded)
            .decode_utf8()
            .expect("extra 应可解码");
        let extra_json: Value = serde_json::from_str(&extra_decoded).expect("extra 应为合法 JSON");

        assert_eq!(
            extra_json["downloadSettings"]["address"], host_v6_secondary,
            "v4上v6下时，downloadSettings.address 应为 IPv6"
        );
        assert_eq!(extra_json["downloadSettings"]["port"], port);
        assert_eq!(extra_json["downloadSettings"]["network"], "xhttp");
        assert_eq!(extra_json["downloadSettings"]["security"], "reality");
        assert_eq!(
            extra_json["downloadSettings"]["realitySettings"]["serverName"],
            sni
        );
    }

    #[test]
    fn test_kcp_mask_variants_count() {
        assert_eq!(KcpMask::all_variants().len(), 14);
    }

    #[test]
    fn test_kcp_mask_code_roundtrip() {
        let codes = [
            "mo", "ma", "no", "sa", "su",
            "hd", "hw", "hs", "hu", "hdt", "hwg",
            "xd", "xi", "hc",
        ];
        for code in codes {
            let mask = KcpMask::from_code(code);
            assert!(mask.is_some(), "Failed to parse mask code: {}", code);
            assert_eq!(mask.unwrap().code(), code);
        }
    }

    #[test]
    fn test_kcp_mask_type_str() {
        assert_eq!(KcpMask::MkcpOriginal.type_str(), "mkcp-original");
        assert_eq!(KcpMask::MkcpAes128Gcm { password: "x".into() }.type_str(), "mkcp-aes128gcm");
        assert_eq!(KcpMask::Noise.type_str(), "noise");
        assert_eq!(KcpMask::Salamander { password: "x".into() }.type_str(), "salamander");
        assert_eq!(KcpMask::Sudoku { password: "x".into() }.type_str(), "sudoku");
        assert_eq!(KcpMask::HeaderDns { domain: "x".into() }.type_str(), "header-dns");
        assert_eq!(KcpMask::HeaderWechat.type_str(), "header-wechat");
        assert_eq!(KcpMask::HeaderSrtp.type_str(), "header-srtp");
        assert_eq!(KcpMask::HeaderUtp.type_str(), "header-utp");
        assert_eq!(KcpMask::HeaderDtls.type_str(), "header-dtls");
        assert_eq!(KcpMask::HeaderWireguard.type_str(), "header-wireguard");
        assert_eq!(KcpMask::Xdns { domains: vec![], resolvers: vec![] }.type_str(), "xdns");
        assert_eq!(KcpMask::Xicmp { listen_ip: "0.0.0.0".into(), id: 0 }.type_str(), "xicmp");
        assert_eq!(KcpMask::HeaderCustom.type_str(), "header-custom");
    }

    #[test]
    fn test_kcp_mask_as_json_original() {
        let mask = KcpMask::MkcpOriginal;
        let json = mask.as_json();
        assert_eq!(json["type"], "mkcp-original");
        assert!(json.get("settings").is_none());
    }

    #[test]
    fn test_kcp_mask_as_json_aes128gcm() {
        let mask = KcpMask::MkcpAes128Gcm { password: "testpass".into() };
        let json = mask.as_json();
        assert_eq!(json["type"], "mkcp-aes128gcm");
        assert_eq!(json["settings"]["password"], "testpass");
    }

    #[test]
    fn test_kcp_mask_as_json_noise() {
        let mask = KcpMask::Noise;
        let json = mask.as_json();
        assert_eq!(json["type"], "noise");
        assert!(json.get("settings").is_none());
    }

    #[test]
    fn test_kcp_mask_as_json_salamander() {
        let mask = KcpMask::Salamander { password: "salpass".into() };
        let json = mask.as_json();
        assert_eq!(json["type"], "salamander");
        assert_eq!(json["settings"]["password"], "salpass");
    }

    #[test]
    fn test_kcp_mask_as_json_sudoku() {
        let mask = KcpMask::Sudoku { password: "sudpass".into() };
        let json = mask.as_json();
        assert_eq!(json["type"], "sudoku");
        assert_eq!(json["settings"]["password"], "sudpass");
    }

    #[test]
    fn test_kcp_mask_as_json_xdns() {
        let mask = KcpMask::Xdns {
            domains: vec!["www.baidu.com".into()],
            resolvers: vec!["+udp://1.1.1.1".into()],
        };
        let json = mask.as_json();
        assert_eq!(json["type"], "xdns");
        assert_eq!(json["settings"]["domains"][0], "www.baidu.com");
        assert_eq!(json["settings"]["resolvers"][0], "+udp://1.1.1.1");
    }

    #[test]
    fn test_kcp_mask_as_json_xicmp() {
        let mask = KcpMask::Xicmp { listen_ip: "0.0.0.0".into(), id: 12345 };
        let json = mask.as_json();
        assert_eq!(json["type"], "xicmp");
        assert_eq!(json["settings"]["listenIp"], "0.0.0.0");
        assert_eq!(json["settings"]["id"], 12345);
    }

    #[test]
    fn test_kcp_mask_as_json_header_custom() {
        let mask = KcpMask::HeaderCustom;
        let json = mask.as_json();
        assert_eq!(json["type"], "header-custom");
        assert!(json.get("settings").is_none());
    }

    #[test]
    fn test_kcp_mask_from_code_aes_generates_password() {
        let mask = KcpMask::from_code("ma").unwrap();
        if let KcpMask::MkcpAes128Gcm { password } = mask {
            assert!(!password.is_empty(), "AES password should be auto-generated");
        } else {
            panic!("Expected MkcpAes128Gcm variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_salamander_generates_password() {
        let mask = KcpMask::from_code("sa").unwrap();
        if let KcpMask::Salamander { password } = mask {
            assert!(!password.is_empty(), "Salamander password should be auto-generated");
        } else {
            panic!("Expected Salamander variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_sudoku_generates_password() {
        let mask = KcpMask::from_code("su").unwrap();
        if let KcpMask::Sudoku { password } = mask {
            assert!(!password.is_empty(), "Sudoku password should be auto-generated");
        } else {
            panic!("Expected Sudoku variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_dns_default_domain() {
        let mask = KcpMask::from_code("hd").unwrap();
        if let KcpMask::HeaderDns { domain } = mask {
            assert_eq!(domain, "www.baidu.com");
        } else {
            panic!("Expected HeaderDns variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_xdns_defaults() {
        let mask = KcpMask::from_code("xd").unwrap();
        if let KcpMask::Xdns { domains, resolvers } = mask {
            assert_eq!(domains, vec!["www.baidu.com"]);
            assert_eq!(resolvers, vec!["+udp://1.1.1.1"]);
        } else {
            panic!("Expected Xdns variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_xicmp_defaults() {
        let mask = KcpMask::from_code("xi").unwrap();
        if let KcpMask::Xicmp { listen_ip, id } = mask {
            assert_eq!(listen_ip, "0.0.0.0");
            assert!(id > 0, "Xicmp ID should be auto-generated");
        } else {
            panic!("Expected Xicmp variant");
        }
    }

    #[test]
    fn test_kcp_mask_from_code_invalid() {
        assert!(KcpMask::from_code("invalid").is_none());
        assert!(KcpMask::from_code("").is_none());
    }

    #[test]
    fn test_kcp_mask_brief_all_variants() {
        let variants = KcpMask::all_variants();
        assert_eq!(variants.len(), 14);
        for m in &variants {
            let brief = m.brief();
            assert!(!brief.is_empty(), "brief should not be empty for {:?}", m);
        }
    }

    #[test]
    fn test_kcp_mask_category_code_all_variants() {
        for m in KcpMask::all_variants() {
            let code = m.category_code();
            assert!(
                code == "enc" || code == "obf" || code == "dis" || code == "ext",
                "category_code should be enc/obf/dis/ext for {:?}, got {}",
                m,
                code
            );
        }
    }

    #[test]
    fn test_kcp_mask_category_code_unique() {
        let mut codes: Vec<&str> = KcpMask::all_variants()
            .iter()
            .map(|m| m.category_code())
            .collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 4, "should have exactly 4 unique category codes");
    }

    #[test]
    fn test_kcp_mask_category_code_matches_category() {
        for m in KcpMask::all_variants() {
            let cat = m.category();
            let code = m.category_code();
            match code {
                "enc" => assert!(cat.contains("加密"), "enc should map to 加密层: {}", cat),
                "obf" => assert!(cat.contains("混淆"), "obf should map to 混淆层: {}", cat),
                "dis" => assert!(cat.contains("伪装"), "dis should map to 伪装层: {}", cat),
                "ext" => assert!(cat.contains("扩展"), "ext should map to 扩展层: {}", cat),
                _ => panic!("unexpected code: {}", code),
            }
        }
    }

    #[test]
    fn test_variants_by_category_count() {
        assert_eq!(KcpMask::variants_by_category("enc").len(), 2);
        assert_eq!(KcpMask::variants_by_category("obf").len(), 3);
        assert_eq!(KcpMask::variants_by_category("dis").len(), 6);
        assert_eq!(KcpMask::variants_by_category("ext").len(), 3);
    }

    #[test]
    fn test_variants_by_category_invalid() {
        assert!(KcpMask::variants_by_category("invalid").is_empty());
        assert!(KcpMask::variants_by_category("").is_empty());
    }

    #[test]
    fn test_category_from_code() {
        assert_eq!(KcpMask::category_from_code("enc"), Some("🔐 加密层"));
        assert_eq!(KcpMask::category_from_code("obf"), Some("🌀 混淆层"));
        assert_eq!(KcpMask::category_from_code("dis"), Some("🎭 伪装层"));
        assert_eq!(KcpMask::category_from_code("ext"), Some("⚡ 扩展层"));
        assert_eq!(KcpMask::category_from_code("xx"), None);
    }

    #[test]
    fn test_category_from_code_roundtrip() {
        for code in ["enc", "obf", "dis", "ext"] {
            let cat = KcpMask::category_from_code(code).unwrap();
            for m in KcpMask::variants_by_category(code) {
                assert_eq!(m.category(), cat);
                assert_eq!(m.category_code(), code);
            }
        }
    }

    #[test]
    fn test_category_buttons_count_matches_variant_counts() {
        let enc = KcpMask::variants_by_category("enc").len();
        let obf = KcpMask::variants_by_category("obf").len();
        let dis = KcpMask::variants_by_category("dis").len();
        let ext = KcpMask::variants_by_category("ext").len();
        assert_eq!(enc + obf + dis + ext, 14, "total variants should be 14");
    }

    #[test]
    fn test_build_kcp_inbound_original_srtp() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST",
            34456,
            "test-uuid",
            "test-email",
            IpVersion::IPv4,
            &masks,
        );

        assert_eq!(config["listen"], "0.0.0.0");
        assert_eq!(config["port"], 34456);
        assert_eq!(config["protocol"], "vless");
        assert_eq!(config["tag"], "KCP-TEST");

        let ss = &config["streamSettings"];
        assert_eq!(ss["network"], "kcp");
        assert_eq!(ss["kcpSettings"]["mtu"], 1350);
        assert_eq!(ss["kcpSettings"]["tti"], 50);
        assert_eq!(ss["kcpSettings"]["uplinkCapacity"], 5);
        assert_eq!(ss["kcpSettings"]["downlinkCapacity"], 20);
        assert_eq!(ss["kcpSettings"]["cwndMultiplier"], 1);
        assert_eq!(ss["kcpSettings"]["maxSendingWindow"], 2097152);
        assert_eq!(ss["security"], "none");

        let udp = &ss["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "mkcp-original");
        assert!(udp[0].get("settings").is_none());
    }

    #[test]
    fn test_build_kcp_inbound_aes_wechat() {
        let masks = vec![KcpMask::MkcpAes128Gcm {
            password: "secretpass".to_string(),
        }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST",
            34456,
            "test-uuid",
            "test-email",
            IpVersion::IPv6,
            &masks,
        );

        assert_eq!(config["listen"], "::");
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "mkcp-aes128gcm");
        assert_eq!(udp[0]["settings"]["password"], "secretpass");
    }

    #[test]
    fn test_build_kcp_inbound_aes_dns() {
        let masks = vec![KcpMask::MkcpAes128Gcm {
            password: "mypassword".to_string(),
        }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST",
            34456,
            "test-uuid",
            "test-email",
            IpVersion::IPv4,
            &masks,
        );

        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "mkcp-aes128gcm");
        assert_eq!(udp[0]["settings"]["password"], "mypassword");
    }

    #[test]
    fn test_kcp_no_reality_settings() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST",
            34456,
            "test-uuid",
            "test-email",
            IpVersion::IPv4,
            &masks,
        );

        assert!(config["streamSettings"].get("realitySettings").is_none());
        assert!(config["streamSettings"].get("tlsSettings").is_none());
    }

    #[test]
    fn test_kcp_no_old_fields() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST",
            34456,
            "test-uuid",
            "test-email",
            IpVersion::IPv4,
            &masks,
        );

        let kcp = &config["streamSettings"]["kcpSettings"];
        assert!(kcp.get("congestion").is_none(), "congestion should be removed");
        assert!(kcp.get("readBufferSize").is_none(), "readBufferSize should be removed");
        assert!(kcp.get("writeBufferSize").is_none(), "writeBufferSize should be removed");
    }

    #[test]
    fn test_generate_kcp_client_link_dual_layer() {
        let masks = vec![KcpMask::MkcpAes128Gcm {
            password: "testpass".to_string(),
        }];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid",
            "192.168.1.1",
            34456,
            "test-user",
            IpVersion::IPv4,
            &masks,
        );

        assert!(link.starts_with("vless://test-uuid@192.168.1.1:34456"));
        assert!(link.contains("type=kcp"));
        assert!(link.contains("security=none"));
        assert!(link.contains("fm="));
        assert!(link.contains("#test%2Duser"));
        assert!(!link.contains("sni="));
        assert!(!link.contains("pbk="));

        let fm_start = link.find("fm=").unwrap() + 3;
        let fm_end = link.find('#').unwrap();
        let fm_encoded = &link[fm_start..fm_end];
        let fm_decoded = percent_decode_str(fm_encoded).decode_utf8().unwrap();
        let fm_json: Value = serde_json::from_str(&fm_decoded).unwrap();
        assert_eq!(fm_json["udp"][0]["type"], "mkcp-aes128gcm");
        assert_eq!(fm_json["udp"][0]["settings"]["password"], "testpass");
    }

    #[test]
    fn test_generate_kcp_client_link_original_wireguard() {
        let masks = vec![KcpMask::MkcpOriginal];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid",
            "192.168.1.1",
            34456,
            "test-user",
            IpVersion::IPv4,
            &masks,
        );

        let fm_start = link.find("fm=").unwrap() + 3;
        let fm_end = link.find('#').unwrap();
        let fm_encoded = &link[fm_start..fm_end];
        let fm_decoded = percent_decode_str(fm_encoded).decode_utf8().unwrap();
        let fm_json: Value = serde_json::from_str(&fm_decoded).unwrap();
        assert_eq!(fm_json["udp"][0]["type"], "mkcp-original");
    }

    #[test]
    fn test_build_kcp_inbound_masks_slice() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "testpass".into() },
            KcpMask::HeaderSrtp,
        ];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "mkcp-aes128gcm");
        assert_eq!(udp[0]["settings"]["password"], "testpass");
        assert_eq!(udp[1]["type"], "header-srtp");
        assert!(udp[1].get("settings").is_none());
    }

    #[test]
    fn test_build_kcp_inbound_single_mask() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp.as_array().unwrap().len(), 1);
        assert_eq!(udp[0]["type"], "mkcp-original");
    }

    #[test]
    fn test_build_kcp_inbound_three_masks() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "pw".into() },
            KcpMask::Noise,
            KcpMask::HeaderDtls,
        ];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv6, &masks,
        );
        assert_eq!(config["listen"], "::");
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp.as_array().unwrap().len(), 3);
        assert_eq!(udp[0]["type"], "mkcp-aes128gcm");
        assert_eq!(udp[1]["type"], "noise");
        assert_eq!(udp[2]["type"], "header-dtls");
    }

    #[test]
    fn test_build_kcp_inbound_split_stack_v4() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::SplitStackV4Primary, &masks,
        );
        assert_eq!(config["listen"], "0.0.0.0");
    }

    #[test]
    fn test_build_kcp_inbound_split_stack_v6() {
        let masks = vec![KcpMask::MkcpOriginal];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::SplitStackV6Primary, &masks,
        );
        assert_eq!(config["listen"], "::");
    }

    #[test]
    fn test_generate_kcp_client_link_masks_slice() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "testpass".into() },
            KcpMask::HeaderDns { domain: "dns.google".into() },
        ];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid", "192.168.1.1", 34456, "test-user",
            IpVersion::IPv4, &masks,
        );
        assert!(link.starts_with("vless://test-uuid@192.168.1.1:34456"));
        assert!(link.contains("type=kcp"));
        assert!(link.contains("security=none"));
        assert!(link.contains("fm="));

        let fm_start = link.find("fm=").unwrap() + 3;
        let fm_end = link.find('#').unwrap();
        let fm_encoded = &link[fm_start..fm_end];
        let fm_decoded = percent_decode_str(fm_encoded).decode_utf8().unwrap();
        let fm_json: Value = serde_json::from_str(&fm_decoded).unwrap();
        assert_eq!(fm_json["udp"][0]["type"], "mkcp-aes128gcm");
        assert_eq!(fm_json["udp"][0]["settings"]["password"], "testpass");
        assert_eq!(fm_json["udp"][1]["type"], "header-dns");
        assert_eq!(fm_json["udp"][1]["settings"]["domain"], "dns.google");
    }

    #[test]
    fn test_generate_kcp_client_link_ipv6() {
        let masks = vec![KcpMask::MkcpOriginal, KcpMask::HeaderWechat];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid", "2001:db8::1", 34456, "test-user",
            IpVersion::IPv6, &masks,
        );
        assert!(link.starts_with("vless://test-uuid@[2001:db8::1]:34456"));
    }

    #[test]
    fn test_generate_kcp_client_link_split_stack_v4() {
        let masks = vec![KcpMask::MkcpOriginal];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid", "1.2.3.4", 34456, "test-user",
            IpVersion::SplitStackV4Primary, &masks,
        );
        assert!(link.starts_with("vless://test-uuid@1.2.3.4:34456"));
    }

    #[test]
    fn test_generate_kcp_client_link_split_stack_v6() {
        let masks = vec![KcpMask::MkcpOriginal];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid", "2001:db8::1", 34456, "test-user",
            IpVersion::SplitStackV6Primary, &masks,
        );
        assert!(link.starts_with("vless://test-uuid@[2001:db8::1]:34456"));
    }

    #[test]
    fn test_kcp_mask_noise_json() {
        let masks = vec![KcpMask::MkcpOriginal, KcpMask::Noise];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "mkcp-original");
        assert_eq!(udp[1]["type"], "noise");
        assert!(udp[1].get("settings").is_none());
    }

    #[test]
    fn test_kcp_mask_salamander_json() {
        let masks = vec![KcpMask::Salamander { password: "salpass".into() }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "salamander");
        assert_eq!(udp[0]["settings"]["password"], "salpass");
    }

    #[test]
    fn test_kcp_mask_sudoku_json() {
        let masks = vec![KcpMask::Sudoku { password: "sudpass".into() }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "sudoku");
        assert_eq!(udp[0]["settings"]["password"], "sudpass");
    }

    #[test]
    fn test_kcp_mask_xdns_json() {
        let masks = vec![KcpMask::Xdns {
            domains: vec!["dns.google".into(), "cloudflare.com".into()],
            resolvers: vec!["+udp://8.8.8.8".into()],
        }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "xdns");
        assert_eq!(udp[0]["settings"]["domains"][0], "dns.google");
        assert_eq!(udp[0]["settings"]["domains"][1], "cloudflare.com");
        assert_eq!(udp[0]["settings"]["resolvers"][0], "+udp://8.8.8.8");
    }

    #[test]
    fn test_kcp_mask_xicmp_json() {
        let masks = vec![KcpMask::Xicmp { listen_ip: "0.0.0.0".into(), id: 99999 }];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "xicmp");
        assert_eq!(udp[0]["settings"]["listenIp"], "0.0.0.0");
        assert_eq!(udp[0]["settings"]["id"], 99999);
    }

    #[test]
    fn test_kcp_mask_header_custom_json() {
        let masks = vec![KcpMask::HeaderCustom];
        let config = ConfigManager::build_kcp_inbound(
            "KCP-TEST", 34456, "test-uuid", "test-email",
            IpVersion::IPv4, &masks,
        );
        let udp = &config["streamSettings"]["finalmask"]["udp"];
        assert_eq!(udp[0]["type"], "header-custom");
        assert!(udp[0].get("settings").is_none());
    }

    #[test]
    fn test_kcp_client_link_with_noise_and_salamander() {
        let masks = vec![KcpMask::Noise, KcpMask::Salamander { password: "salpw".into() }];
        let link = ConfigManager::generate_kcp_client_link(
            "test-uuid", "1.2.3.4", 443, "test-user",
            IpVersion::IPv4, &masks,
        );
        let fm_start = link.find("fm=").unwrap() + 3;
        let fm_end = link.find('#').unwrap();
        let fm_encoded = &link[fm_start..fm_end];
        let fm_decoded = percent_decode_str(fm_encoded).decode_utf8().unwrap();
        let fm_json: Value = serde_json::from_str(&fm_decoded).unwrap();
        assert_eq!(fm_json["udp"][0]["type"], "noise");
        assert_eq!(fm_json["udp"][1]["type"], "salamander");
        assert_eq!(fm_json["udp"][1]["settings"]["password"], "salpw");
    }

    #[test]
    fn test_kcp_parse_codes_valid() {
        let masks = KcpMask::parse_codes(&["ma", "hd"]).unwrap();
        assert_eq!(masks.len(), 2);
        assert_eq!(masks[0].code(), "ma");
        assert_eq!(masks[1].code(), "hd");
    }

    #[test]
    fn test_kcp_parse_codes_invalid() {
        let result = KcpMask::parse_codes(&["ma", "invalid"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_kcp_parse_codes_three_masks() {
        let masks = KcpMask::parse_codes(&["mo", "no", "hw"]).unwrap();
        assert_eq!(masks.len(), 3);
        assert_eq!(masks[0].code(), "mo");
        assert_eq!(masks[1].code(), "no");
        assert_eq!(masks[2].code(), "hw");
    }

    #[test]
    fn test_is_header_conn_classification() {
        assert!(KcpMask::MkcpOriginal.is_header_conn());
        assert!(KcpMask::MkcpAes128Gcm { password: "test".to_string() }.is_header_conn());
        assert!(KcpMask::Salamander { password: "test".to_string() }.is_header_conn());
        assert!(KcpMask::HeaderDns { domain: "example.com".to_string() }.is_header_conn());
        assert!(KcpMask::HeaderWechat.is_header_conn());
        assert!(KcpMask::HeaderSrtp.is_header_conn());
        assert!(KcpMask::HeaderUtp.is_header_conn());
        assert!(KcpMask::HeaderDtls.is_header_conn());
        assert!(KcpMask::HeaderWireguard.is_header_conn());
        assert!(KcpMask::HeaderCustom.is_header_conn());

        assert!(!KcpMask::Noise.is_header_conn());
        assert!(!KcpMask::Sudoku { password: "test".to_string() }.is_header_conn());
        assert!(!KcpMask::Xdns { domains: vec![], resolvers: vec![] }.is_header_conn());
        assert!(!KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }.is_header_conn());
    }

    #[test]
    fn test_is_disguise_header_includes_custom() {
        assert!(KcpMask::HeaderCustom.is_disguise_header());
        assert!(KcpMask::HeaderDns { domain: "example.com".to_string() }.is_disguise_header());
        assert!(KcpMask::HeaderWechat.is_disguise_header());
        assert!(KcpMask::HeaderSrtp.is_disguise_header());
        assert!(KcpMask::HeaderUtp.is_disguise_header());
        assert!(KcpMask::HeaderDtls.is_disguise_header());
        assert!(KcpMask::HeaderWireguard.is_disguise_header());

        assert!(!KcpMask::MkcpOriginal.is_disguise_header());
        assert!(!KcpMask::Salamander { password: "test".to_string() }.is_disguise_header());
        assert!(!KcpMask::Noise.is_disguise_header());
    }

    #[test]
    fn test_canonical_order_transport_replacement_first() {
        let masks = vec![
            KcpMask::HeaderSrtp,
            KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        assert!(ordered[0].is_xicmp());
    }

    #[test]
    fn test_canonical_order_sudoku_last() {
        let masks = vec![
            KcpMask::Sudoku { password: "test".to_string() },
            KcpMask::HeaderSrtp,
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        assert!(ordered.last().unwrap().is_sudoku());
    }

    #[test]
    fn test_canonical_order_encryption_after_disguise() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::HeaderSrtp,
        ];
        let ordered = KcpMask::canonical_order(&masks);
        let enc_pos = ordered.iter().position(|m| m.is_encryption()).unwrap();
        let dis_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
        assert!(dis_pos < enc_pos, "disguise header should come before encryption");
    }

    #[test]
    fn test_canonical_order_salamander_after_disguise_before_encryption() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::Salamander { password: "test".to_string() },
            KcpMask::HeaderDns { domain: "example.com".to_string() },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        let dis_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
        let sal_pos = ordered.iter().position(|m| matches!(m, KcpMask::Salamander { .. })).unwrap();
        let enc_pos = ordered.iter().position(|m| m.is_encryption()).unwrap();
        assert!(dis_pos < sal_pos, "disguise should be before salamander");
        assert!(sal_pos < enc_pos, "salamander should be before encryption");
    }

    #[test]
    fn test_canonical_order_noise_after_transport_before_headers() {
        let masks = vec![
            KcpMask::HeaderSrtp,
            KcpMask::Noise,
            KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        let xicmp_pos = ordered.iter().position(|m| m.is_xicmp()).unwrap();
        let noise_pos = ordered.iter().position(|m| matches!(m, KcpMask::Noise)).unwrap();
        let header_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
        assert!(xicmp_pos < noise_pos, "xicmp should be before noise");
        assert!(noise_pos < header_pos, "noise should be before disguise header");
    }

    #[test]
    fn test_canonical_order_full_stack() {
        let masks = vec![
            KcpMask::Sudoku { password: "test".to_string() },
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::HeaderDns { domain: "example.com".to_string() },
            KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
            KcpMask::Noise,
            KcpMask::Salamander { password: "test".to_string() },
        ];
        let ordered = KcpMask::canonical_order(&masks);
        assert!(ordered[0].is_xicmp());
        assert!(matches!(ordered[1], KcpMask::Noise));
        assert!(ordered[2].is_disguise_header());
        assert!(matches!(ordered[3], KcpMask::Salamander { .. }));
        assert!(ordered[4].is_encryption());
        assert!(ordered[5].is_sudoku());
    }

    #[test]
    fn test_canonical_order_simple_stack() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::HeaderSrtp,
        ];
        let ordered = KcpMask::canonical_order(&masks);
        assert!(ordered[0].is_disguise_header());
        assert!(ordered[1].is_encryption());
    }

    #[test]
    fn test_compatible_with_xicmp_not_first() {
        let existing = vec![KcpMask::HeaderSrtp];
        let xicmp = KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 };
        assert!(xicmp.is_compatible_with(&existing).is_err());
    }

    #[test]
    fn test_validate_stack_xdns_not_enforced_first() {
        let masks = vec![
            KcpMask::HeaderSrtp,
            KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] },
        ];
        assert!(KcpMask::validate_stack(&masks).is_ok());
    }

    #[test]
    fn test_validate_stack_sudoku_duplicate() {
        let masks = vec![
            KcpMask::Sudoku { password: "test1".to_string() },
            KcpMask::Sudoku { password: "test2".to_string() },
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_validate_stack_no_layer_limit() {
        let masks = vec![
            KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] },
            KcpMask::Noise,
            KcpMask::HeaderDns { domain: "a.com".to_string() },
            KcpMask::HeaderSrtp,
            KcpMask::Salamander { password: "test".to_string() },
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::Sudoku { password: "test".to_string() },
        ];
        assert!(KcpMask::validate_stack(&masks).is_ok());
    }

    #[test]
    fn test_compatible_with_xdns_xicmp_exclusive() {
        let existing = vec![KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }];
        let xdns = KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] };
        assert!(xdns.is_compatible_with(&existing).is_err());

        let existing2 = vec![KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] }];
        let xicmp = KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 };
        assert!(xicmp.is_compatible_with(&existing2).is_err());
    }

    #[test]
    fn test_compatible_with_duplicate_encryption() {
        let existing = vec![KcpMask::MkcpAes128Gcm { password: "test".to_string() }];
        let dup = KcpMask::MkcpOriginal;
        assert!(dup.is_compatible_with(&existing).is_err());
    }

    #[test]
    fn test_compatible_with_duplicate_sudoku() {
        let existing = vec![KcpMask::Sudoku { password: "test1".to_string() }];
        let dup = KcpMask::Sudoku { password: "test2".to_string() };
        assert!(dup.is_compatible_with(&existing).is_err());
    }

    #[test]
    fn test_compatible_with_duplicate_header() {
        let existing = vec![KcpMask::HeaderSrtp];
        let dup = KcpMask::HeaderSrtp;
        assert!(dup.is_compatible_with(&existing).is_err());
    }

    #[test]
    fn test_compatible_with_mkcp_original_alone() {
        let alone = KcpMask::MkcpOriginal;
        assert!(alone.is_compatible_with(&[]).is_err());

        let with_header = KcpMask::MkcpOriginal;
        let existing = vec![KcpMask::HeaderSrtp];
        assert!(with_header.is_compatible_with(&existing).is_ok());
    }

    #[test]
    fn test_validate_stack_xicmp_not_first() {
        let masks = vec![
            KcpMask::HeaderSrtp,
            KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_validate_stack_sudoku_not_last() {
        let masks = vec![
            KcpMask::Sudoku { password: "test".to_string() },
            KcpMask::HeaderSrtp,
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_validate_stack_encryption_before_disguise() {
        let masks = vec![
            KcpMask::MkcpAes128Gcm { password: "test".to_string() },
            KcpMask::HeaderSrtp,
        ];
        assert!(KcpMask::validate_stack(&masks).is_err());
    }

    #[test]
    fn test_validate_stack_header_overflow() {
        let masks: Vec<KcpMask> = (0..200)
            .map(|_| KcpMask::HeaderDns { domain: "sub.domain.example.com".to_string() })
            .collect();
        assert!(KcpMask::validate_stack(&masks).is_err());
    }
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
