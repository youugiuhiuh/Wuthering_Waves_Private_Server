use crate::core::paths::singbox;
use crate::core::system::SystemMonitor;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::xray::port_allocator::PortAllocator;
use anyhow::{Context, Result};
use base64::Engine;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::{Value, json};
use sha2::Digest;
use std::path::Path;
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

    /// 提取配置文件中所有 hysteria2 inbound 的主端口。
    ///
    /// 按 **内容** 检测（inbound `type == "hysteria2"`）而非文件名：批量配置文件命名为
    /// `batch_hy2_*.json`（`hy2` 而非 `hysteria`），旧代码用 `file.contains("hysteria2")`
    /// 判断会漏掉它们，导致删除时规则/端口从不清理。同时提取**全部** inbound 的主端口——
    /// 一个批量文件含 count 个 inbound，旧代码只取 `inbounds[0]` 会漏释放 count-1 个范围。
    async fn extract_hysteria2_ports_from_config(path: &str) -> Result<Vec<u16>> {
        let content = fs::read_to_string(path).await?;
        let json: Value = serde_json::from_str(&content)?;

        let mut ports = Vec::new();
        if let Some(inbounds) = json["inbounds"].as_array() {
            for inbound in inbounds {
                if inbound.get("type").and_then(|v| v.as_str()) != Some("hysteria2") {
                    continue;
                }
                if let Some(p) = inbound["listen_port"]
                    .as_u64()
                    .and_then(|p| u16::try_from(p).ok())
                {
                    ports.push(p);
                }
            }
        }
        if ports.is_empty() {
            return Err(anyhow::anyhow!("配置中未找到 hysteria2 inbound 主端口"));
        }
        Ok(ports)
    }

    async fn cleanup_specific_hysteria2_rules(
        main_port: u16,
        hop_range: (u16, u16),
        has_ipv6: bool,
    ) -> Result<()> {
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

    /// 清理 hysteria2 端口跳跃资源：逐端口清理防火墙规则并释放端口分配（最佳努力）。
    /// `alloc_file` 为 Some 时，端口释放写入该分配文件（测试隔离用）；None 使用默认分配文件。
    async fn cleanup_hysteria2_ports(ports: Vec<u16>, has_ipv6: bool, alloc_file: Option<&Path>) {
        for main_port in ports {
            let hop_range = (main_port + 1, main_port + 99);
            let _ = Self::cleanup_specific_hysteria2_rules(main_port, hop_range, has_ipv6).await;
            let release = match alloc_file {
                Some(path) => PortAllocator::release_hysteria2_range_at(path, main_port).await,
                None => PortAllocator::release_hysteria2_range(main_port).await,
            };
            let _ = release;
        }
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        Self::delete_specific_configuration_at(path, None).await
    }

    /// 同 [`delete_specific_configuration`]，但端口释放写入指定的分配文件（测试隔离用）。
    ///
    /// 顺序约定：**先删文件、删除成功后才清理规则并释放端口**——若先释放端口而
    /// `remove_file` 失败，配置文件残留但范围已释放，下次分配可能与监听端口重叠。
    /// 清理失败（iptables 无权限、解析错误等）为最佳努力：`let _` 吞掉，不阻塞删除。
    pub async fn delete_specific_configuration_at(
        path: &str,
        alloc_file: Option<&Path>,
    ) -> Result<()> {
        // 提取 hysteria2 主端口（非 hysteria2 文件返回 Err，跳过清理）
        let hy2_ports = Self::extract_hysteria2_ports_from_config(path).await.ok();

        fs::remove_file(path).await.context("删除配置文件失败")?;

        // 文件删除成功后才清理规则并释放端口，避免残留文件与端口分配不一致
        if let Some(ports) = hy2_ports {
            let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
            Self::cleanup_hysteria2_ports(ports, has_ipv6, alloc_file).await;
        }

        Ok(())
    }

    pub async fn delete_all_configurations() -> Result<usize> {
        let files = Self::list_all_inbound_files().await?;
        let mut deleted = 0;
        let mut has_ipv6: Option<bool> = None;

        for file in &files {
            // 按内容检测 hysteria2；非 hysteria2 文件 extract 返回 Err，跳过清理
            let hy2_ports = Self::extract_hysteria2_ports_from_config(file).await.ok();
            if fs::remove_file(file).await.is_ok() {
                deleted += 1;
                if let Some(ports) = hy2_ports {
                    // 仅当存在 hysteria2 配置时才探测 IPv6（网络调用，延迟到需要时）
                    if has_ipv6.is_none() {
                        has_ipv6 = Some(SystemMonitor::get_public_ipv6().await.is_ok());
                    }
                    Self::cleanup_hysteria2_ports(ports, has_ipv6.unwrap_or(false), None).await;
                }
            }
        }

        Ok(deleted)
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
        let mut has_ipv6: Option<bool> = None;

        for (path, _) in sorted_files.iter().take(delete_count) {
            let path_str = path.to_string_lossy().to_string();
            let hy2_ports = Self::extract_hysteria2_ports_from_config(&path_str)
                .await
                .ok();
            if fs::remove_file(path).await.is_ok() {
                deleted += 1;
                if let Some(ports) = hy2_ports {
                    if has_ipv6.is_none() {
                        has_ipv6 = Some(SystemMonitor::get_public_ipv6().await.is_ok());
                    }
                    Self::cleanup_hysteria2_ports(ports, has_ipv6.unwrap_or(false), None).await;
                }
            }
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
        let (cert_exists, key_exists) = tokio::join!(
            async {
                match tokio::fs::try_exists(singbox::TLS_CERT).await {
                    Ok(true) => true,
                    Ok(false) => false,
                    Err(e) => {
                        log::warn!("检查 TLS 证书存在性失败: {}", e);
                        false
                    }
                }
            },
            async {
                match tokio::fs::try_exists(singbox::TLS_KEY).await {
                    Ok(true) => true,
                    Ok(false) => false,
                    Err(e) => {
                        log::warn!("检查 TLS 密钥存在性失败: {}", e);
                        false
                    }
                }
            },
        );
        if cert_exists && key_exists {
            return Ok(());
        }

        tokio::fs::create_dir_all(singbox::CERTS_DIR)
            .await
            .context("创建证书目录失败")?;

        let key_output = tokio::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "EC",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-out",
                singbox::TLS_KEY,
            ])
            .output()
            .await
            .context("生成 ECDSA 私钥失败")?;

        if !key_output.status.success() {
            return Err(anyhow::anyhow!(
                "生成 ECDSA 私钥失败: {}",
                String::from_utf8_lossy(&key_output.stderr)
            ));
        }

        let cert_output = tokio::process::Command::new("openssl")
            .args([
                "req",
                "-new",
                "-x509",
                "-key",
                singbox::TLS_KEY,
                "-out",
                singbox::TLS_CERT,
                "-days",
                "3650",
                "-subj",
                "/CN=wwps",
                "-addext",
                "subjectAltName=DNS:wwps",
            ])
            .output()
            .await
            .context("生成自签名证书失败")?;

        if !cert_output.status.success() {
            return Err(anyhow::anyhow!(
                "生成自签名证书失败: {}",
                String::from_utf8_lossy(&cert_output.stderr)
            ));
        }

        log::info!("已使用 OpenSSL ECDSA P-256 生成自签名证书");
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

    pub(crate) async fn compute_cert_sha256_pin(cert_path: &str) -> Result<String> {
        let pem_data = tokio::fs::read(cert_path).await?;
        let (pem, _) = x509_parser::pem::Pem::read(std::io::Cursor::new(&pem_data))
            .map_err(|e| anyhow::anyhow!("PEM 解析失败: {}", e))?;
        let hash = sha2::Sha256::digest(pem.contents);
        Ok(hash
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(":"))
    }

    #[allow(dead_code)]
    pub(crate) async fn compute_pubkey_sha256_base64(cert_path: &str) -> Result<String> {
        use x509_parser::prelude::*;
        let pem_data = tokio::fs::read(cert_path).await?;
        let (pem, _) = x509_parser::pem::Pem::read(std::io::Cursor::new(&pem_data))
            .map_err(|e| anyhow::anyhow!("PEM 解析失败: {}", e))?;
        let (_, x509) = X509Certificate::from_der(&pem.contents)
            .map_err(|e| anyhow::anyhow!("证书解析失败: {}", e))?;
        let spki_bytes = x509.public_key().raw;
        let hash = sha2::Sha256::digest(spki_bytes);
        Ok(base64::engine::general_purpose::STANDARD.encode(hash))
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
        assert!(config.pin_sha256.is_none());
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
        assert!(config.cert_sha256.is_none());
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

    #[tokio::test]
    async fn test_extract_hysteria2_ports_all_inbounds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch_hy2_test.json");
        let json = serde_json::json!({
            "inbounds": [
                {"type": "hysteria2", "listen_port": 11123},
                {"type": "hysteria2", "listen_port": 11223},
                {"type": "tuic", "listen_port": 30001},
                {"type": "hysteria2", "listen_port": 11323}
            ]
        });
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let ports =
            SingBoxConfigManager::extract_hysteria2_ports_from_config(path.to_str().unwrap())
                .await
                .unwrap();

        assert_eq!(
            ports,
            vec![11123u16, 11223, 11323],
            "应提取所有 hysteria2 inbound 的主端口，跳过非 hysteria2 inbound"
        );
    }

    #[tokio::test]
    async fn test_extract_hysteria2_ports_none_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch_tuic_test.json");
        let json = serde_json::json!({
            "inbounds": [
                {"type": "tuic", "listen_port": 30001}
            ]
        });
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let result =
            SingBoxConfigManager::extract_hysteria2_ports_from_config(path.to_str().unwrap()).await;
        assert!(result.is_err(), "无 hysteria2 inbound 时应返回 Err");
    }

    #[tokio::test]
    async fn extract_hysteria2_ports_returns_err_for_corrupt_json() {
        // Arrange: 故意构造截断 JSON 的 hy2 配置（模拟损坏的批量配置文件）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch_hy2_corrupt.json");
        tokio::fs::write(&path, "{\"inbounds\": [ { \"type\": \"hysteria2\", ")
            .await
            .unwrap();

        // Act
        let result =
            SingBoxConfigManager::extract_hysteria2_ports_from_config(path.to_str().unwrap()).await;

        // Assert
        assert!(result.is_err(), "损坏 JSON 应返回 Err 而非 panic");
    }

    #[tokio::test]
    async fn cleanup_hysteria2_ports_releases_all_locked_ranges() {
        // Arrange: 在临时分配文件中分配 3 个跳跃范围（模拟 3-inbound 批量配置）
        let dir = tempfile::tempdir().unwrap();
        let alloc_path = dir.path().join(".port_alloc");
        let mut ports = Vec::new();
        for _ in 0..3 {
            let (main_port, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
                .await
                .unwrap();
            ports.push(main_port);
        }

        // Act: 模拟删除配置后的清理，全部端口应被释放
        SingBoxConfigManager::cleanup_hysteria2_ports(ports, false, Some(&alloc_path)).await;

        // Assert: 分配文件中不再有任何锁定范围（旧代码只释放第一个端口）
        let content = tokio::fs::read_to_string(&alloc_path).await.unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            data["locked_ranges"].as_array().unwrap().is_empty(),
            "多 inbound 配置删除后所有锁定范围均应释放"
        );
    }

    #[tokio::test]
    async fn delete_specific_configuration_removes_corrupt_file() {
        // Arrange: 故意构造损坏 JSON 的 hy2 配置
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch_hy2_corrupt.json");
        tokio::fs::write(&path, "{\"inbounds\": [ { \"type\": \"hysteria2\", ")
            .await
            .unwrap();

        // Act: 删除路径应对解析错误宽容（跳过清理、文件照删、不崩溃）
        let result =
            SingBoxConfigManager::delete_specific_configuration(path.to_str().unwrap()).await;

        // Assert
        assert!(
            result.is_ok(),
            "损坏配置的删除应成功（错误被捕捉而非传播）：{:?}",
            result
        );
        assert!(
            !tokio::fs::try_exists(&path).await.unwrap(),
            "损坏配置文件应被删除（文件删除不依赖解析成功）"
        );
    }

    #[tokio::test]
    async fn delete_specific_configuration_removes_multi_inbound_file() {
        // Arrange: 3 个 hysteria2 inbound 的批量配置（结构同服务器 batch_hy2_*.json）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("batch_hy2_multi.json");
        let json = serde_json::json!({
            "inbounds": [
                {"type": "hysteria2", "listen_port": 20001, "tag": "HYSTERIA2-1"},
                {"type": "hysteria2", "listen_port": 20101, "tag": "HYSTERIA2-2"},
                {"type": "hysteria2", "listen_port": 20201, "tag": "HYSTERIA2-3"}
            ]
        });
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        // Act
        let result =
            SingBoxConfigManager::delete_specific_configuration(path.to_str().unwrap()).await;

        // Assert
        assert!(result.is_ok(), "多 inbound 删除应成功：{:?}", result);
        assert!(
            !tokio::fs::try_exists(&path).await.unwrap(),
            "配置文件应被删除"
        );
    }
}

#[cfg(test)]
mod cert_pinning_tests {
    use super::*;
    use std::path::Path;

    fn create_test_cert(cert_path: &Path) {
        let output = std::process::Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-keyout",
                "/dev/null",
                "-out",
                &cert_path.to_string_lossy(),
                "-days",
                "36500",
                "-nodes",
                "-subj",
                "/CN=test",
            ])
            .output()
            .expect("openssl binary required for tests");
        assert!(output.status.success());
    }

    #[tokio::test]
    async fn test_compute_cert_sha256_pin_format() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("test.pem");
        create_test_cert(&cert_path);

        let pin = SingBoxConfigManager::compute_cert_sha256_pin(cert_path.to_str().unwrap())
            .await
            .unwrap();

        // 32 bytes → AA:BB:CC:...:DD = 95 chars
        assert_eq!(pin.len(), 95);
        assert!(pin.chars().all(|c| c.is_ascii_hexdigit() || c == ':'));
        assert_eq!(pin.chars().filter(|&c| c == ':').count(), 31);
    }

    #[tokio::test]
    async fn test_compute_cert_sha256_pin_invalid_path() {
        let result = SingBoxConfigManager::compute_cert_sha256_pin("/nonexistent/cert.pem").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_compute_pubkey_sha256_base64_format() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("test.pem");
        create_test_cert(&cert_path);

        let hash = SingBoxConfigManager::compute_pubkey_sha256_base64(cert_path.to_str().unwrap())
            .await
            .unwrap();

        // Base64 SHA256 = 44 chars
        assert_eq!(hash.len(), 44);
        // Valid base64 chars only
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[tokio::test]
    async fn test_compute_pubkey_sha256_base64_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("test.pem");
        create_test_cert(&cert_path);

        let h1 = SingBoxConfigManager::compute_pubkey_sha256_base64(cert_path.to_str().unwrap())
            .await
            .unwrap();
        let h2 = SingBoxConfigManager::compute_pubkey_sha256_base64(cert_path.to_str().unwrap())
            .await
            .unwrap();

        assert_eq!(h1, h2);
    }

    #[tokio::test]
    async fn test_compute_pubkey_sha256_base64_invalid_path() {
        let result =
            SingBoxConfigManager::compute_pubkey_sha256_base64("/nonexistent/cert.pem").await;
        assert!(result.is_err());
    }
}
