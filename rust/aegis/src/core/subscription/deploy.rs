use crate::core::paths;
use crate::core::subscription::cert::{self, TlsMode, TlsResult};
use crate::core::subscription::config;
use crate::core::subscription::minisign;
use crate::core::subscription::token::TokenManager;
use crate::core::system::SystemMonitor;
use tokio::io::AsyncWriteExt;

pub struct DeployParams {
    pub domain: String,
    pub port: u16,
    pub rate_limit: u32,
    pub tls_mode: TlsMode,
}

pub struct DeployResult {
    pub sub_url: String,
    pub token: String,
}

pub fn should_verify_binary(sig_data: &[u8]) -> bool {
    !sig_data.is_empty()
}

pub fn resolve_binary_name() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "sub-server-arm64"
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        "sub-server"
    }
}

pub async fn download_binary(
    repo_owner: &str,
    repo_name: &str,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("build client failed: {e}"))?;

    let release_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        repo_owner, repo_name
    );
    let resp = client
        .get(&release_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("fetch release info failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read response body failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("GitHub API returned {status}: {body}"));
    }
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse release JSON failed: {e}"))?;
    let tag_name = json["tag_name"]
        .as_str()
        .ok_or_else(|| "tag_name not found in release".to_string())?;

    let base_url = format!(
        "https://github.com/{}/{}/releases/download/{}",
        repo_owner, repo_name, tag_name
    );

    let binary_url = format!("{}/{}", &base_url, resolve_binary_name());
    let sig_url = format!("{}/sub-server.minisig", &base_url);

    // Stream binary download directly to a temp file to avoid OOM
    let tmp_path = format!("{}.tmp", paths::sub_server::BIN);
    let bin_resp = client
        .get(&binary_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("download binary failed: {e}"))?;
    let mut tmp_file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("create temp file failed: {e}"))?;
    let mut stream = bin_resp.bytes_stream();
    use tokio_stream::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream chunk failed: {e}"))?;
        tmp_file
            .write_all(&chunk)
            .await
            .map_err(|e| format!("write chunk failed: {e}"))?;
    }
    tmp_file
        .flush()
        .await
        .map_err(|e| format!("flush failed: {e}"))?;

    // Read the full binary into memory only after download completes
    let binary_data = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| format!("read temp file failed: {e}"))?;

    // Minisig is small, download directly
    // If the signature file doesn't exist in the release (404), return
    // empty sig data so callers can soft-fail verification.
    let sig_resp = client
        .get(&sig_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("download signature failed: {e}"))?;
    let sig_data = if sig_resp.status().as_u16() == 404 {
        log::warn!("minisig signature not found at {}", sig_url);
        Vec::new()
    } else {
        sig_resp
            .bytes()
            .await
            .map_err(|e| format!("read signature body failed: {e}"))?
            .to_vec()
    };

    Ok((binary_data, sig_data))
}

pub fn verify_binary(
    binary_data: &[u8],
    sig_data: &[u8],
    expected_version: &str,
    expected_asset: &str,
) -> Result<(), String> {
    let info = minisign::verify_minisign(binary_data, sig_data)?;
    let (version, asset) = minisign::parse_trusted_comment(&info.trusted_comment)?;
    if !version.starts_with(expected_version) {
        return Err(format!(
            "version mismatch: expected prefix '{}', got '{}'",
            expected_version, version
        ));
    }
    if asset != expected_asset {
        return Err(format!(
            "asset mismatch: expected '{}', got '{}'",
            expected_asset, asset
        ));
    }
    Ok(())
}

pub fn deploy_binary(binary_data: &[u8]) -> Result<(), String> {
    let tmp_path = format!("{}.tmp", paths::sub_server::BIN);
    // Clean up any stale temp file
    let _ = std::fs::remove_file(&tmp_path);
    std::fs::write(&tmp_path, binary_data).map_err(|e| format!("write binary failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("set permissions failed: {e}"))?;
    }
    std::fs::rename(&tmp_path, paths::sub_server::BIN)
        .map_err(|e| format!("rename binary failed: {e}"))?;
    Ok(())
}

pub fn generate_systemd_unit(port: u16, tls_cert: &str, tls_key: &str) -> String {
    let tls_flags = if !tls_cert.is_empty() && !tls_key.is_empty() {
        format!(" --tls-cert={} --tls-key={}", tls_cert, tls_key)
    } else {
        String::new()
    };
    format!(
        "[Unit]\n\
         Description=WWPS Subscription Server\n\
         After=network.target\n\
         After=wwps-aegis.service\n\
         BindsTo=wwps-aegis.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} --listen-addr=:{port} --aegis-grpc=unix:///var/run/aegis/sub.sock --rate-limit=10{tls_flags}\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = paths::sub_server::BIN,
    )
}

pub fn write_systemd_service(port: u16, tls_cert: &str, tls_key: &str) -> Result<(), String> {
    let service_file = "/etc/systemd/system/wwps-sub-server.service";
    let unit = generate_systemd_unit(port, tls_cert, tls_key);
    std::fs::write(service_file, &unit).map_err(|e| format!("write systemd unit failed: {e}"))?;

    let status = std::process::Command::new("systemctl")
        .arg("daemon-reload")
        .status()
        .map_err(|e| format!("systemctl daemon-reload failed: {e}"))?;
    if !status.success() {
        return Err("systemctl daemon-reload failed".to_string());
    }

    let status = std::process::Command::new("systemctl")
        .args(["enable", paths::sub_server::SERVICE])
        .status()
        .map_err(|e| format!("systemctl enable failed: {e}"))?;
    if !status.success() {
        return Err("systemctl enable failed".to_string());
    }

    let status = std::process::Command::new("systemctl")
        .args(["restart", paths::sub_server::SERVICE])
        .status()
        .map_err(|e| format!("systemctl restart failed: {e}"))?;
    if !status.success() {
        return Err("systemctl restart failed".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_download_retry_exhausts_attempts() {
        let result = super::download_with_retry(
            || async { Err::<Vec<u8>, String>("mock failure".to_string()) },
            3,
            std::time::Duration::from_millis(1),
        )
        .await;
        assert!(result.is_err(), "should fail after exhausting retries");
    }

    #[tokio::test]
    async fn test_grpc_readiness_timeout() {
        let result =
            super::wait_for_grpc_socket("/nonexistent/sock", std::time::Duration::from_millis(10))
                .await;
        assert!(result.is_err(), "should timeout on non-existent socket");
        let err = result.unwrap_err();
        assert!(err.contains("timed out"), "error should mention timeout");
    }

    #[test]
    fn test_should_verify_binary() {
        assert!(!super::should_verify_binary(&[]), "empty sig = skip");
        assert!(
            super::should_verify_binary(&[0u8; 64]),
            "non-empty sig = verify"
        );
    }

    #[test]
    fn test_resolve_binary_name() {
        let name = super::resolve_binary_name();
        assert!(
            name == "sub-server" || name == "sub-server-arm64",
            "binary name should match known architectures"
        );
    }

    #[test]
    fn test_systemd_unit_has_aegis_dependency() {
        let port = 8443;
        let cert = "/etc/wwps/sub-server/certs/fullchain.pem";
        let key = "/etc/wwps/sub-server/certs/privkey.pem";
        let result = super::generate_systemd_unit(port, cert, key);
        assert!(
            result.contains("After=wwps-aegis.service"),
            "unit should depend on aegis"
        );
        assert!(
            result.contains("BindsTo=wwps-aegis.service"),
            "unit should bind to aegis"
        );
    }
}

pub fn open_firewall_port(port: u16) {
    let _ = std::process::Command::new("ufw")
        .args(["allow", &port.to_string()])
        .status();
}

pub async fn wait_for_grpc_socket(
    socket_path: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        if tokio::fs::metadata(socket_path).await.is_ok() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(format!("timed out waiting for gRPC socket: {socket_path}"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

pub async fn download_with_retry<F, Fut, T>(
    f: F,
    max_attempts: u32,
    base_delay: std::time::Duration,
) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last_err = String::new();
    for attempt in 1..=max_attempts {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = e;
                if attempt < max_attempts {
                    let delay = base_delay * (2u64.pow(attempt - 1) as u32);
                    log::warn!(
                        "download attempt {}/{} failed, retrying in {}s: {}",
                        attempt,
                        max_attempts,
                        delay.as_secs(),
                        last_err
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(format!(
        "download failed after {max_attempts} attempts: {last_err}"
    ))
}

pub async fn run_deploy(params: &DeployParams, tm: &TokenManager) -> Result<DeployResult, String> {
    let repo_owner = "youugiuhiuh";
    let repo_name = "Wuthering_Waves_Private_Server";

    // Stop existing sub-server service before overwriting binary
    let _ = std::process::Command::new("systemctl")
        .args(["stop", paths::sub_server::SERVICE])
        .status();

    let (binary_data, sig_data) = download_with_retry(
        || download_binary(repo_owner, repo_name),
        3,
        std::time::Duration::from_secs(2),
    )
    .await?;
    if should_verify_binary(&sig_data) {
        verify_binary(&binary_data, &sig_data, "3", resolve_binary_name())?;
    } else {
        log::warn!("no minisig signature found, skipping binary verification");
    }
    deploy_binary(&binary_data)?;

    // Auto-detect public IP when no domain was provided
    let effective_domain = if params.domain.is_empty() || params.domain == "0.0.0.0" {
        match SystemMonitor::get_public_ip().await {
            Ok(ip) => {
                log::info!("auto-detected public IP: {}", ip);
                ip
            }
            Err(e) => {
                return Err(format!("failed to detect public IP: {e}"));
            }
        }
    } else {
        params.domain.clone()
    };

    let tls_result = match params.tls_mode {
        TlsMode::DomainAcme => match cert::setup_acme_domain(&effective_domain) {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "acme.sh domain cert failed ({}), falling back to self-signed",
                    e
                );
                cert::setup_self_signed()?
            }
        },
        TlsMode::IpAcme => match cert::setup_acme_ip(&effective_domain) {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "acme.sh IP cert failed ({}), falling back to self-signed",
                    e
                );
                cert::setup_self_signed()?
            }
        },
        TlsMode::SelfSigned => cert::setup_self_signed()?,
        TlsMode::ReverseProxy => TlsResult::SkippedReverseProxy,
    };

    let (tls_cert, tls_key) = match &tls_result {
        TlsResult::Ready {
            cert_path,
            key_path,
        } => (cert_path.clone(), key_path.clone()),
        TlsResult::SkippedReverseProxy => (String::new(), String::new()),
    };

    let addr = format!("0.0.0.0:{}", params.port);
    config::write_config(&addr, &tls_cert, &tls_key, params.rate_limit)?;

    write_systemd_service(params.port, &tls_cert, &tls_key)?;
    open_firewall_port(params.port);
    wait_for_grpc_socket(
        paths::sub_server::GRPC_SOCK,
        std::time::Duration::from_secs(30),
    )
    .await?;

    let token_record = tm
        .create_token("default", &[])
        .map_err(|e| format!("create token failed: {e}"))?;

    let display_host = &effective_domain;
    let port_part = if params.tls_mode == TlsMode::ReverseProxy {
        String::new()
    } else {
        format!(":{}", params.port)
    };
    let sub_url = format!(
        "https://{}{}/sub/{}",
        display_host, port_part, token_record.token
    );

    Ok(DeployResult {
        sub_url,
        token: token_record.token,
    })
}
