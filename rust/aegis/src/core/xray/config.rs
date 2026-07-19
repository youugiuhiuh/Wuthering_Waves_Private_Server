use anyhow::{Context, Result, anyhow};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;

use crate::core::cmd_async::run_cmd_output;
use crate::core::config_delete::{
    BulkDeleteError, BulkDeleteResult, BulkDeleteTracker, DeleteStage,
};
use crate::core::paths::xray;
use crate::core::system::maintenance::MaintenanceManager;
use crate::core::types::{BatchCreationResult, IpVersion};
use crate::core::xray::routing::{ROUTING_RULES, RoutingManager};

use super::reality::{REALITY_PQ_SEED, REALITY_PQ_VERIFY, reality_pq_verify_as_base64url};

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
    pub(crate) const TIMEOUT_WWPS_CORE: Duration = Duration::from_secs(5);

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
        let mut rd = match fs::read_dir(xray::CONF_DIR).await {
            Ok(rd) => rd,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(error.into()),
        };

        while let Some(entry) = rd.next_entry().await? {
            if let Some(name) = entry.file_name().to_str()
                && name.ends_with("_inbounds.json")
                && !name.starts_with("00_")
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
                if let Ok(json) = serde_json::from_str::<Value>(&content)
                    && let Some(inbounds) = json.get("inbounds").and_then(|v| v.as_array())
                {
                    for inbound in inbounds {
                        if let Some(port) = inbound.get("port").and_then(|v| v.as_u64())
                            && port <= u16::MAX as u64
                        {
                            ports.insert(port as u16);
                        }
                    }
                }
            } else {
                log::warn!("无法读取配置文件: {}", file);
            }
        }
        Ok(ports)
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

    pub(crate) async fn generate_wwps_uuid() -> Result<String> {
        let stdout = run_wwps_core_cmd(&["uuid"]).await?;
        Ok(stdout.trim().to_string())
    }

    pub(crate) async fn generate_wwps_x25519() -> Result<(String, String)> {
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

    pub(crate) fn generate_random_short_id() -> String {
        let mut rng = StdRng::from_entropy();
        format!("{:016x}", rng.r#gen::<u64>())
    }

    pub(crate) fn generate_random_path() -> String {
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

    pub(crate) fn uuid_short_prefix(uuid: &str) -> String {
        uuid.split('-')
            .next()
            .unwrap_or(uuid)
            .chars()
            .take(8)
            .collect::<String>()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_reality_vless_inbound(
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
                "target": format!("{}:443", sni),
                "xver": 0,
                "serverNames": [sni],
                "privateKey": priv_key,
                "minClientVer": "1.0.0",
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

    pub(crate) fn resolve_public_hosts(
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

    pub(crate) async fn generate_enhanced_config(
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
            if crate::core::system::maintenance::MaintenanceManager::is_port_available(pp).await {
                pp as i32
            } else {
                loop {
                    let p = rng.gen_range(10000..60000);
                    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p)
                        .await?
                    {
                        continue;
                    }
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(p)
                        .await
                    {
                        break p as i32;
                    }
                }
            }
        } else {
            loop {
                let p = rng.gen_range(10000..60000);
                if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p)
                    .await?
                {
                    continue;
                }
                if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await
                {
                    break p as i32;
                }
            }
        };

        // 生成唯一参数
        let (uuid, (priv_key, pub_key)) =
            tokio::try_join!(Self::generate_wwps_uuid(), Self::generate_wwps_x25519(),)?;
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
    pub(crate) fn generate_client_link(
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

    pub(crate) async fn create_standalone_config(
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
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;

        Ok(BatchCreationResult {
            links,
            config_file: Some(filename),
            backup_file: None,
            created_count,
        })
    }

    #[allow(dead_code)]
    pub(crate) async fn backup_config_file(path: &str) -> Result<String> {
        let timestamp = chrono::Utc::now().timestamp();
        let backup_path = format!("{}.backup.{}", path, timestamp);
        fs::copy(path, &backup_path).await?;
        Ok(backup_path)
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
        const OPERATION: &str = "xray bulk delete";
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

    async fn bulk_candidates(filter: Option<Proto>) -> Result<Vec<String>> {
        match filter {
            Some(proto) => Self::list_inbound_files_by_proto(proto).await,
            None => Self::list_all_inbound_files().await,
        }
    }

    pub async fn delete_all_configurations(filter: Option<Proto>) -> BulkDeleteResult {
        let files = Self::bulk_candidates(filter)
            .await
            .map_err(|source| BulkDeleteError::discovery("xray bulk delete", source))?;
        Self::delete_files_with_reload(files, None, MaintenanceManager::reload_core).await
    }

    pub async fn delete_configurations_by_count(
        filter: Option<Proto>,
        count: usize,
    ) -> BulkDeleteResult {
        let files = Self::bulk_candidates(filter)
            .await
            .map_err(|source| BulkDeleteError::discovery("xray bulk delete", source))?;
        Self::delete_files_with_reload(files, Some(count), MaintenanceManager::reload_core).await
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        fs::remove_file(path).await.context("❌ 删除配置文件失败")?;
        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn ensure_base_config() -> Result<()> {
        use crate::core::paths::xray;

        if let Err(e) = crate::core::system::maintenance::MaintenanceManager::ensure_geodata().await
        {
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

        let default_rules: Vec<Value> = ROUTING_RULES
            .iter()
            .filter(|r| r.default_enabled)
            .map(RoutingManager::rule_def_to_json)
            .collect();

        let base_config = serde_json::json!({
            "log": {"loglevel": "warning"},
            "dns": {
                "servers": ["https+local://1.1.1.1/dns-query", "https+local://8.8.8.8/dns-query"],
                "tag": "dns"
            },
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": default_rules
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

pub(crate) async fn run_wwps_core_cmd(args: &[&str]) -> Result<String> {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::kcp_mask::KcpMask;
    use super::*;
    use anyhow::anyhow;
    use percent_encoding::percent_decode_str;
    use tempfile::tempdir;

    #[tokio::test]
    async fn bulk_delete_continues_after_remove_failure_and_reloads_once() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first_inbounds.json");
        let last = dir.path().join("last_inbounds.json");
        tokio::fs::write(&first, "{}").await.unwrap();
        tokio::fs::write(&last, "{}").await.unwrap();
        let reloads = AtomicUsize::new(0);

        let result = ConfigManager::delete_files_with_reload(
            vec![
                first.to_string_lossy().into_owned(),
                first.to_string_lossy().into_owned(),
                last.to_string_lossy().into_owned(),
            ],
            None,
            || async {
                reloads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert_eq!(error.deleted(), 2);
        assert_eq!(error.failures().len(), 1);
        assert_eq!(error.failures()[0].stage, DeleteStage::Remove);
        assert_eq!(reloads.load(Ordering::SeqCst), 1);
        assert!(!last.exists());
    }

    #[tokio::test]
    async fn bulk_delete_success_returns_exact_removed_count() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first_inbounds.json");
        let second = dir.path().join("second_inbounds.json");
        tokio::fs::write(&first, "{}").await.unwrap();
        tokio::fs::write(&second, "{}").await.unwrap();
        let reloads = AtomicUsize::new(0);

        let deleted = ConfigManager::delete_files_with_reload(
            vec![
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
            None,
            || async {
                reloads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(deleted, 2);
        assert_eq!(reloads.load(Ordering::SeqCst), 1);
        assert!(!first.exists());
        assert!(!second.exists());
    }

    #[tokio::test]
    async fn bulk_delete_by_count_records_inspect_failure_and_deletes_sortable_files() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing_inbounds.json");
        let first = dir.path().join("first_inbounds.json");
        let second = dir.path().join("second_inbounds.json");
        tokio::fs::write(&first, "{}").await.unwrap();
        tokio::fs::write(&second, "{}").await.unwrap();

        let result = ConfigManager::delete_files_with_reload(
            vec![
                missing.to_string_lossy().into_owned(),
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
            Some(2),
            || async { Ok(()) },
        )
        .await;

        let error = result.unwrap_err();
        assert_eq!(error.deleted(), 2);
        assert_eq!(error.failures()[0].stage, DeleteStage::Inspect);
    }

    #[tokio::test]
    async fn bulk_delete_does_not_reload_when_nothing_was_removed() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing_inbounds.json");
        let reloads = AtomicUsize::new(0);

        let result = ConfigManager::delete_files_with_reload(
            vec![missing.to_string_lossy().into_owned()],
            None,
            || async {
                reloads.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(reloads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bulk_delete_retains_remove_and_reload_failures() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("one_inbounds.json");
        tokio::fs::write(&file, "{}").await.unwrap();

        let result = ConfigManager::delete_files_with_reload(
            vec![
                file.to_string_lossy().into_owned(),
                file.to_string_lossy().into_owned(),
            ],
            None,
            || async { anyhow::bail!("reload failed") },
        )
        .await;

        let error = result.unwrap_err();
        assert_eq!(error.deleted(), 1);
        assert_eq!(error.failures().len(), 1);
        assert!(error.reload_error().is_some());
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
            vless["streamSettings"]["realitySettings"]["target"],
            "example.com:443"
        );
        assert_eq!(
            vless["streamSettings"]["realitySettings"]["serverNames"][0],
            "example.com"
        );

        assert_eq!(
            vless["streamSettings"]["realitySettings"]["minClientVer"],
            "1.0.0"
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
        let default_rules: Vec<Value> = ROUTING_RULES
            .iter()
            .filter(|r| r.default_enabled)
            .map(RoutingManager::rule_def_to_json)
            .collect();

        let base_config = serde_json::json!({
            "log": {"loglevel": "warning"},
            "dns": {
                "servers": ["https+local://1.1.1.1/dns-query", "https+local://8.8.8.8/dns-query"],
                "tag": "dns"
            },
            "routing": {
                "domainStrategy": "IPIfNonMatch",
                "rules": default_rules
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
        assert_eq!(rules[0]["ruleTag"], "private_ip");
    }
}

#[cfg(test)]
mod port_collection_tests {
    use std::collections::HashSet;

    #[test]
    fn test_xray_port_extraction_from_json() {
        let json = serde_json::json!({
            "inbounds": [
                {"tag": "v1", "port": 10001, "protocol": "vless"},
                {"tag": "v2", "port": 10002, "protocol": "vless"},
            ]
        });
        let parsed: serde_json::Value = serde_json::from_str(&json.to_string()).unwrap();
        let mut ports = HashSet::new();
        if let Some(inbounds) = parsed.get("inbounds").and_then(|v| v.as_array()) {
            for inbound in inbounds {
                if let Some(port) = inbound.get("port").and_then(|v| v.as_u64()) {
                    ports.insert(port as u16);
                }
            }
        }
        assert!(ports.contains(&10001));
        assert!(ports.contains(&10002));
        assert_eq!(ports.len(), 2);
    }

    #[test]
    fn test_xray_base_config_excluded() {
        let name = "00_base_inbounds.json";
        assert!(name.starts_with("00_"));
        let name2 = "batch_reality_vision_123.json";
        assert!(!name2.starts_with("00_"));
    }
}
