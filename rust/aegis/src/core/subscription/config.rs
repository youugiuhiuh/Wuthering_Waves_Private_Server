use crate::core::paths;
use serde::Serialize;
use std::fs;

#[derive(Serialize)]
pub struct SubServerConfig {
    pub listen_addr: String,
    pub tls_cert: String,
    pub tls_key: String,
    pub aegis_grpc: String,
    pub rate_limit: u32,
    pub cache_ttl: u32,
}

pub fn write_config(
    addr: &str,
    tls_cert: &str,
    tls_key: &str,
    rate_limit: u32,
) -> Result<(), String> {
    let cfg = SubServerConfig {
        listen_addr: addr.to_string(),
        tls_cert: tls_cert.to_string(),
        tls_key: tls_key.to_string(),
        aegis_grpc: format!("unix://{}", paths::sub_server::GRPC_SOCK),
        rate_limit,
        cache_ttl: 60,
    };
    let json =
        serde_json::to_string_pretty(&cfg).map_err(|e| format!("serialize config failed: {e}"))?;
    let cfg_dir = paths::sub_server::DIR;
    fs::create_dir_all(cfg_dir).map_err(|e| format!("create config dir failed: {e}"))?;
    fs::write(paths::sub_server::CONFIG_FILE, &json)
        .map_err(|e| format!("write config failed: {e}"))?;
    Ok(())
}
