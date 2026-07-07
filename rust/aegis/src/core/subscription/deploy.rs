use crate::core::paths;
use crate::core::subscription::cert::{self, TlsMode, TlsResult};
use crate::core::subscription::config;
use crate::core::subscription::minisign;
use crate::core::subscription::token::TokenManager;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStep {
    DownloadBinary,
    VerifyBinary,
    WriteBinary,
    SetupTls,
    WriteConfig,
    SetupService,
    OpenFirewall,
    CreateToken,
}

impl DeployStep {
    pub fn index(self) -> u8 {
        match self {
            Self::DownloadBinary => 0,
            Self::VerifyBinary => 1,
            Self::WriteBinary => 2,
            Self::SetupTls => 3,
            Self::WriteConfig => 4,
            Self::SetupService => 5,
            Self::OpenFirewall => 6,
            Self::CreateToken => 7,
        }
    }

    pub fn desc(&self) -> &'static str {
        match self {
            Self::DownloadBinary => "step_download",
            Self::VerifyBinary => "step_verify",
            Self::WriteBinary => "step_write",
            Self::SetupTls => "step_tls",
            Self::WriteConfig => "step_config",
            Self::SetupService => "step_service",
            Self::OpenFirewall => "step_firewall",
            Self::CreateToken => "step_token",
        }
    }

    pub const TOTAL: u8 = 8;
}

pub fn is_valid_ipv4(s: &str) -> bool {
    if s.is_empty() || s.contains(' ') || s.starts_with('.') || s.ends_with('.') {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u8>().is_ok())
}

fn is_172_private(ip: &str) -> bool {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    if parts[0] != "172" {
        return false;
    }
    parts[1]
        .parse::<u8>()
        .map(|second| (16..=31).contains(&second))
        .unwrap_or(false)
}

pub async fn get_public_ipv4() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("create HTTP client failed: {e}"))?;

    let resp = client
        .get("https://api.ipify.org?format=text")
        .send()
        .await
        .map_err(|e| format!("ipify request failed: {e}"))?;

    let ip = resp
        .text()
        .await
        .map_err(|e| format!("ipify response failed: {e}"))?;
    let ip = ip.trim().to_string();

    if !is_valid_ipv4(&ip) {
        return Err(format!("ipify returned invalid IPv4: '{ip}'"));
    }
    // 127.0.0.0/8, 10.0.0.0/8 RFC 1918 class A
    // 172.16.0.0/12 RFC 1918 class B range
    // 192.168.0.0/16 RFC 1918 class C
    if ip.starts_with("127.")
        || ip.starts_with("10.")
        || is_172_private(&ip)
        || ip.starts_with("192.168.")
    {
        return Err(format!("ipify returned private IP: '{ip}'"));
    }
    if ip == "0.0.0.0" {
        return Err("ipify returned 0.0.0.0".to_string());
    }

    Ok(ip)
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

    let binary_url = format!("{}/sub-server", &base_url);
    let sig_url = format!("{}/sub-server.minisig", &base_url);

    let binary_data = client
        .get(&binary_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("download binary failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read binary body failed: {e}"))?;

    let sig_data = client
        .get(&sig_url)
        .header("User-Agent", "wwps-aegis")
        .send()
        .await
        .map_err(|e| format!("download signature failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read signature body failed: {e}"))?;

    Ok((binary_data.to_vec(), sig_data.to_vec()))
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
    std::fs::write(paths::sub_server::BIN, binary_data)
        .map_err(|e| format!("write binary failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            paths::sub_server::BIN,
            std::fs::Permissions::from_mode(0o755),
        )
        .map_err(|e| format!("set permissions failed: {e}"))?;
    }
    Ok(())
}

pub fn write_systemd_service(port: u16) -> Result<(), String> {
    let service_file = "/etc/systemd/system/wwps-sub-server.service";
    let unit = format!(
        "[Unit]\n\
         Description=WWPS Subscription Server\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} --listen-addr=:{port} --aegis-grpc=unix:///var/run/aegis/sub.sock --rate-limit=10\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = paths::sub_server::BIN,
    );
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

pub fn open_firewall_port(port: u16) {
    let _ = std::process::Command::new("ufw")
        .args(["allow", &port.to_string()])
        .status();
}

pub async fn run_deploy<F>(
    params: &DeployParams,
    tm: &TokenManager,
    on_progress: F,
) -> Result<DeployResult, String>
where
    F: Fn(DeployStep, u8, u8) + Send + Sync,
{
    let repo_owner = "NicholasDewar";
    let repo_name = "Wuthering_Waves_Private_Server";

    on_progress(DeployStep::DownloadBinary, 0, DeployStep::TOTAL);
    let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;

    on_progress(DeployStep::VerifyBinary, 1, DeployStep::TOTAL);
    verify_binary(&binary_data, &sig_data, "3", "sub-server")?;

    on_progress(DeployStep::WriteBinary, 2, DeployStep::TOTAL);
    deploy_binary(&binary_data)?;

    on_progress(DeployStep::SetupTls, 3, DeployStep::TOTAL);
    let tls_result = match params.tls_mode {
        TlsMode::DomainAcme => cert::setup_acme_domain(&params.domain)?,
        TlsMode::IpAcme => cert::setup_acme_ip(&params.domain)?,
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

    on_progress(DeployStep::WriteConfig, 4, DeployStep::TOTAL);
    let addr = format!("0.0.0.0:{}", params.port);
    config::write_config(&addr, &tls_cert, &tls_key, params.rate_limit)?;

    on_progress(DeployStep::SetupService, 5, DeployStep::TOTAL);
    write_systemd_service(params.port)?;

    on_progress(DeployStep::OpenFirewall, 6, DeployStep::TOTAL);
    open_firewall_port(params.port);

    on_progress(DeployStep::CreateToken, 7, DeployStep::TOTAL);
    let token_record = tm
        .create_token("default", &[])
        .map_err(|e| format!("create token failed: {e}"))?;

    let port_part = if params.tls_mode == TlsMode::ReverseProxy {
        String::new()
    } else {
        format!(":{}", params.port)
    };
    let sub_url = format!(
        "https://{}{}/sub/{}",
        params.domain, port_part, token_record.token
    );

    Ok(DeployResult {
        sub_url,
        token: token_record.token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_ipv4_accepts_public_ip() {
        assert!(is_valid_ipv4("8.8.8.8"));
    }

    #[test]
    fn is_valid_ipv4_accepts_loopback() {
        assert!(is_valid_ipv4("127.0.0.1"));
    }

    #[test]
    fn is_valid_ipv4_rejects_empty() {
        assert!(!is_valid_ipv4(""));
    }

    #[test]
    fn is_valid_ipv4_rejects_with_space() {
        assert!(!is_valid_ipv4("192. 168.1.1"));
    }

    #[test]
    fn is_valid_ipv4_rejects_too_few_octets() {
        assert!(!is_valid_ipv4("192.168.1"));
    }

    #[test]
    fn is_valid_ipv4_rejects_too_many_octets() {
        assert!(!is_valid_ipv4("1.2.3.4.5"));
    }

    #[test]
    fn is_valid_ipv4_rejects_overflowing_octet() {
        assert!(!is_valid_ipv4("256.1.2.3"));
    }

    #[test]
    fn is_valid_ipv4_rejects_leading_dot() {
        assert!(!is_valid_ipv4(".1.2.3.4"));
    }

    #[test]
    fn is_valid_ipv4_rejects_trailing_dot() {
        assert!(!is_valid_ipv4("1.2.3.4."));
    }

    #[test]
    fn is_172_private_accepts_172_16() {
        assert!(is_172_private("172.16.0.1"));
    }

    #[test]
    fn is_172_private_accepts_172_31() {
        assert!(is_172_private("172.31.255.255"));
    }

    #[test]
    fn is_172_private_rejects_172_15() {
        assert!(!is_172_private("172.15.0.1"));
    }

    #[test]
    fn is_172_private_rejects_172_32() {
        assert!(!is_172_private("172.32.0.1"));
    }

    #[test]
    fn is_172_private_rejects_non_172() {
        assert!(!is_172_private("10.0.0.1"));
    }

    #[test]
    fn is_172_private_rejects_invalid_second_octet() {
        assert!(!is_172_private("172.xyz.0.1"));
    }

    #[test]
    fn deploy_step_index_download_is_0() {
        assert_eq!(DeployStep::DownloadBinary.index(), 0);
    }

    #[test]
    fn deploy_step_index_create_token_is_7() {
        assert_eq!(DeployStep::CreateToken.index(), 7);
    }

    #[test]
    fn deploy_step_total_is_8() {
        assert_eq!(DeployStep::TOTAL, 8);
    }

    #[test]
    fn deploy_step_desc_returns_non_empty() {
        for step in &[
            DeployStep::DownloadBinary,
            DeployStep::VerifyBinary,
            DeployStep::WriteBinary,
            DeployStep::SetupTls,
            DeployStep::WriteConfig,
            DeployStep::SetupService,
            DeployStep::OpenFirewall,
            DeployStep::CreateToken,
        ] {
            assert!(!step.desc().is_empty());
        }
    }
}
