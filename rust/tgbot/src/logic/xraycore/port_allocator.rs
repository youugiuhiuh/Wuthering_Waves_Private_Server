use crate::core::paths::{singbox, xray};
use crate::logic::security::firewall_scanner::FirewallScanner;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs;

const PORT_ALLOC_FILE: &str = "/etc/wwps/.port_alloc";
const XRAY_PORT_MIN: u16 = 10000;
const XRAY_PORT_MAX: u16 = 60000;
const HOP_SIZE: u16 = 100;

const WWPS_BOX_CONF_DIR: &str = "/etc/wwps/wwps-box/conf";

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
    pub async fn check_hysteria2_limit() -> Result<bool> {
        let conf_dir = PathBuf::from(WWPS_BOX_CONF_DIR);
        if !conf_dir.exists() {
            return Ok(true);
        }

        let mut count = 0;
        let mut entries = fs::read_dir(&conf_dir).await?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str()
                && name.ends_with(".json")
            {
                let path = entry.path();
                if let Ok(content) = fs::read_to_string(&path).await
                    && content.contains("hysteria2")
                {
                    count += 1;
                }
            }
        }

        Ok(count < 50)
    }

    async fn scan_all_occupied_ports() -> Result<HashSet<u16>> {
        let mut occupied = HashSet::new();

        occupied.insert(22);
        occupied.insert(80);
        occupied.insert(443);

        if let Ok(ports) = FirewallScanner::scan_dir_for_ports(xray::CONF_DIR).await {
            occupied.extend(ports);
        }

        if let Ok(entries) = fs::read_dir(&singbox::CONF_DIR).await {
            let mut dir = entries;
            while let Ok(Some(entry)) = dir.next_entry().await {
                if let Some(name) = entry.file_name().to_str()
                    && name.ends_with(".json")
                    && !name.starts_with("00_")
                {
                    let path = entry.path();
                    if let Ok(content) = fs::read_to_string(&path).await
                        && let Ok(ports) = Self::extract_ports_from_json(&content)
                    {
                        occupied.extend(ports);
                    }
                }
            }
        }

        Ok(occupied)
    }

    fn find_consecutive_range(occupied: &HashSet<u16>, size: u16) -> Result<u16> {
        for main_port in XRAY_PORT_MIN..=(XRAY_PORT_MAX.saturating_sub(size)) {
            let mut found = true;
            for port in main_port..(main_port + size) {
                if occupied.contains(&port) {
                    found = false;
                    break;
                }
            }
            if found {
                return Ok(main_port);
            }
        }
        anyhow::bail!(
            "在 {} 范围内找不到连续的 {} 个空闲端口",
            XRAY_PORT_MIN,
            size
        )
    }

    fn extract_ports_from_json(content: &str) -> Result<Vec<u16>> {
        let mut ports = Vec::new();

        let re = regex::Regex::new(r#""listen_port"\s*:\s*(\d+)"#).unwrap();
        for cap in re.captures_iter(content) {
            if let Some(m) = cap.get(1)
                && let Ok(p) = m.as_str().parse::<u16>()
            {
                ports.push(p);
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
        ranges
            .iter()
            .any(|(start, end)| port >= *start && port <= *end)
    }

    pub async fn allocate_hysteria2() -> Result<(u16, (u16, u16))> {
        let occupied = Self::scan_all_occupied_ports().await?;
        let main_port = Self::find_consecutive_range(&occupied, HOP_SIZE)?;
        let hop_end = main_port + 99;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut data = load_port_alloc().await.unwrap_or_default();
        data.locked_ranges.push(LockedRange {
            start: main_port,
            end: hop_end,
            protocol: "hysteria2".to_string(),
            created_at: now,
        });
        save_port_alloc(&data).await?;

        log::info!(
            "Hysteria2 端口分配: 主端口 {}, 跳跃范围 {}-{}",
            main_port,
            main_port + 1,
            hop_end
        );

        Ok((main_port, (main_port + 1, hop_end)))
    }

    pub async fn release_hysteria2_range(main_port: u16) -> Result<()> {
        let mut data = load_port_alloc().await.unwrap_or_default();
        let before = data.locked_ranges.len();
        data.locked_ranges
            .retain(|r| !(r.protocol == "hysteria2" && r.start == main_port));

        if data.locked_ranges.len() < before {
            save_port_alloc(&data).await?;
            log::info!(
                "Hysteria2 端口范围已释放: 主端口 {}",
                main_port
            );
        } else {
            log::warn!(
                "Hysteria2 端口范围未找到: 主端口 {} (可能已被释放)",
                main_port
            );
        }

        Ok(())
    }

    pub async fn get_hysteria2_range() -> Option<(u16, (u16, u16))> {
        let data = load_port_alloc().await.unwrap_or_default();
        data.locked_ranges
            .iter()
            .find(|r| r.protocol == "hysteria2")
            .map(|r| {
                let main_port = r.start;
                let hop_start = main_port + 1;
                let hop_end = r.end;
                (main_port, (hop_start, hop_end))
            })
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

    #[test]
    fn test_find_consecutive_range() {
        let mut occupied = HashSet::new();
        occupied.insert(100);
        occupied.insert(101);

        let result = PortAllocator::find_consecutive_range(&occupied, 10);
        assert!(result.is_ok());
        let start = result.unwrap();
        assert!(start < 100 || start > 111);
    }

    #[test]
    fn test_release_removes_matching_range() {
        let mut data = PortAllocData {
            locked_ranges: vec![
                LockedRange {
                    start: 10000,
                    end: 10099,
                    protocol: "hysteria2".to_string(),
                    created_at: 1234567890,
                },
                LockedRange {
                    start: 20000,
                    end: 20099,
                    protocol: "hysteria2".to_string(),
                    created_at: 1234567891,
                },
            ],
            initialized: true,
        };
        data.locked_ranges.retain(|r| !(r.protocol == "hysteria2" && r.start == 10000));
        assert_eq!(data.locked_ranges.len(), 1);
        assert_eq!(data.locked_ranges[0].start, 20000);
    }

    #[test]
    fn test_release_no_match_is_noop() {
        let data = PortAllocData {
            locked_ranges: vec![LockedRange {
                start: 20000,
                end: 20099,
                protocol: "hysteria2".to_string(),
                created_at: 1234567890,
            }],
            initialized: true,
        };
        let filtered: Vec<_> = data
            .locked_ranges
            .iter()
            .filter(|r| !(r.protocol == "hysteria2" && r.start == 9999))
            .cloned()
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].start, 20000);
    }
}
