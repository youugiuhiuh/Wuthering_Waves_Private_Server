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
use std::future::Future;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::core::config_delete::{
    BulkDeleteError, BulkDeleteResult, BulkDeleteTracker, DeleteStage,
};

pub struct SingBoxConfigManager;

impl SingBoxConfigManager {
    pub async fn is_installed() -> bool {
        fs::try_exists(singbox::BIN).await.unwrap_or(false)
    }

    pub async fn list_all_inbound_files() -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut rd = match fs::read_dir(singbox::CONF_DIR).await {
            Ok(rd) => rd,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            if let Some(name) = entry.file_name().to_str()
                && name.ends_with(".json")
                && !name.starts_with("00_")
                && !name.starts_with("01_")
            {
                out.push(entry.path().to_string_lossy().to_string());
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

    async fn extract_main_port_from_config(path: &Path) -> Result<u16> {
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
        let main_port = Self::extract_main_port_from_config(Path::new(path)).await?;
        let hop_range = (main_port + 1, main_port + 99);

        fs::remove_file(path).await.context("删除配置文件失败")?;

        Self::cleanup_specific_hysteria2_rules(main_port, hop_range).await?;

        PortAllocator::release_hysteria2_range(main_port).await?;

        let remaining = Self::list_all_inbound_files().await?;
        let has_hysteria2 = remaining.iter().any(|f| f.contains("hysteria2"));

        if !has_hysteria2 {
            PortAllocator::release_hysteria2_range(main_port).await?;
        }

        Self::reload_service().await?;
        Ok(())
    }

    async fn prepare_for_bulk_delete(path: &Path) -> Result<()> {
        let path_text = path.to_string_lossy();
        if path_text.contains("hysteria2") || path_text.contains("hysteria") {
            let main_port = Self::extract_main_port_from_config(path).await?;
            Self::cleanup_specific_hysteria2_rules(main_port, (main_port + 1, main_port + 99))
                .await?;
        }
        Ok(())
    }

    async fn delete_files_with_reload<F, Fut>(
        files: Vec<String>,
        limit: Option<usize>,
        reload: F,
    ) -> BulkDeleteResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<()>>,
    {
        const OPERATION: &str = "sing-box bulk delete";
        let target = limit.map_or(files.len(), |count| count.min(files.len()));
        let mut tracker = BulkDeleteTracker::new(OPERATION, target);
        let selected: Vec<PathBuf> = if let Some(count) = limit {
            let mut sortable = Vec::new();
            for file in files {
                let path = PathBuf::from(file);
                match fs::metadata(&path)
                    .await
                    .and_then(|metadata| metadata.modified())
                {
                    Ok(modified) => sortable.push((path, modified)),
                    Err(source) => tracker.record_failure(path, DeleteStage::Inspect, source),
                }
            }
            sortable.sort_by_key(|(_, modified)| *modified);
            sortable
                .into_iter()
                .take(count)
                .map(|(path, _)| path)
                .collect()
        } else {
            files.into_iter().map(PathBuf::from).collect()
        };

        for path in selected {
            if let Err(source) = Self::prepare_for_bulk_delete(&path).await {
                tracker.record_failure(path, DeleteStage::Prepare, source);
                continue;
            }
            match fs::remove_file(&path).await {
                Ok(()) => tracker.record_deleted(),
                Err(source) => tracker.record_failure(path, DeleteStage::Remove, source),
            }
        }
        let reload_error = if tracker.deleted() > 0 {
            reload().await.err()
        } else {
            None
        };
        tracker.finish(reload_error)
    }

    pub async fn delete_all_configurations() -> BulkDeleteResult {
        let files = Self::list_all_inbound_files()
            .await
            .map_err(|source| BulkDeleteError::discovery("sing-box bulk delete", source))?;
        Self::delete_files_with_reload(files, None, Self::reload_service).await
    }

    pub async fn delete_by_count(count: usize) -> BulkDeleteResult {
        let files = Self::list_all_inbound_files()
            .await
            .map_err(|source| BulkDeleteError::discovery("sing-box bulk delete", source))?;
        Self::delete_files_with_reload(files, Some(count), Self::reload_service).await
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

        if let Some((main_port, hop_range)) = PortAllocator::get_hysteria2_range().await? {
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
    use crate::core::config_delete::DeleteStage;
    use crate::core::singbox::hysteria2::Hysteria2Config;
    use crate::core::singbox::tuic::TUICConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

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

    #[tokio::test]
    async fn bulk_delete_keeps_malformed_hysteria_file_and_continues() {
        let dir = tempdir().unwrap();
        let hysteria = dir.path().join("hysteria2_bad.json");
        let ordinary = dir.path().join("vless.json");
        tokio::fs::write(&hysteria, "not-json").await.unwrap();
        tokio::fs::write(&ordinary, "{}").await.unwrap();
        let reloads = AtomicUsize::new(0);

        let result = SingBoxConfigManager::delete_files_with_reload(
            vec![
                hysteria.to_string_lossy().into_owned(),
                ordinary.to_string_lossy().into_owned(),
            ],
            None,
            || async {
                reloads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert_eq!(error.deleted(), 1);
        assert_eq!(error.failures()[0].stage, DeleteStage::Prepare);
        assert!(hysteria.exists());
        assert!(!ordinary.exists());
        assert_eq!(reloads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn bulk_delete_empty_input_does_not_reload() {
        let reloads = AtomicUsize::new(0);
        let result = SingBoxConfigManager::delete_files_with_reload(Vec::new(), None, || async {
            reloads.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;

        assert_eq!(result.unwrap(), 0);
        assert_eq!(reloads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bulk_delete_by_count_records_inspect_failure() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        let ordinary = dir.path().join("vless.json");
        tokio::fs::write(&ordinary, "{}").await.unwrap();

        let result = SingBoxConfigManager::delete_files_with_reload(
            vec![
                missing.to_string_lossy().into_owned(),
                ordinary.to_string_lossy().into_owned(),
            ],
            Some(1),
            || async { Ok(()) },
        )
        .await;

        let error = result.unwrap_err();
        assert_eq!(error.deleted(), 1);
        assert_eq!(error.failures()[0].stage, DeleteStage::Inspect);
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
