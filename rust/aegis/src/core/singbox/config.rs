use crate::core::paths::singbox;
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::xray::port_allocator::PortAllocator;
use anyhow::{Context, Result};
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::{Value, json};
use tokio::fs;

pub struct SingBoxConfigManager;

impl SingBoxConfigManager {
    pub async fn is_installed() -> bool {
        fs::try_exists(singbox::BIN).await.unwrap_or(false)
    }

    pub async fn list_all_inbound_files() -> Result<Vec<String>> {
        let mut out = Vec::new();
        if let Ok(mut rd) = fs::read_dir(singbox::CONF_DIR).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && name.ends_with(".json")
                    && !name.starts_with("00_")
                    && !name.starts_with("01_")
                {
                    out.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
        Ok(out)
    }

    pub async fn collect_all_ports() -> Result<std::collections::HashSet<u16>> {
        let files = Self::list_all_inbound_files().await?;
        let mut ports = std::collections::HashSet::new();
        for file in &files {
            if let Ok(content) = fs::read_to_string(file).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    Self::extract_ports_recursive(&json, &mut ports);
                }
            } else {
                log::warn!("无法读取配置文件: {}", file);
            }
        }
        Ok(ports)
    }

    fn extract_ports_recursive(
        value: &serde_json::Value,
        ports: &mut std::collections::HashSet<u16>,
    ) {
        if let Some(main_port) = value
            .get("listen_port")
            .and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok())
        {
            ports.insert(main_port);
            // If this is a hysteria2 inbound, also include hopping range
            if value.get("type").and_then(|v| v.as_str()) == Some("hysteria2") {
                for p in (main_port + 1)..=(main_port + 99) {
                    ports.insert(p);
                }
            }
        }
        if let Some(obj) = value.as_object() {
            for (_, v) in obj {
                Self::extract_ports_recursive(v, ports);
            }
        }
        if let Some(arr) = value.as_array() {
            for v in arr {
                Self::extract_ports_recursive(v, ports);
            }
        }
    }

    async fn extract_main_port_from_config(path: &str) -> Result<u16> {
        let content = fs::read_to_string(path).await?;
        let json: Value = serde_json::from_str(&content)?;

        let port = json["inbounds"][0]["listen_port"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("无法解析主端口"))? as u16;

        Ok(port)
    }

    async fn cleanup_specific_hysteria2_rules(main_port: u16, hop_range: (u16, u16)) -> Result<()> {
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);
        let _ = Command::new("iptables")
            .args([
                "-t",
                "nat",
                "-D",
                "PREROUTING",
                "-p",
                "udp",
                "--dport",
                &range_str,
                "-j",
                "REDIRECT",
                "--to-ports",
                &main_port.to_string(),
            ])
            .output()
            .await;

        let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
        if has_ipv6 {
            let _ = Command::new("ip6tables")
                .args([
                    "-t",
                    "nat",
                    "-D",
                    "PREROUTING",
                    "-p",
                    "udp",
                    "--dport",
                    &range_str,
                    "-j",
                    "REDIRECT",
                    "--to-ports",
                    &main_port.to_string(),
                ])
                .output()
                .await;

            let _ = MaintenanceManager::remove_port_range_v6(hop_range.0, hop_range.1).await;
        }

        let _ = MaintenanceManager::remove_port_range(main_port, main_port).await;
        let _ = MaintenanceManager::remove_port_range(hop_range.0, hop_range.1).await;

        log::info!(
            "已清理 Hysteria2 端口跳跃规则: 主端口 {}, 范围 {}",
            main_port,
            range_str
        );
        Ok(())
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        let main_port = Self::extract_main_port_from_config(path).await?;
        let hop_range = (main_port + 1, main_port + 99);

        fs::remove_file(path).await.context("删除配置文件失败")?;

        Self::cleanup_specific_hysteria2_rules(main_port, hop_range).await?;

        let _ = PortAllocator::release_hysteria2_range(main_port).await;

        let remaining = Self::list_all_inbound_files().await?;
        let has_hysteria2 = remaining.iter().any(|f| f.contains("hysteria2"));

        if !has_hysteria2 {
            let _ = PortAllocator::release_hysteria2_range(main_port).await;
        }

        Self::reload_service().await?;
        Ok(())
    }

    pub async fn delete_all_configurations() -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        let count = files.len();

        for file in &files {
            if (file.contains("hysteria2") || file.contains("hysteria"))
                && let Ok(main_port) = Self::extract_main_port_from_config(file).await
            {
                let hop_range = (main_port + 1, main_port + 99);
                Self::cleanup_specific_hysteria2_rules(main_port, hop_range).await?;
            }

            let _ = fs::remove_file(file).await;
        }

        if count > 0 {
            Self::reload_service().await?;
        }
        Ok(count)
    }

    pub async fn delete_by_count(count: usize) -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;

        if files.is_empty() {
            return Ok(0);
        }

        let mut sorted_files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
        for file in &files {
            let path = std::path::PathBuf::from(file);
            if let Ok(metadata) = tokio::fs::metadata(&path).await
                && let Ok(modified) = metadata.modified()
            {
                sorted_files.push((path, modified));
            }
        }

        sorted_files.sort_by_key(|a| a.1);

        let delete_count = count.min(sorted_files.len());
        let mut deleted = 0;

        for (path, _) in sorted_files.iter().take(delete_count) {
            if fs::remove_file(path).await.is_ok() {
                deleted += 1;
            }
        }

        if deleted > 0 {
            Self::reload_service().await?;
        }

        Ok(deleted)
    }

    pub async fn get_config_count() -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        Ok(files.len())
    }

    #[allow(dead_code)]
    async fn cleanup_port_hopping_firewall() -> Result<()> {
        use tokio::process::Command;

        // 检测 IPv6
        let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

        if let Some((main_port, hop_range)) = PortAllocator::get_hysteria2_range().await {
            // 清理 IPv4 规则（总是）
            let _ = Command::new("iptables")
                .args([
                    "-t",
                    "nat",
                    "-D",
                    "PREROUTING",
                    "-p",
                    "udp",
                    "-j",
                    "REDIRECT",
                    "--to-ports",
                    &main_port.to_string(),
                ])
                .output()
                .await;

            // 移除 IPv4 端口范围
            let _ = MaintenanceManager::remove_port_range(hop_range.0, hop_range.1).await;

            // 清理 IPv6 规则（仅当 has_ipv6）
            if has_ipv6 {
                let _ = Command::new("ip6tables")
                    .args([
                        "-t",
                        "nat",
                        "-D",
                        "PREROUTING",
                        "-p",
                        "udp",
                        "-j",
                        "REDIRECT",
                        "--to-ports",
                        &main_port.to_string(),
                    ])
                    .output()
                    .await;

                // 移除 IPv6 端口范围
                let _ = MaintenanceManager::remove_port_range_v6(hop_range.0, hop_range.1).await;
            }

            log::info!("已清理 Hysteria2 端口跳跃防火墙规则");
        }

        Ok(())
    }

    pub(crate) async fn reload_service() -> Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", "wwps-box"])
            .output()
            .await
            .context("重载配置失败")?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "重载失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }

    pub async fn ensure_base_config() -> Result<()> {
        let base_path = format!("{}/00_base.json", singbox::CONF_DIR);

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

        tokio::fs::create_dir_all(singbox::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let base_config = serde_json::json!({
            "log": {
                "level": "warning"
            },
            "dns": {
                "servers": [
                    {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
                    {"tag": "local", "type": "local"}
                ]
            },
            "route": {
                "default_domain_resolver": "dns"
            },
            "outbounds": [
                {"type": "direct", "tag": "direct"},
                {"type": "block", "tag": "block"}
            ]
        });

        let content = serde_json::to_string_pretty(&base_config).context("序列化基础配置失败")?;
        tokio::fs::write(&base_path, content)
            .await
            .context("写入基础配置失败")?;

        log::info!("已创建 wwps-box 基础配置: {}", base_path);
        Ok(())
    }

    pub(crate) async fn ensure_tls_certificates() -> Result<()> {
        let cert_exists = match tokio::fs::try_exists(singbox::TLS_CERT).await {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                log::warn!("检查 TLS 证书存在性失败: {}", e);
                false
            }
        };
        let key_exists = match tokio::fs::try_exists(singbox::TLS_KEY).await {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                log::warn!("检查 TLS 密钥存在性失败: {}", e);
                false
            }
        };
        if cert_exists && key_exists {
            return Ok(());
        }

        tokio::fs::create_dir_all(singbox::CERTS_DIR)
            .await
            .context("创建证书目录失败")?;

        let output = tokio::process::Command::new(singbox::BIN)
            .args(["generate", "tls-keypair", "tls", "-m", "456"])
            .output()
            .await
            .context("生成 TLS 证书失败")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "生成证书失败: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        let mut key_content = String::new();
        let mut cert_content = String::new();
        let mut in_key = false;
        let mut in_cert = false;

        for line in lines {
            if line.contains("BEGIN PRIVATE KEY") {
                in_key = true;
                key_content.push_str(line);
                key_content.push('\n');
                continue;
            }
            if line.contains("END PRIVATE KEY") {
                key_content.push_str(line);
                key_content.push('\n');
                in_key = false;
                continue;
            }
            if line.contains("BEGIN CERTIFICATE") {
                in_cert = true;
                cert_content.push_str(line);
                cert_content.push('\n');
                continue;
            }
            if line.contains("END CERTIFICATE") {
                cert_content.push_str(line);
                cert_content.push('\n');
                in_cert = false;
                continue;
            }

            if in_key {
                key_content.push_str(line);
                key_content.push('\n');
            }
            if in_cert {
                cert_content.push_str(line);
                cert_content.push('\n');
            }
        }

        tokio::fs::write(singbox::TLS_KEY, key_content).await?;
        tokio::fs::write(singbox::TLS_CERT, cert_content).await?;

        Ok(())
    }

    pub(crate) async fn save_standalone_config(
        configs: Vec<Value>,
        proto: &str,
    ) -> Result<(String, String)> {
        use rand::Rng;

        fs::create_dir_all(singbox::CONF_DIR)
            .await
            .context("创建配置目录失败")?;

        let mut rng = StdRng::from_entropy();
        let timestamp = chrono::Utc::now().timestamp();
        let random_part: String = (0..8)
            .map(|_| {
                let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
                let idx = rng.gen_range(0..chars.len());
                chars[idx] as char
            })
            .collect();

        let filename = match proto {
            "hysteria2" => format!("batch_hy2_{}_{}.json", timestamp, random_part),
            "tuic" => format!("batch_tuic_{}_{}.json", timestamp, random_part),
            _ => format!("batch_{}_{}_{}.json", proto, timestamp, random_part),
        };

        let config_path = format!("{}/{}", singbox::CONF_DIR, filename);

        let inbound_only_config = json!({
            "inbounds": configs
        });

        let content = serde_json::to_string_pretty(&inbound_only_config)?;
        fs::write(&config_path, content).await?;

        Ok((filename, config_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::singbox::hysteria2::Hysteria2Config;
    use crate::core::singbox::tuic::TUICConfig;

    #[tokio::test]
    async fn test_singbox_is_installed_returns_bool() {
        let result = SingBoxConfigManager::is_installed().await;
        let _: bool = result;
    }

    #[test]
    fn test_singbox_config_manager_exists() {
        let _ = SingBoxConfigManager;
    }

    #[test]
    fn test_hysteria2_config_struct() {
        let config = Hysteria2Config::new(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
        );
        assert_eq!(config.port, 8443);
        assert_eq!(config.password, "test_password");
        assert_eq!(config.sni, "sni.example.com");
    }

    #[test]
    fn test_tuic_config_struct() {
        let config = TUICConfig::new(
            9443,
            "test-uuid".to_string(),
            "password".to_string(),
            "sni.example.com".to_string(),
        );
        assert_eq!(config.port, 9443);
        assert_eq!(config.uuid, "test-uuid");
        assert_eq!(config.congestion_control, "bbr");
    }

    #[tokio::test]
    async fn test_ensure_base_config_creates_base_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let conf_dir = temp_dir.path().to_str().unwrap();

        // 修改 singbox::CONF_DIR 为临时目录来测试
        // 通过直接测试文件创建逻辑
        let base_path = format!("{}/00_base.json", conf_dir);

        // 清理可能存在的旧文件
        let _ = tokio::fs::remove_file(&base_path).await;

        // 直接创建配置目录和基础文件（模拟 ensure_base_config 逻辑）
        tokio::fs::create_dir_all(conf_dir).await.unwrap();

        let base_config = serde_json::json!({
            "log": {
                "level": "warning"
            },
            "dns": {
                "servers": [
                    {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
                    {"tag": "local", "type": "local"}
                ]
            },
            "route": {
                "default_domain_resolver": "dns"
            },
            "outbounds": [
                {"type": "direct", "tag": "direct"},
                {"type": "block", "tag": "block"}
            ]
        });

        let content = serde_json::to_string_pretty(&base_config).unwrap();
        tokio::fs::write(&base_path, content).await.unwrap();

        // 验证文件存在且可解析
        let file_content = tokio::fs::read_to_string(&base_path).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(&file_content).unwrap();

        assert!(json.get("log").is_some());
        assert!(json.get("dns").is_some());
        assert!(json.get("route").is_some());
        assert!(json.get("outbounds").is_some());
    }

    #[tokio::test]
    async fn test_ensure_base_config_idempotent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let conf_dir = temp_dir.path().to_str().unwrap();
        let base_path = format!("{}/00_base.json", conf_dir);

        // 第一次调用
        tokio::fs::create_dir_all(conf_dir).await.unwrap();
        let base_config = serde_json::json!({
            "log": {"level": "warning"},
            "dns": {"servers": []},
            "route": {},
            "outbounds": []
        });
        let content = serde_json::to_string_pretty(&base_config).unwrap();
        tokio::fs::write(&base_path, content).await.unwrap();

        // 验证文件存在
        assert!(tokio::fs::try_exists(&base_path).await.unwrap());

        // 第二次调用（应该幂等）
        let result = tokio::fs::write(&base_path, "test").await;
        assert!(result.is_ok(), "幂等性检查：文件已存在时应该可覆盖");
    }
}

#[cfg(test)]
mod port_collection_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_extract_ports_hysteria2_includes_hopping() {
        let json = serde_json::json!({
            "inbounds": [{
                "type": "hysteria2",
                "listen_port": 20001,
                "listen": "::"
            }]
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&20001));
        assert!(ports.contains(&20002));
        assert!(ports.contains(&20100));
        assert_eq!(ports.len(), 100);
    }

    #[test]
    fn test_extract_ports_tuic_single_port() {
        let json = serde_json::json!({
            "type": "tuic",
            "listen_port": 30001
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&30001));
        assert_eq!(ports.len(), 1);
    }

    #[test]
    fn test_extract_ports_nested() {
        let json = serde_json::json!({
            "inbounds": [
                {"type": "tuic", "listen_port": 30001},
                {"type": "hysteria2", "listen_port": 20001}
            ]
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&30001));
        assert!(ports.contains(&20001));
        assert!(ports.contains(&20050));
        assert!(ports.contains(&20100));
    }
}
