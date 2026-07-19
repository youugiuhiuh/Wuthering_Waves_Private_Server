use crate::core::paths::{singbox, xray};
use crate::core::security::firewall_scanner::FirewallScanner;
use crate::core::security::secure_fs::{atomic_write_at_async, open_dir};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
#[cfg(test)]
use std::time::Duration;
use tokio::fs;

const PORT_ALLOC_FILE: &str = "/etc/wwps/.port_alloc";
const PORT_ALLOC_LOCK_FILE: &str = "/etc/wwps/.port_alloc.lock";
const XRAY_PORT_MIN: u16 = 10000;
const XRAY_PORT_MAX: u16 = 60000;
const HOP_SIZE: u16 = 100;

#[derive(Clone)]
struct AllocatorPaths {
    state_file: PathBuf,
    #[allow(dead_code)]
    lock_file: PathBuf,
    xray_conf_dir: PathBuf,
    singbox_conf_dir: PathBuf,
    #[cfg(test)]
    after_load_delay: Duration,
}

impl AllocatorPaths {
    fn production() -> Self {
        Self {
            state_file: PathBuf::from(PORT_ALLOC_FILE),
            lock_file: PathBuf::from(PORT_ALLOC_LOCK_FILE),
            xray_conf_dir: PathBuf::from(xray::CONF_DIR),
            singbox_conf_dir: PathBuf::from(singbox::CONF_DIR),
            #[cfg(test)]
            after_load_delay: Duration::ZERO,
        }
    }
}

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

async fn load_port_alloc(path: &Path) -> Result<PortAllocData> {
    if !fs::try_exists(path).await? {
        return Ok(PortAllocData::default());
    }
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("读取端口分配数据失败: {}", path.display()))?;
    serde_json::from_str(&content).context("解析端口分配数据失败")
}

async fn save_port_alloc(path: &Path, data: &PortAllocData) -> Result<()> {
    let parent = path.parent().context("端口分配文件没有父目录")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("创建端口分配目录失败: {}", parent.display()))?;
    let name = path
        .file_name()
        .context("端口分配文件没有文件名")?
        .to_os_string();
    let bytes = serde_json::to_vec_pretty(data).context("序列化端口分配数据失败")?;
    let dir = open_dir(parent)?;
    atomic_write_at_async(dir, name, bytes)
        .await
        .context("原子写入端口分配数据失败")
}

static PORT_ALLOC_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

pub struct PortAllocator;

impl PortAllocator {
    pub async fn check_hysteria2_limit() -> Result<bool> {
        let conf_dir = PathBuf::from(singbox::CONF_DIR);
        if !fs::try_exists(&conf_dir).await? {
            return Ok(true);
        }
        let mut count = 0;
        let mut entries = fs::read_dir(&conf_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            if entry.file_type().await?.is_file()
                && name.to_string_lossy().ends_with(".json")
                && fs::read_to_string(entry.path())
                    .await?
                    .contains("hysteria2")
            {
                count += 1;
            }
        }
        Ok(count < 50)
    }

    async fn scan_all_occupied_ports(
        paths: &AllocatorPaths,
        data: &PortAllocData,
    ) -> Result<HashSet<u16>> {
        let mut occupied = HashSet::from([22, 80, 443]);
        occupied.extend(
            FirewallScanner::scan_dir_for_ports(&paths.xray_conf_dir)
                .await
                .context("扫描 Xray 配置端口失败")?,
        );

        if fs::try_exists(&paths.singbox_conf_dir).await? {
            let mut entries = fs::read_dir(&paths.singbox_conf_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if entry.file_type().await?.is_file()
                    && name.ends_with(".json")
                    && !name.starts_with("00_")
                {
                    let content = fs::read_to_string(entry.path()).await?;
                    occupied.extend(PortAllocator::extract_ports_from_json(&content)?);
                }
            }
        }

        for range in &data.locked_ranges {
            occupied.extend(range.start..=range.end);
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

    pub async fn get_locked_ranges() -> Result<Vec<(u16, u16)>> {
        Self::get_locked_ranges_with(&AllocatorPaths::production()).await
    }

    async fn get_locked_ranges_with(paths: &AllocatorPaths) -> Result<Vec<(u16, u16)>> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        Ok(load_port_alloc(&paths.state_file)
            .await?
            .locked_ranges
            .iter()
            .map(|range| (range.start, range.end))
            .collect())
    }

    pub async fn is_port_in_locked_range(port: u16) -> Result<bool> {
        Self::is_port_in_locked_range_with(&AllocatorPaths::production(), port).await
    }

    async fn is_port_in_locked_range_with(paths: &AllocatorPaths, port: u16) -> Result<bool> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        Ok(load_port_alloc(&paths.state_file)
            .await?
            .locked_ranges
            .iter()
            .any(|range| port >= range.start && port <= range.end))
    }

    pub async fn allocate_hysteria2() -> Result<(u16, (u16, u16))> {
        Self::allocate_hysteria2_with(&AllocatorPaths::production()).await
    }

    async fn allocate_hysteria2_with(paths: &AllocatorPaths) -> Result<(u16, (u16, u16))> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        let mut data = load_port_alloc(&paths.state_file).await?;
        let occupied = Self::scan_all_occupied_ports(paths, &data).await?;
        let main_port = Self::find_consecutive_range(&occupied, HOP_SIZE)?;
        let hop_end = main_port + HOP_SIZE - 1;
        #[cfg(test)]
        tokio::time::sleep(paths.after_load_delay).await;
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("系统时间早于 UNIX epoch")?
            .as_secs() as i64;
        data.locked_ranges.push(LockedRange {
            start: main_port,
            end: hop_end,
            protocol: "hysteria2".to_string(),
            created_at,
        });
        save_port_alloc(&paths.state_file, &data).await?;
        log::info!(
            "Hysteria2 端口分配: 主端口 {}, 跳跃范围 {}-{}",
            main_port,
            main_port + 1,
            hop_end
        );
        Ok((main_port, (main_port + 1, hop_end)))
    }

    pub async fn release_hysteria2_range(main_port: u16) -> Result<()> {
        Self::release_hysteria2_range_with(&AllocatorPaths::production(), main_port).await
    }

    async fn release_hysteria2_range_with(paths: &AllocatorPaths, main_port: u16) -> Result<()> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        let mut data = load_port_alloc(&paths.state_file).await?;
        let before = data.locked_ranges.len();
        data.locked_ranges
            .retain(|range| !(range.protocol == "hysteria2" && range.start == main_port));
        if data.locked_ranges.len() < before {
            save_port_alloc(&paths.state_file, &data).await?;
            log::info!("Hysteria2 端口范围已释放: 主端口 {}", main_port);
        } else {
            log::warn!("Hysteria2 端口范围未找到: 主端口 {}", main_port);
        }
        Ok(())
    }

    pub async fn get_hysteria2_range() -> Result<Option<(u16, (u16, u16))>> {
        Self::get_hysteria2_range_with(&AllocatorPaths::production()).await
    }

    async fn get_hysteria2_range_with(paths: &AllocatorPaths) -> Result<Option<(u16, (u16, u16))>> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        Ok(load_port_alloc(&paths.state_file)
            .await?
            .locked_ranges
            .iter()
            .find(|range| range.protocol == "hysteria2")
            .map(|range| (range.start, (range.start + 1, range.end))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet as StdHashSet;
    use std::fs as std_fs;
    use std::os::unix::fs::symlink;
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn test_paths(root: &Path) -> AllocatorPaths {
        let xray_conf_dir = root.join("xray");
        let singbox_conf_dir = root.join("singbox");
        std_fs::create_dir_all(&xray_conf_dir).unwrap();
        std_fs::create_dir_all(&singbox_conf_dir).unwrap();
        AllocatorPaths {
            state_file: root.join(".port_alloc"),
            lock_file: root.join(".port_alloc.lock"),
            xray_conf_dir,
            singbox_conf_dir,
            #[cfg(test)]
            after_load_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn corrupt_state_fails_closed_and_is_unchanged() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        std_fs::write(&paths.state_file, b"not-json").unwrap();

        let ranges = PortAllocator::get_locked_ranges_with(&paths).await;
        let contains = PortAllocator::is_port_in_locked_range_with(&paths, 10000).await;
        let hysteria2 = PortAllocator::get_hysteria2_range_with(&paths).await;
        let release = PortAllocator::release_hysteria2_range_with(&paths, 10000).await;

        assert!(ranges.is_err());
        assert!(contains.is_err());
        assert!(hysteria2.is_err());
        assert!(release.is_err());
        assert_eq!(std_fs::read(&paths.state_file).unwrap(), b"not-json");
    }

    #[tokio::test]
    async fn unreadable_state_fails_closed() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        std_fs::create_dir(&paths.state_file).unwrap();

        let result = PortAllocator::get_hysteria2_range_with(&paths).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn xray_scan_read_error_is_propagated() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        std_fs::write(paths.xray_conf_dir.join("broken.json"), [0xff]).unwrap();

        let result = PortAllocator::allocate_hysteria2_with(&paths).await;

        assert!(result.is_err());
        assert!(!paths.state_file.exists());
    }

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
        assert!(!(100..=111).contains(&start));
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
        data.locked_ranges
            .retain(|r| !(r.protocol == "hysteria2" && r.start == 10000));
        assert_eq!(data.locked_ranges.len(), 1);
        assert_eq!(data.locked_ranges[0].start, 20000);
    }

    #[test]
    fn test_locked_ranges_expand_to_occupied_ports() {
        let data = PortAllocData {
            locked_ranges: vec![
                LockedRange {
                    start: 30000,
                    end: 30000,
                    protocol: "hysteria2".to_string(),
                    created_at: 1234567890,
                },
                LockedRange {
                    start: 31000,
                    end: 31099,
                    protocol: "hysteria2".to_string(),
                    created_at: 1234567891,
                },
            ],
            initialized: true,
        };
        let mut occupied = HashSet::new();
        for range in &data.locked_ranges {
            for port in range.start..=range.end {
                occupied.insert(port);
            }
        }
        assert!(
            occupied.contains(&30000),
            "single port range must be included"
        );
        assert!(
            occupied.contains(&31000),
            "hop range start must be included"
        );
        assert!(occupied.contains(&31099), "hop range end must be included");
        assert!(
            !occupied.contains(&30999),
            "port before range must not be included"
        );
        assert_eq!(occupied.len(), 101);
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

    #[tokio::test]
    async fn allocation_persists_one_complete_json_document() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());

        let allocated = PortAllocator::allocate_hysteria2_with(&paths)
            .await
            .unwrap();
        let persisted: PortAllocData =
            serde_json::from_slice(&std_fs::read(&paths.state_file).unwrap()).unwrap();

        assert_eq!(persisted.locked_ranges.len(), 1);
        assert_eq!(persisted.locked_ranges[0].start, allocated.0);
        assert_eq!(persisted.locked_ranges[0].end, allocated.1.1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn persistence_failure_returns_no_allocation_and_preserves_old_state() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        let authoritative = temp.path().join("authoritative.json");
        let old = serde_json::to_vec(&PortAllocData::default()).unwrap();
        std_fs::write(&authoritative, &old).unwrap();
        symlink(&authoritative, &paths.state_file).unwrap();

        let result = PortAllocator::allocate_hysteria2_with(&paths).await;

        assert!(result.is_err());
        assert_eq!(std_fs::read(&authoritative).unwrap(), old);
        assert!(
            std_fs::symlink_metadata(&paths.state_file)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn concurrent_threads_allocate_distinct_ranges() {
        let temp = TempDir::new().unwrap();
        let mut paths = test_paths(temp.path());
        paths.after_load_delay = Duration::from_millis(100);
        let paths = Arc::new(paths);
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let paths = Arc::clone(&paths);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(PortAllocator::allocate_hysteria2_with(&paths))
                        .unwrap()
                        .0
                })
            })
            .collect();
        let ports: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();

        assert_eq!(ports.iter().copied().collect::<StdHashSet<_>>().len(), 8);
        let persisted: PortAllocData =
            serde_json::from_slice(&std_fs::read(&paths.state_file).unwrap()).unwrap();
        assert_eq!(persisted.locked_ranges.len(), 8);
    }
}
