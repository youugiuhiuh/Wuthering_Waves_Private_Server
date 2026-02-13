use anyhow::{Context, Result, anyhow};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tokio::fs;

use crate::logic::cmd_async::run_cmd_output;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IpVersion {
    IPv4,
    IPv6,
    SplitStack, // 新增: 上行 v6, 下行 v4 (Split-Traffic)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RealityProto {
    Vision,
    XHTTP,
}

#[derive(Debug, Clone)]
pub struct BatchCreationResult {
    pub links: Vec<String>,
    pub config_file: Option<String>, // 独立文件名
    pub backup_file: Option<String>, // 备份文件名
    pub created_count: usize,
}

pub struct ConfigManager;

impl ConfigManager {
    const CONFIG_BASE_PATH: &'static str = "/etc/wwps/";
    const WWPS_CORE_BIN: &'static str = "/etc/wwps/wwps-core/wwps-core";
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
        let dirs = vec![
            format!("{}wwps-core/conf/", Self::CONFIG_BASE_PATH),
            format!("{}wwps-box/conf/config/", Self::CONFIG_BASE_PATH),
        ];

        let mut out = Vec::new();
        for dir in dirs {
            if let Ok(mut rd) = fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    if let Some(name) = entry.file_name().to_str()
                        && name.ends_with("_inbounds.json")
                    {
                        out.push(entry.path().to_string_lossy().to_string());
                    }
                }
            }
        }
        Ok(out)
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
        let mut rng = rand::thread_rng();
        format!("{:016x}", rng.r#gen::<u64>())
    }

    fn generate_random_path() -> String {
        let mut rng = rand::thread_rng();
        let suffix: String = (0..5)
            .map(|_| {
                let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        format!("/xhttp_{}", suffix)
    }

    pub async fn generate_secure_batch_filename(proto: RealityProto) -> Result<String> {
        let uuid = Self::generate_wwps_uuid().await?;
        let uuid_short = Self::uuid_short_prefix(&uuid);
        let prefix = match proto {
            RealityProto::Vision => "batch_reality",
            RealityProto::XHTTP => "batch_xhttp",
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
        proto: RealityProto,
        path: Option<&str>,
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 => "0.0.0.0",
            IpVersion::IPv6 | IpVersion::SplitStack => "::", // SplitStack 模式上行为 v6，所以监听 ::
        };

        let client = if proto == RealityProto::Vision {
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
                RealityProto::Vision => "tcp",
                RealityProto::XHTTP => "xhttp",
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

        if proto == RealityProto::XHTTP {
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
                "metadataOnly": false // 使用 #4118 建议的配置
            }
        })
    }

    pub async fn batch_create_reality_vision_enhanced(
        count: usize,
        standalone: bool,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let host = match ip_version {
            IpVersion::IPv4 => crate::logic::system::SystemMonitor::get_public_ip().await,
            IpVersion::IPv6 | IpVersion::SplitStack => {
                crate::logic::system::SystemMonitor::get_public_ipv6()
                    .await
                    .unwrap_or_else(|_| "::1".to_string())
            }
        };

        let mut rng = StdRng::from_entropy();
        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = crate::logic::sni_selector::SNISelector::get_for_country(
            &country_code,
            RealityProto::Vision,
        );

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..count {
            let sni = selector.next();

            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, RealityProto::Vision).await?;

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
                RealityProto::Vision,
                path.as_deref(),
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
                RealityProto::Vision,
                path.as_deref(),
                None,
            );
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        if standalone {
            Self::create_standalone_config(batch_configs, links, RealityProto::Vision).await
        } else {
            Self::update_existing_config(batch_configs, links).await
        }
    }

    pub async fn batch_create_xhttp_reality_enhanced(
        count: usize,
        standalone: bool,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let (host, host_v4) = match ip_version {
            IpVersion::IPv4 => (
                crate::logic::system::SystemMonitor::get_public_ip().await,
                None,
            ),
            IpVersion::IPv6 => (
                crate::logic::system::SystemMonitor::get_public_ipv6()
                    .await
                    .unwrap_or_else(|_| "::1".to_string()),
                None,
            ),
            IpVersion::SplitStack => {
                let v6 = crate::logic::system::SystemMonitor::get_public_ipv6()
                    .await
                    .unwrap_or_else(|_| "::1".to_string());
                let v4 = crate::logic::system::SystemMonitor::get_public_ip().await;
                (v6, Some(v4))
            }
        };

        let mut rng = StdRng::from_entropy();
        let geoip = crate::logic::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = crate::logic::sni_selector::SNISelector::get_for_country(
            &country_code,
            RealityProto::XHTTP,
        );

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        for i in 0..count {
            let sni = selector.next();

            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                Self::generate_enhanced_config(&mut rng, sni, i, RealityProto::XHTTP).await?;

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
                RealityProto::XHTTP,
                path.as_deref(),
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
                RealityProto::XHTTP,
                path.as_deref(),
                host_v4.as_deref(),
            );
            links.push(link);

            let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        if standalone {
            Self::create_standalone_config(batch_configs, links, RealityProto::XHTTP).await
        } else {
            Self::update_existing_config(batch_configs, links).await
        }
    }

    async fn generate_enhanced_config(
        rng: &mut StdRng,
        sni: String,
        index: usize,
        proto: RealityProto,
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
        // 随机端口
        let port = loop {
            let p = rng.gen_range(10000..60000);
            if crate::logic::maintenance::MaintenanceManager::is_port_available(p).await {
                break p;
            }
        };

        // 生成唯一参数
        let uuid = Self::generate_wwps_uuid().await?;
        let (priv_key, pub_key) = Self::generate_wwps_x25519().await?;
        let short_id = Self::generate_random_short_id();
        let uuid_short = Self::uuid_short_prefix(&uuid);

        let suffix = match proto {
            RealityProto::Vision => "vless_reality_vision",
            RealityProto::XHTTP => "vless_xhttp_reality",
        };
        let email = format!("{}-{}", uuid_short, suffix);
        let tag = format!(
            "{}-{}-{}",
            if proto == RealityProto::Vision {
                "VLESS"
            } else {
                "XHTTP"
            },
            uuid_short,
            index
        );

        let path = if proto == RealityProto::XHTTP {
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
        proto: RealityProto,
        path: Option<&str>,
        host_v4_secondary: Option<&str>, // 仅在 SplitStack 模式下使用
    ) -> String {
        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStack => format!("[{}]", host),
            IpVersion::IPv4 => host.to_string(),
        };

        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_pbk = utf8_percent_encode(pub_key, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();

        match proto {
            RealityProto::Vision => {
                format!(
                    "vless://{}@{}:{}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=tcp&headerType=none#{}",
                    uuid, fmt_host, port, encoded_sni, encoded_pbk, short_id, encoded_email
                )
            }
            RealityProto::XHTTP => {
                // 参考 GitHub #716 标准提案
                let actual_path = path.unwrap_or("/xhttp_client_upload");
                let encoded_path = utf8_percent_encode(actual_path, NON_ALPHANUMERIC).to_string();
                let mut link = format!(
                    "vless://{}@{}:{}?encryption=none&security=reality&sni={}&fp=chrome&pbk={}&sid={}&type=xhttp&path={}&mode=auto",
                    uuid, fmt_host, port, encoded_sni, encoded_pbk, short_id, encoded_path
                );

                if ip_version == IpVersion::SplitStack {
                    if let Some(v4) = host_v4_secondary {
                        // 构建 extra.downloadSettings JSON 并进行 URL 编码
                        let extra_json = json!({
                            "downloadSettings": {
                                "address": v4,
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
                }

                format!("{}#{}", link, encoded_email)
            }
        }
    }

    async fn create_standalone_config(
        configs: Vec<Value>,
        links: Vec<String>,
        proto: RealityProto,
    ) -> Result<BatchCreationResult> {
        // 生成独立文件名
        let filename = Self::generate_secure_batch_filename(proto).await?;
        let config_path = format!("/etc/wwps/wwps-core/conf/{}", filename);

        let created_count = configs.len();

        // 创建完整配置结构
        let config = json!({
            "log": {
                "loglevel": "warning"
            },
            "dns": {
                "servers": [
                    "https+local://dns.google/dns-query"
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
        let backup_path = Self::backup_config_file(
            "/etc/wwps/wwps-core/conf/07_VLESS_vision_reality_inbounds.json",
        )
        .await?;

        // 更新现有配置
        let path = "/etc/wwps/wwps-core/conf/07_VLESS_vision_reality_inbounds.json";
        let mut v: Value = serde_json::from_str(&fs::read_to_string(path).await?)?;

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
        fs::write(path, serde_json::to_string_pretty(&v)?).await?;
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
        let config_path = "/etc/wwps/wwps-core/conf/10_warp_routing.json";

        if rules.is_empty() {
            let _ = fs::remove_file(config_path).await;
            crate::logic::maintenance::MaintenanceManager::reload_core().await?;
            return Ok(());
        }

        let outbounds = match mode {
            WarpMode::Default => json!([
                {
                    "tag": "warp",
                    "protocol": "socks",
                    "settings": {
                        "servers": [{ "address": "127.0.0.1", "port": 40000 }]
                    }
                }
            ]),
            WarpMode::IPv4 => json!([
                {
                    "tag": "warp-socks",
                    "protocol": "socks",
                    "settings": {
                        "servers": [{ "address": "127.0.0.1", "port": 40000 }]
                    }
                },
                {
                    "tag": "warp",
                    "protocol": "freedom",
                    "settings": { "domainStrategy": "UseIPv4" },
                    "streamSettings": { "sockopt": { "dialerProxy": "warp-socks" } }
                }
            ]),
            WarpMode::IPv6 => json!([
                {
                    "tag": "warp-socks",
                    "protocol": "socks",
                    "settings": {
                        "servers": [{ "address": "127.0.0.1", "port": 40000 }]
                    }
                },
                {
                    "tag": "warp",
                    "protocol": "freedom",
                    "settings": { "domainStrategy": "UseIPv6" },
                    "streamSettings": { "sockopt": { "dialerProxy": "warp-socks" } }
                }
            ]),
        };

        let config = json!({
            "outbounds": outbounds,
            "routing": {
                "rules": [
                    {
                        "type": "field",
                        "outboundTag": "warp",
                        "domain": rules
                    }
                ]
            }
        });

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(config_path, content).await?;
        crate::logic::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn get_warp_routing_rules() -> Result<(Vec<String>, WarpMode)> {
        let config_path = "/etc/wwps/wwps-core/conf/10_warp_routing.json";
        if !Path::new(config_path).exists() {
            return Ok((Vec::new(), WarpMode::Default));
        }

        let content = fs::read_to_string(config_path).await?;
        let v: Value = serde_json::from_str(&content)?;

        // Extract rules
        let rules = if let Some(rules) = v["routing"]["rules"].as_array() {
            if let Some(first_rule) = rules.first() {
                if let Some(domains) = first_rule["domain"].as_array() {
                    domains
                        .iter()
                        .filter_map(|d| d.as_str().map(String::from))
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Extract IP mode
        let mode = if let Some(outbounds) = v["outbounds"].as_array() {
            if outbounds.len() == 2 {
                if let Some(freedom) = outbounds.iter().find(|o| o["protocol"] == "freedom") {
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
            }
        } else {
            WarpMode::Default
        };

        Ok((rules, mode))
    }
}

async fn run_wwps_core_cmd(args: &[&str]) -> Result<String> {
    let (status, stdout, stderr) = run_cmd_output(
        ConfigManager::WWPS_CORE_BIN,
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
            RealityProto::Vision,
            None,
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
            RealityProto::XHTTP,
            Some(path),
        );

        assert_eq!(vless["streamSettings"]["network"], "xhttp");
        assert_eq!(vless["streamSettings"]["xhttpSettings"]["path"], path);
        assert_eq!(vless["streamSettings"]["xhttpSettings"]["mode"], "auto");
        // 验证 XHTTP 没有 flow
        assert!(vless["settings"]["clients"][0].get("flow").is_none());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarpMode {
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
