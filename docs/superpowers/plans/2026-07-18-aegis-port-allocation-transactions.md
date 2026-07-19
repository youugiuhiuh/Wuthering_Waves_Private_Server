# Port Allocation Transactions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Hysteria2 port allocation a fail-closed transaction that serializes threads and Linux processes and atomically persists the complete allocation state.

**Architecture:** Keep the JSON file as source of truth. A private path context makes failure and multi-process tests deterministic while public methods retain their names; mutations and lookups take a process-local Tokio mutex followed by a bounded `flock(LOCK_EX | LOCK_NB)`, then read, scan, mutate, and persist under both guards. Reuse `secure_fs::open_dir` and `atomic_write_at_async` for same-directory replacement, file and directory synchronization.

**Tech Stack:** Rust 2024, Tokio, `std`, existing `libc`, `anyhow`, `serde_json`, existing `secure_fs`

**Files touched:**
- Modify: `rust/aegis/src/core/security/firewall_scanner.rs`
- Modify: `rust/aegis/src/core/xray/port_allocator.rs`
- Modify: `rust/aegis/src/core/xray/config.rs`
- Modify: `rust/aegis/src/core/xray/kcp.rs`
- Modify: `rust/aegis/src/core/singbox/hy2_batch.rs`
- Modify: `rust/aegis/src/core/singbox/tuic_batch.rs`
- Modify: `rust/aegis/src/core/singbox/config.rs`
- Test target: `rust/aegis/src/core/xray/port_allocator.rs` (`tests` module, including child-process helper)

## Global Constraints

- The allocation JSON remains authoritative until a complete replacement is synced and renamed.
- Corrupt, unreadable, unscannable, or unpersistable state aborts; no error becomes an empty default.
- The transaction holds both locks across state read, occupied-port scan, selection or release, persistence, and directory synchronization.
- Linux advisory lock acquisition is bounded at 2 seconds and reports the lock path and timeout.
- Public query signatures become `get_locked_ranges() -> Result<Vec<(u16, u16)>>`, `is_port_in_locked_range(u16) -> Result<bool>`, and `get_hysteria2_range() -> Result<Option<(u16, (u16, u16))>>`.
- Add no dependency; use existing `libc`, `std`, Tokio, and `secure_fs::{open_dir, atomic_write_at_async}`.
- Changes stay inside port allocation and its direct callers; socket reservation after allocation remains outside this phase.
- Every task follows RED, GREEN, REFACTOR and ends with its own commit.

---

### Task 1: Fail-Closed State, Scans, Queries, and Callers

**Files:**
- Modify: `rust/aegis/src/core/security/firewall_scanner.rs:165-184`
- Modify: `rust/aegis/src/core/xray/port_allocator.rs:1-227,230-360`
- Modify: `rust/aegis/src/core/xray/config.rs:296-327`
- Modify: `rust/aegis/src/core/xray/kcp.rs:114-126`
- Modify: `rust/aegis/src/core/singbox/hy2_batch.rs:48-59`
- Modify: `rust/aegis/src/core/singbox/tuic_batch.rs:44-57`
- Modify: `rust/aegis/src/core/singbox/config.rs:156-171,258-266`

**Interfaces:**
- Consumes: `FirewallScanner::scan_dir_for_ports<P: AsRef<Path>>(dir: P) -> Result<HashSet<u16>>`.
- Produces: private `AllocatorPaths::production()`, `load_port_alloc(&Path) -> Result<PortAllocData>`, and `scan_all_occupied_ports(&AllocatorPaths, &PortAllocData) -> Result<HashSet<u16>>`.
- Produces: public fallible query signatures listed in Global Constraints; allocation and release retain their current public signatures.

- [ ] **Step 1: Add deterministic failing tests for corrupt/unreadable state and scan errors**

Replace the `tests` module imports with the following imports and helper, then append the three tests:

```rust
mod tests {
    use super::*;
    use std::fs as std_fs;
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
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run from `rust/aegis`:

```bash
cargo test --lib core::xray::port_allocator::tests -- --test-threads=1
```

Expected: compilation fails while compiling the three new tests, with missing `AllocatorPaths`, `get_locked_ranges_with`, `get_hysteria2_range_with`, and `allocate_hysteria2_with`.

- [ ] **Step 3: Make `FirewallScanner::scan_dir_for_ports` propagate discovery and file-read errors**

Replace the function body at `rust/aegis/src/core/security/firewall_scanner.rs:166-183`:

```rust
    pub async fn scan_dir_for_ports<P: AsRef<Path>>(dir: P) -> Result<HashSet<u16>> {
        let mut ports = HashSet::new();
        let dir = dir.as_ref();
        if !fs::try_exists(dir).await? {
            return Ok(ports);
        }

        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
            {
                ports.extend(Self::extract_ports_from_file(&path).await?);
            }
        }
        Ok(ports)
    }
```

- [ ] **Step 4: Add path context and fail-closed load/scan helpers**

In `port_allocator.rs`, import `Path` and `Duration`, then replace constants/load/save with:

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;

const PORT_ALLOC_FILE: &str = "/etc/wwps/.port_alloc";
const PORT_ALLOC_LOCK_FILE: &str = "/etc/wwps/.port_alloc.lock";
const XRAY_PORT_MIN: u16 = 10000;
const XRAY_PORT_MAX: u16 = 60000;
const HOP_SIZE: u16 = 100;

#[derive(Clone)]
struct AllocatorPaths {
    state_file: PathBuf,
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

async fn load_port_alloc(path: &Path) -> Result<PortAllocData> {
    if !fs::try_exists(path).await? {
        return Ok(PortAllocData::default());
    }
    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("读取端口分配数据失败: {}", path.display()))?;
    serde_json::from_str(&content).context("解析端口分配数据失败")
}
```

Replace `save_port_alloc` and `scan_all_occupied_ports` for this first GREEN increment:

```rust
async fn save_port_alloc(path: &Path, data: &PortAllocData) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let content = serde_json::to_string_pretty(data)?;
    fs::write(path, content).await?;
    Ok(())
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
```

- [ ] **Step 5: Replace limit and locked-range queries with fallible variants**

Replace `check_hysteria2_limit`, `get_locked_ranges`, and `is_port_in_locked_range` with:

```rust
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
                && fs::read_to_string(entry.path()).await?.contains("hysteria2")
            {
                count += 1;
            }
        }
        Ok(count < 50)
    }

    pub async fn get_locked_ranges() -> Result<Vec<(u16, u16)>> {
        Self::get_locked_ranges_with(&AllocatorPaths::production()).await
    }

    async fn get_locked_ranges_with(paths: &AllocatorPaths) -> Result<Vec<(u16, u16)>> {
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
        Ok(Self::get_locked_ranges_with(paths)
            .await?
            .iter()
            .any(|(start, end)| port >= *start && port <= *end))
    }
```

- [ ] **Step 6: Replace allocation, release, and Hysteria2 lookup with fallible variants**

Replace `allocate_hysteria2`, `release_hysteria2_range`, and `get_hysteria2_range` with:

```rust

    pub async fn allocate_hysteria2() -> Result<(u16, (u16, u16))> {
        Self::allocate_hysteria2_with(&AllocatorPaths::production()).await
    }

    async fn allocate_hysteria2_with(paths: &AllocatorPaths) -> Result<(u16, (u16, u16))> {
        let mut data = load_port_alloc(&paths.state_file).await?;
        let occupied = scan_all_occupied_ports(paths, &data).await?;
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

    async fn release_hysteria2_range_with(
        paths: &AllocatorPaths,
        main_port: u16,
    ) -> Result<()> {
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

    async fn get_hysteria2_range_with(
        paths: &AllocatorPaths,
    ) -> Result<Option<(u16, (u16, u16))>> {
        Ok(load_port_alloc(&paths.state_file)
            .await?
            .locked_ranges
            .iter()
            .find(|range| range.protocol == "hysteria2")
            .map(|range| (range.start, (range.start + 1, range.end))))
    }
```

- [ ] **Step 7: Propagate the new query results through every caller**

In `xray/config.rs` at lines 302 and 317 and in `xray/kcp.rs` at line 117, retain the fully qualified type and add `?`:

```rust
if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await? {
```

In `singbox/hy2_batch.rs` at line 53, use its existing import:

```rust
if PortAllocator::is_port_in_locked_range(p).await? {
```

In `singbox/tuic_batch.rs` at line 49, retain the fully qualified type:

```rust
if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await? {
```

- [ ] **Step 8: Propagate release and lookup errors in the sing-box manager**

In `singbox/config.rs`, replace both swallowed releases with:

```rust
PortAllocator::release_hysteria2_range(main_port).await?;
```

Replace the lookup at line 265 with:

```rust
if let Some((main_port, hop_range)) = PortAllocator::get_hysteria2_range().await? {
```

- [ ] **Step 9: Run focused tests and caller type-check; verify GREEN**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests -- --test-threads=1
cargo check --lib
```

Expected: all port allocator unit tests pass; `cargo check` exits 0 with no mismatched `Result` errors.

- [ ] **Step 10: REFACTOR and commit the fail-closed boundary**

Run `cargo fmt`, rerun the commands in Step 9, then commit:

```bash
git add src/core/security/firewall_scanner.rs src/core/xray/port_allocator.rs src/core/xray/config.rs src/core/xray/kcp.rs src/core/singbox/hy2_batch.rs src/core/singbox/tuic_batch.rs src/core/singbox/config.rs
git commit -m "fix: propagate port allocator state errors"
```

---
### Task 2: Atomically Persist the Complete State

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs` (`save_port_alloc`, tests)

**Interfaces:**
- Consumes: `open_dir(path: &Path) -> Result<std::fs::File>` and `atomic_write_at_async(dir: File, name: OsString, bytes: Vec<u8>) -> Result<()>` from `core/security/secure_fs.rs`.
- Produces: `save_port_alloc(path: &Path, data: &PortAllocData) -> Result<()>` with same-directory atomic replacement and file/directory sync.

- [ ] **Step 1: Add failing success/failure persistence tests**

Append to `port_allocator.rs` tests:

```rust
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
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        let authoritative = temp.path().join("authoritative.json");
        let old = serde_json::to_vec(&PortAllocData::default()).unwrap();
        std_fs::write(&authoritative, &old).unwrap();
        symlink(&authoritative, &paths.state_file).unwrap();

        let result = PortAllocator::allocate_hysteria2_with(&paths).await;

        assert!(result.is_err());
        assert_eq!(std_fs::read(&authoritative).unwrap(), old);
        assert!(std_fs::symlink_metadata(&paths.state_file)
            .unwrap()
            .file_type()
            .is_symlink());
    }
```

- [ ] **Step 2: Run the persistence failure test and verify RED**

Run from `rust/aegis`:

```bash
cargo test --lib core::xray::port_allocator::tests::persistence_failure_returns_no_allocation_and_preserves_old_state -- --exact --nocapture
```

Expected: FAIL because direct `tokio::fs::write` follows the symlink, returns success, and changes `authoritative.json`.

- [ ] **Step 3: Replace direct save with the existing secure atomic primitive**

Add imports:

```rust
use crate::core::security::secure_fs::{atomic_write_at_async, open_dir};
```

Replace `save_port_alloc`:

```rust
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
```

- [ ] **Step 4: Run both persistence tests and verify GREEN**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests::allocation_persists_one_complete_json_document -- --exact
cargo test --lib core::xray::port_allocator::tests::persistence_failure_returns_no_allocation_and_preserves_old_state -- --exact
```

Expected: both tests PASS; the failure test returns no allocation and leaves old bytes authoritative.

- [ ] **Step 5: REFACTOR and commit atomic persistence**

Run `cargo fmt` and `cargo test --lib core::xray::port_allocator::tests -- --test-threads=1`, then commit from `rust/aegis`:

```bash
git add src/core/xray/port_allocator.rs
git commit -m "fix: atomically persist port allocation state"
```

---

### Task 3: Serialize Transactions Across In-Process Threads

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs` (static mutex, all `_with` operations, tests)

**Interfaces:**
- Produces: `static PORT_ALLOC_MUTEX: LazyLock<tokio::sync::Mutex<()>>`.
- Preserves: all public signatures produced by Task 1.

- [ ] **Step 1: Add a failing concurrent-thread allocation test**

Add imports to the tests module and append the test:

```rust
    use std::collections::HashSet as StdHashSet;
    use std::sync::{Arc, Barrier};

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
```

- [ ] **Step 2: Run the thread test and verify RED**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests::concurrent_threads_allocate_distinct_ranges -- --exact --nocapture
```

Expected: FAIL because concurrent threads read the same state version and return duplicate port `10000` and/or persist fewer than eight ranges.

- [ ] **Step 3: Add the process-local mutex and hold it across each complete operation**

Add imports and static:

```rust
use std::sync::LazyLock;

static PORT_ALLOC_MUTEX: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
```

Add this as the first statement in each of `get_locked_ranges_with`, `allocate_hysteria2_with`, `release_hysteria2_range_with`, and `get_hysteria2_range_with`:

```rust
let _process_guard = PORT_ALLOC_MUTEX.lock().await;
```

Replace `is_port_in_locked_range_with` to avoid recursively taking the non-reentrant mutex:

```rust
    async fn is_port_in_locked_range_with(paths: &AllocatorPaths, port: u16) -> Result<bool> {
        let _process_guard = PORT_ALLOC_MUTEX.lock().await;
        Ok(load_port_alloc(&paths.state_file)
            .await?
            .locked_ranges
            .iter()
            .any(|range| port >= range.start && port <= range.end))
    }
```

- [ ] **Step 4: Run the thread and failure tests and verify GREEN**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests::concurrent_threads_allocate_distinct_ranges -- --exact --nocapture
cargo test --lib core::xray::port_allocator::tests -- --test-threads=1
```

Expected: PASS; eight distinct ranges are persisted and all fail-closed tests remain green.

- [ ] **Step 5: REFACTOR and commit process-local serialization**

Run `cargo fmt` and the commands in Step 4, then commit:

```bash
git add src/core/xray/port_allocator.rs
git commit -m "fix: serialize in-process port allocation transactions"
```

---

### Task 4: Add Bounded Linux Advisory Locking and Cross-Process Proof

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs` (lock helper, all `_with` operations, tests)

**Interfaces:**
- Produces: `acquire_advisory_lock(path: &Path, timeout: Duration) -> Result<std::fs::File>`; dropping the returned file releases `flock`.
- Produces: `LOCK_TIMEOUT = Duration::from_secs(2)` and `LOCK_RETRY = Duration::from_millis(25)`.
- Preserves: mutex-before-file-lock acquisition order for every operation.

- [ ] **Step 1: Add the bounded-lock failure test**

Add test imports and append:

```rust
    use std::fs::OpenOptions as StdOpenOptions;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::os::unix::io::AsRawFd as _;

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn advisory_lock_timeout_is_bounded_and_actionable() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        let holder = StdOpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(&paths.lock_file)
            .unwrap();
        // SAFETY: `holder` owns a valid file descriptor for the duration of the call.
        assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);

        let error = acquire_advisory_lock(&paths.lock_file, Duration::from_millis(50))
            .await
            .unwrap_err();

        let message = error.to_string();
        assert!(message.contains(paths.lock_file.to_str().unwrap()));
        assert!(message.contains("50ms"));
    }
```

- [ ] **Step 2: Add an ignored child helper and two-process allocation test**

Append to the tests module:

```rust
    const PROCESS_ROOT_ENV: &str = "AEGIS_PORT_ALLOC_PROCESS_TEST_ROOT";
    const PROCESS_HELPER_NAME: &str =
        "core::xray::port_allocator::tests::process_allocation_helper";

    #[tokio::test]
    #[ignore]
    async fn process_allocation_helper() {
        let root = PathBuf::from(std::env::var(PROCESS_ROOT_ENV).unwrap());
        let mut paths = test_paths(&root);
        paths.after_load_delay = Duration::from_millis(200);
        PortAllocator::allocate_hysteria2_with(&paths)
            .await
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn separate_processes_allocate_distinct_ranges() {
        let temp = TempDir::new().unwrap();
        let paths = test_paths(temp.path());
        let test_binary = std::env::current_exe().unwrap();
        let spawn = || {
            std::process::Command::new(&test_binary)
                .arg(PROCESS_HELPER_NAME)
                .arg("--exact")
                .arg("--ignored")
                .arg("--nocapture")
                .env(PROCESS_ROOT_ENV, temp.path())
                .spawn()
                .unwrap()
        };
        let first = spawn();
        let second = spawn();
        let first = first.wait_with_output().unwrap();
        let second = second.wait_with_output().unwrap();

        assert!(
            first.status.success(),
            "first child failed: {}",
            String::from_utf8_lossy(&first.stderr)
        );
        assert!(
            second.status.success(),
            "second child failed: {}",
            String::from_utf8_lossy(&second.stderr)
        );
        let persisted: PortAllocData =
            serde_json::from_slice(&std_fs::read(&paths.state_file).unwrap()).unwrap();
        let starts: StdHashSet<_> = persisted
            .locked_ranges
            .iter()
            .map(|range| range.start)
            .collect();
        assert_eq!(persisted.locked_ranges.len(), 2);
        assert_eq!(starts.len(), 2);
    }
```

- [ ] **Step 3: Run both new tests and verify RED**

Run from `rust/aegis`:

```bash
cargo test --lib core::xray::port_allocator::tests::advisory_lock_timeout_is_bounded_and_actionable -- --exact --nocapture
cargo test --lib core::xray::port_allocator::tests::separate_processes_allocate_distinct_ranges -- --exact --nocapture
```

Expected: both commands fail compilation with `cannot find function acquire_advisory_lock`; no lock implementation exists yet.

- [ ] **Step 4: Implement nonblocking `flock` retry with a 2-second bound**

Add imports/constants and the helper before `PortAllocator`:

```rust
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::time::Instant;

const LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const LOCK_RETRY: Duration = Duration::from_millis(25);

#[allow(clippy::undocumented_unsafe_blocks)]
async fn acquire_advisory_lock(path: &Path, timeout: Duration) -> Result<File> {
    let parent = path.parent().context("端口分配锁文件没有父目录")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("创建端口分配锁目录失败: {}", parent.display()))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("打开端口分配锁失败: {}", path.display()))?;
    let deadline = Instant::now() + timeout;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        let would_block = matches!(
            error.raw_os_error(),
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
        );
        if !would_block {
            return Err(error).with_context(|| {
                format!("获取端口分配锁失败: {}", path.display())
            });
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "等待端口分配锁超时: {} ({timeout:?})",
                path.display()
            );
        }
        tokio::time::sleep(LOCK_RETRY).await;
    }
}
```

- [ ] **Step 5: Hold the file lock after the mutex in every operation**

Immediately after `let _process_guard = PORT_ALLOC_MUTEX.lock().await;` in all five `_with` operation methods, add:

```rust
let _file_guard = acquire_advisory_lock(&paths.lock_file, LOCK_TIMEOUT).await?;
```

The five methods are `get_locked_ranges_with`, `is_port_in_locked_range_with`, `allocate_hysteria2_with`, `release_hysteria2_range_with`, and `get_hysteria2_range_with`. Do not release either guard before the method returns.

- [ ] **Step 6: Run the bounded-lock test and verify GREEN**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests::advisory_lock_timeout_is_bounded_and_actionable -- --exact --nocapture
```

Expected: PASS in under one second; error text contains the exact lock path and `50ms`.

- [ ] **Step 7: Run the process test with locking and verify GREEN**

Run:

```bash
cargo test --lib core::xray::port_allocator::tests::separate_processes_allocate_distinct_ranges -- --exact --nocapture
```

Expected: PASS; the state JSON contains two ranges with distinct starts.

- [ ] **Step 8: REFACTOR and run all mandatory Rust gates**

Keep the private path context and process delay under test-only use; do not add a production environment override. Run from `rust/aegis`:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test -- --test-threads=1
```

Expected: all commands exit 0; the child helper is reported ignored during the full suite, while the parent process test passes.

- [ ] **Step 9: Commit cross-process transactions**

```bash
git add src/core/xray/port_allocator.rs
git commit -m "fix: lock port allocation across processes"
```

---

## Coverage Matrix

| Requirement | Proof |
| --- | --- |
| In-process mutual exclusion | Task 3 `concurrent_threads_allocate_distinct_ranges` |
| Linux bounded advisory lock | Task 4 `advisory_lock_timeout_is_bounded_and_actionable` |
| Atomic complete-state write | Task 2 success test plus existing `secure_fs::atomic_write_at_async` sync/rename behavior |
| Corrupt state fails closed | Task 1 `corrupt_state_fails_closed_and_is_unchanged` |
| Unreadable state fails closed | Task 1 `unreadable_state_fails_closed` |
| Persistence failure fails closed and preserves old state | Task 2 symlink-injected failure test |
| Scan errors propagate | Task 1 invalid UTF-8 Xray config test and strict scanner implementation |
| Concurrent threads allocate distinct ranges | Task 3 eight-thread test |
| Independent processes allocate distinct ranges | Task 4 child-test-binary integration test |
| Release and lookup remove `unwrap_or_default` | Task 1 fallible methods and `?` propagation through all direct callers |
| No new dependency | All tasks use existing `libc`, `std`, Tokio, and `secure_fs` |

## Residual Risk

The lock prevents allocator-state races but does not reserve sockets. A different program can bind a selected port after commit; existing consumers must continue handling bind or firewall failure.
