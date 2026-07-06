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
         ExecStart={bin} --listen-addr=:{port} --config {cfg}\n\
         Restart=always\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = paths::sub_server::BIN,
        cfg = paths::sub_server::CONFIG_FILE,
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

pub async fn run_deploy(params: &DeployParams, tm: &TokenManager) -> Result<DeployResult, String> {
    let repo_owner = "NicholasDewar";
    let repo_name = "Wuthering_Waves_Private_Server";

    let (binary_data, sig_data) = download_binary(repo_owner, repo_name).await?;
    verify_binary(&binary_data, &sig_data, "3", "sub-server")?;
    deploy_binary(&binary_data)?;

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

    let addr = format!("0.0.0.0:{}", params.port);
    config::write_config(&addr, &tls_cert, &tls_key, params.rate_limit)?;

    write_systemd_service(params.port)?;
    open_firewall_port(params.port);

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
