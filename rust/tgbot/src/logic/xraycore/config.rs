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

use crate::core::paths::{warp, xray};
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::logic::cmd_async::run_cmd_output;

pub use super::kcp_mask::KcpMask;

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
pub struct ConfigManager;

impl ConfigManager {
    #[allow(dead_code)]
    const CONFIG_BASE_PATH: &'static str = xray::DIR;
    const TIMEOUT_WWPS_CORE: Duration = Duration::from_secs(5);

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
                    && !name.starts_with("00_")
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

    #[allow(clippy::too_many_arguments)]
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
        ip_version: IpVersion,
        mask_codes: &[&str],
    ) -> Result<BatchCreationResult> {
        let masks = KcpMask::parse_codes(mask_codes).map_err(|e| anyhow!("{}", e))?;

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

            let config = Self::build_kcp_inbound(&tag, port, &uuid, &email, ip_version, &masks);
            batch_configs.push(config);

            let link =
                Self::generate_kcp_client_link(&uuid, &host, port, &email, ip_version, &masks);
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        Self::create_standalone_config(batch_configs, links, Proto::Kcp).await
    }

    pub async fn batch_create_reality_vision_enhanced(
        count: usize,
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
            let sni = selector.get_next();

            // 判断当前 SNI 是否适合启用 PQ（证书链长度 + 公钥算法）。
            let pq_ok = crate::logic::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, Proto::Vision, preferred).await?;

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

        Self::create_standalone_config(batch_configs, links, Proto::Vision).await
    }

    pub async fn batch_create_xhttp_reality_enhanced(
        count: usize,
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
            let sni = selector.get_next();

            let pq_ok = crate::logic::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, Proto::XHTTP, preferred).await?;

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

        Self::create_standalone_config(batch_configs, links, Proto::XHTTP).await
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
                    if crate::logic::port_allocator::PortAllocator::is_port_in_locked_range(p).await
                    {
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

    #[allow(clippy::too_many_arguments)]
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

                if enable_pq && let Some(pqv) = reality_pq_verify_as_base64url(&REALITY_PQ_VERIFY) {
                    let encoded_pqv = utf8_percent_encode(&pqv, NON_ALPHANUMERIC).to_string();
                    link.push_str(&format!("&pqv={}", encoded_pqv));
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

                if enable_pq && let Some(pqv) = reality_pq_verify_as_base64url(&REALITY_PQ_VERIFY) {
                    let encoded_pqv = utf8_percent_encode(&pqv, NON_ALPHANUMERIC).to_string();
                    link.push_str(&format!("&pqv={}", encoded_pqv));
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

        // 只写入 inbounds 片段（00_base.json 提供 log/dns/outbounds/routing）
        let config = json!({
            "inbounds": configs
        });

        // 保存文件
        let content = serde_json::to_string_pretty(&config).context("序列化配置失败")?;
        fs::write(&config_path, content)
            .await
            .context("写入配置文件失败")?;
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count,
        })
    }

    #[allow(dead_code)]
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
            if let Ok(meta) = std::fs::metadata(&f)
                && let Ok(time) = meta.modified()
            {
                file_with_time.push((f, time));
            }
        }
        file_with_time.sort_by_key(|a| a.1);

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

    pub async fn ensure_base_config() -> Result<()> {
        use crate::core::paths::xray;

        if let Err(e) = crate::logic::maintenance::MaintenanceManager::ensure_geodata().await {
            log::warn!("确保 geodata 文件失败: {}", e);
        }

        let base_path = format!("{}/00_base.json", xray::CONF_DIR);

        let exists = match tokio::fs::try_exists(&base_path).await {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                log::warn!("检查基础配置存在性失败: {}", e);
                false
            }
        };
        if exists {
            return Ok(());
        }

        tokio::fs::create_dir_all(xray::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let base_config = serde_json::json!({
            "log": {"loglevel": "warning"},
            "dns": {
                "servers": ["https+local://1.1.1.1/dns-query", "https+local://8.8.8.8/dns-query"],
                "tag": "dns"
            },
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": [
                    {"type": "field", "ip": ["geoip:private"], "outboundTag": "blocked"}
                ]
            },
            "outbounds": [
                {"protocol": "freedom", "settings": {}, "tag": "direct"},
                {"protocol": "blackhole", "settings": {}, "tag": "blocked"}
            ]
        });

        let content = serde_json::to_string_pretty(&base_config).context("序列化基础配置失败")?;
        tokio::fs::write(&base_path, content)
            .await
            .context("写入基础配置失败")?;

        log::info!("已创建 wwps-core 基础配置: {}", base_path);
        Ok(())
    }
}

async fn run_wwps_core_cmd(args: &[&str]) -> Result<String> {
    let (status, stdout, stderr) =
        run_cmd_output(xray::BIN, args, ConfigManager::TIMEOUT_WWPS_CORE).await?;

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
            "ml", "mla", "no", "sa", "su", "mld", "mlw", "mls", "mlu", "mldt", "mlg", "xd", "xi",
            "rl",
        ];
        for code in codes {
            let mask = KcpMask::from_code(code);
            assert!(mask.is_some(), "Failed to parse mask code: {}", code);
            assert_eq!(mask.unwrap().code(), code);
        }
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

    #[tokio::test]
    async fn test_ensure_base_config_structure() {
        let base_config = serde_json::json!({
            "log": {"loglevel": "warning"},
            "dns": {
                "servers": ["https+local://1.1.1.1/dns-query", "https+local://8.8.8.8/dns-query"],
                "tag": "dns"
            },
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": [
                    {"type": "field", "ip": ["geoip:private"], "outboundTag": "blocked"}
                ]
            },
            "outbounds": [
                {"protocol": "freedom", "settings": {}, "tag": "direct"},
                {"protocol": "blackhole", "settings": {}, "tag": "blocked"}
            ]
        });

        assert!(base_config.get("log").is_some());
        assert!(base_config.get("dns").is_some());
        assert!(base_config.get("routing").is_some());
        assert!(base_config.get("outbounds").is_some());
        let rules = base_config["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
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
