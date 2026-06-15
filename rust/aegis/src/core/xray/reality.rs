use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use once_cell::sync::Lazy;
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::path::Path;
use tokio::fs;

use crate::core::cmd_async::run_cmd_output;
use crate::core::types::{BatchCreationResult, IpVersion};
use super::config::ConfigManager;
use super::config::run_wwps_core_cmd;

pub(crate) static REALITY_PQ_SEED: Lazy<String> = Lazy::new(|| {
    if let Ok(v) = std::env::var("AEGIS_REALITY_PQ_SEED") {
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

pub(crate) static REALITY_PQ_VERIFY: Lazy<String> = Lazy::new(|| {
    if let Ok(v) = std::env::var("AEGIS_REALITY_PQ_VERIFY") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    if let Ok(v) = std::env::var("AEGIS_REALITY_PQ_PUB") {
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

pub(crate) fn reality_pq_verify_as_base64url(raw: &str) -> Option<String> {
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

impl ConfigManager {
    pub fn is_reality_pq_configured() -> bool {
        if std::env::var("AEGIS_REALITY_PQ_SEED")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if std::env::var("AEGIS_REALITY_PQ_VERIFY")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if std::env::var("AEGIS_REALITY_PQ_PUB")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        Path::new("/etc/wwps/reality_pq.seed").exists()
            || Path::new("/etc/wwps/reality_pq.pub").exists()
    }

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

    pub async fn generate_reality_pq_keys() -> Result<()> {
        const PQ_SEED_PATH: &str = "/etc/wwps/reality_pq.seed";
        const PQ_PUB_PATH: &str = "/etc/wwps/reality_pq.pub";
        let stdout = match run_wwps_core_cmd(&["mldsa65"]).await {
            Ok(out) => out,
            Err(_) => {
                let (status, out, err) =
                    run_cmd_output("xray", &["mldsa65"], ConfigManager::TIMEOUT_WWPS_CORE).await?;
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

    pub async fn batch_create_reality_vision_enhanced(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        let (host, _) = ConfigManager::resolve_public_hosts(
            ip_version,
            crate::core::system::SystemMonitor::get_public_ip().await,
            crate::core::system::SystemMonitor::get_public_ipv6().await,
        )?;

        let mut rng = StdRng::from_entropy();
        let geoip = crate::core::network::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;

        let mut selector = crate::core::sni::selector::SNISelector::get_for_country(&country_code);

        let mut links = Vec::new();
        let mut batch_configs = Vec::new();

        let port_443_available =
            crate::core::system::maintenance::MaintenanceManager::is_port_available(443).await;

        for i in 0..count {
            let sni = selector.get_next();

            let pq_ok = crate::core::security::tls_probe::sni_is_pq_friendly(&sni).await;

            let preferred = if i == 0 && port_443_available {
                Some(443u16)
            } else {
                None
            };
            let (port, uuid, priv_key, pub_key, short_id, sni, email, tag, path) =
                ConfigManager::generate_enhanced_config(&mut rng, sni, i, super::config::Proto::Vision, preferred).await?;

            let config = ConfigManager::build_reality_vless_inbound(
                &tag,
                port,
                &uuid,
                &email,
                &sni,
                &pub_key,
                &priv_key,
                &short_id,
                ip_version,
                super::config::Proto::Vision,
                path.as_deref(),
                pq_ok,
            );

            batch_configs.push(config);

            let link = ConfigManager::generate_client_link(
                &uuid,
                &host,
                port,
                &sni,
                &pub_key,
                &short_id,
                &email,
                ip_version,
                super::config::Proto::Vision,
                path.as_deref(),
                None,
                pq_ok,
            );
            links.push(link);

            let _ = crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        ConfigManager::create_standalone_config(batch_configs, links, super::config::Proto::Vision).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;

    #[test]
    fn test_reality_pq_verify_as_base64url() {
        let bytes_with_special = b"\xfc\xfd\xfe\xff";
        let std_b64 = general_purpose::STANDARD.encode(bytes_with_special);
        let out = reality_pq_verify_as_base64url(&std_b64).expect("应成功转换");
        assert!(!out.contains('+'));
        assert!(!out.contains('/'));
        assert_eq!(
            general_purpose::URL_SAFE_NO_PAD.decode(&out).ok(),
            Some(bytes_with_special.to_vec())
        );

        let url_b64 = general_purpose::URL_SAFE_NO_PAD.encode(b"world");
        let out2 = reality_pq_verify_as_base64url(&url_b64).expect("应成功转换");
        assert_eq!(out2, url_b64);

        assert!(reality_pq_verify_as_base64url("").is_none());
        assert!(reality_pq_verify_as_base64url("!!!").is_none());
    }
}
