use crate::core::paths::{singbox, xray};
use crate::logic::firewall_scanner::FirewallScanner;
use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs;

const PORT_ALLOC_FILE: &str = "/etc/wwps/.port_alloc";
const XRAY_PORT_MIN: u16 = 10000;
const XRAY_PORT_MAX: u16 = 60000;
const HOP_SIZE: u16 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortAllocData {
    #[serde(default)]
    pub locked_ranges: Vec<LockedRange>,
    #[serde(default)]
    pub initialized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedRange {
    pub start: u16,
    pub end: u16,
    pub protocol: String,
    pub created_at: i64,
}

async fn load_port_alloc() -> Result<PortAllocData> {
    let path = PathBuf::from(PORT_ALLOC_FILE);
    if !path.exists() {
        return Ok(PortAllocData::default());
    }
    let content = fs::read_to_string(&path).await?;
    let data: PortAllocData = serde_json::from_str(&content).context("解析端口分配数据失败")?;
    Ok(data)
}

async fn save_port_alloc(data: &PortAllocData) -> Result<()> {
    let path = PathBuf::from(PORT_ALLOC_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let content = serde_json::to_string_pretty(data)?;
    fs::write(&path, content).await?;
    Ok(())
}

pub struct PortAllocator;

impl PortAllocator {
    pub async fn init() -> Result<()> {
        let mut data = load_port_alloc().await?;
        
        if data.initialized {
            return Ok(());
        }

        let mut occupied_ports = HashSet::new();

        occupied_ports.insert(22);
        occupied_ports.insert(80);
        occupied_ports.insert(443);

        if let Ok(xray_ports) = FirewallScanner::scan_dir_for_ports(xray::CONF_DIR).await {
            occupied_ports.extend(xray_ports);
        }

        if let Ok(sb_dirs) = fs::read_dir(&singbox::CONF_DIR).await {
            let mut dir = sb_dirs;
            while let Ok(Some(entry)) = dir.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") && !name.starts_with("00_") {
                        let path = entry.path();
                        if let Ok(content) = fs::read_to_string(&path).await {
                            if let Ok(ports) = Self::extract_ports_from_json(&content) {
                                occupied_ports.extend(ports);
                            }
                        }
                    }
                }
            }
        }

        let mut all_ports: Vec<u16> = (XRAY_PORT_MIN..XRAY_PORT_MAX).collect();
        all_ports.retain(|p| !occupied_ports.contains(p));

        if !all_ports.is_empty() {
            all_ports.sort();
            
            let start = *all_ports.first().unwrap();
            let end = start + HOP_SIZE - 1;
            
            if end <= XRAY_PORT_MAX {
                data.locked_ranges.push(LockedRange {
                    start,
                    end,
                    protocol: "hysteria2".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                });
            }
        }

        data.initialized = true;
        save_port_alloc(&data).await?;
        
        log::info!("端口分配器初始化完成，锁定范围: {} - {}", 
            data.locked_ranges.first().map(|r| r.start).unwrap_or(0),
            data.locked_ranges.first().map(|r| r.end).unwrap_or(0)
        );
        
        Ok(())
    }

    fn extract_ports_from_json(content: &str) -> Result<Vec<u16>> {
        let mut ports = Vec::new();
        
        let re = regex::Regex::new(r#""listen_port"\s*:\s*(\d+)"#).unwrap();
        for cap in re.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                if let Ok(p) = m.as_str().parse::<u16>() {
                    ports.push(p);
                }
            }
        }
        
        Ok(ports)
    }

    pub async fn get_locked_ranges() -> Vec<(u16, u16)> {
        let data = load_port_alloc().await.unwrap_or_default();
        data.locked_ranges
            .iter()
            .map(|r| (r.start, r.end))
            .collect()
    }

    pub async fn is_port_in_locked_range(port: u16) -> bool {
        let ranges = Self::get_locked_ranges().await;
        ranges.iter().any(|(start, end)| port >= *start && port <= *end)
    }

    pub async fn allocate_hysteria2() -> Result<(u16, (u16, u16))> {
        let mut data = load_port_alloc().await?;
        
        if let Some(existing) = data.locked_ranges.first() {
            if existing.protocol == "hysteria2" {
                return Ok((existing.start, (existing.start + 1, existing.end)));
            }
        }

        let mut occupied = HashSet::new();
        
        if let Ok(xray_ports) = FirewallScanner::scan_dir_for_ports(xray::CONF_DIR).await {
            occupied.extend(xray_ports);
        }

        if let Ok(sb_dirs) = fs::read_dir(&singbox::CONF_DIR).await {
            let mut dir = sb_dirs;
            while let Ok(Some(entry)) = dir.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") && !name.starts_with("00_") {
                        let path = entry.path();
                        if let Ok(content) = fs::read_to_string(&path).await {
                            if let Ok(ports) = Self::extract_ports_from_json(&content) {
                                occupied.extend(ports);
                            }
                        }
                    }
                }
            }
        }

        let mut rng = StdRng::from_entropy();
        
        let main_port = loop {
            let p = rng.gen_range(XRAY_PORT_MIN..XRAY_PORT_MAX);
            
            let ranges = Self::get_locked_ranges().await;
            let in_locked = ranges.iter().any(|(s, e)| p >= *s && p <= *e);
            if in_locked {
                continue;
            }
            
            if occupied.contains(&p) {
                continue;
            }

            if Self::check_port_available(p).await {
                break p;
            }
        };

        let hop_start = main_port + 1;
        let hop_end = (hop_start + HOP_SIZE - 1).min(XRAY_PORT_MAX);
        let hop_range = (hop_start, hop_end);

        let locked = LockedRange {
            start: main_port,
            end: hop_end,
            protocol: "hysteria2".to_string(),
            created_at: chrono::Utc::now().timestamp(),
        };
        
        data.locked_ranges.push(locked);
        save_port_alloc(&data).await?;

        log::info!("Hysteria2 端口分配: 主端口 {}, 跳跃范围 {}-{}", 
            main_port, hop_range.0, hop_range.1);

        Ok((main_port, hop_range))
    }

    async fn check_port_available(port: u16) -> bool {
        let output = tokio::process::Command::new("ss")
            .args(["-t", "-l", "-n"])
            .output()
            .await;

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return !stdout.contains(&format!(":{}", port));
        }

        false
    }

    pub async fn release_hysteria2_range() -> Result<()> {
        let mut data = load_port_alloc().await?;
        data.locked_ranges.retain(|r| r.protocol != "hysteria2");
        save_port_alloc(&data).await?;
        log::info!("Hysteria2 端口范围已释放");
        Ok(())
    }

    pub async fn get_hysteria2_range() -> Option<(u16, u16)> {
        let data = load_port_alloc().await.unwrap_or_default();
        data.locked_ranges
            .iter()
            .find(|r| r.protocol == "hysteria2")
            .map(|r| (r.start, r.end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_range_serialization() {
        let range = LockedRange {
            start: 1000,
            end: 1100,
            protocol: "hysteria2".to_string(),
            created_at: 1234567890,
        };
        let json = serde_json::to_string(&range).unwrap();
        assert!(json.contains("hysteria2"));
    }

    #[test]
    fn test_port_alloc_data_serialization() {
        let data = PortAllocData {
            locked_ranges: vec![LockedRange {
                start: 1000,
                end: 1100,
                protocol: "hysteria2".to_string(),
                created_at: 1234567890,
            }],
            initialized: true,
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("1000"));
        assert!(json.contains("1100"));
    }
}