# Sing-box Hysteria2 端口跳跃删除后端口不释放修复计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 hysteria2 端口跳跃（hopping）配置被删除后，`.port_alloc` 中的锁定端口范围未释放，导致重新创建时无法还原/复用旧端口的问题。

**Architecture:** 删除路径（`delete_all_configurations` / `delete_by_count` / `delete_specific_configuration`）统一通过新的 `cleanup_hysteria2_config(path, has_ipv6)` 辅助函数清理每个 hysteria2 配置：按 inbound `type == "hysteria2"` 内容检测提取**全部**主端口（不再依赖文件名、不再只取第一个 inbound），逐个删除 iptables/ip6tables 规则、移除维护端口范围、调用 `PortAllocator::release_hysteria2_range()` 释放锁定范围。`PortAllocator` 增加 `_at(path)` 内部变体（公开 API 不变），使"释放后可复用端口"逻辑可测试。

**Tech Stack:** Rust (edition 2024), tokio, serde_json, cargo nextest, tempfile (已有 dev-dependency)。

**Spec:** 根因分析见本计划下方「根因分析」节（normal 模式 bugfix，无独立 spec 文件）。

## 根因分析（服务器 102.134.50.23 实证）

- 服务器 `.port_alloc` 有 17 个 hysteria2 锁定范围，但只剩 2 个配置文件和 6 条 iptables 规则（2 批 × 3 inbound）。
- 10000-11022 共 11 个范围对应的配置已被删除、iptables 规则已被清（重启后不持久），但 `.port_alloc` 记录永久残留。
- 代码根因：
  1. `delete_all_configurations()` 从不调用 `PortAllocator::release_hysteria2_range()`。
  2. `delete_by_count()` 只删文件，防火墙规则、维护端口范围、端口分配全部不清理。
  3. `extract_main_port_from_config()` 只提取 `inbounds[0]` 的端口，但一个批量文件含 count 个 inbound → 多 inbound 文件删除时漏释放 count-1 个范围。
  4. 文件名判断 `file.contains("hysteria2") || file.contains("hysteria")` 匹配不到 `batch_hy2_*.json`（文件名是 `hy2` 而非 `hysteria`）→ `delete_all_configurations()` 的规则清理对 hy2 批量文件实际不生效。
- 分配器 `scan_all_occupied_ports()` 把 `.port_alloc` 锁定范围视为占用 → 再次创建永远跳过旧端口、分配更高端口 → "删除再创建不会还原端口"，且 `.port_alloc` 无限膨胀直至 `find_consecutive_range` 失败。

## Global Constraints

- **公开 API 不变**：`PortAllocator::allocate_hysteria2()` / `release_hysteria2_range()` / `get_hysteria2_range()` / `is_port_in_locked_range()` 签名与行为保持兼容；新增 `_at` 变体仅 `pub(crate)` 或私有（测试同文件可访问）。
- **按内容检测 hysteria2**：不再用文件名判断；统一用 `inbound["type"] == "hysteria2"`。
- **最佳努力清理**：删除路径中清理失败（iptables 无 root、解析失败等）不阻塞文件删除；`delete_specific_configuration` 对非 hysteria2 文件保持原有行为（直接删除 + reload）。
- **i18n**：无新增用户可见文案（仅内部 log）。
- **依赖**：不新增/删除依赖（tempfile 已在 dev-dependencies）。
- **Quality gate**（rust-lint-format），从 `rust/aegis` 执行：
  `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`
- Rust skill 规则：`err-no-unwrap-prod`、`test-arrange-act-assert`、`test-descriptive-names`。

---

### Task 1: PortAllocator 路径化重构 + 释放复用回归测试（TDD）

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs`

**Interfaces:**
- Consumes: 无（现有 `PORT_ALLOC_FILE` const）。
- Produces:
  - 私有 `async fn load_port_alloc_at(path: &Path) -> Result<PortAllocData>` / `save_port_alloc_at(path: &Path, data: &PortAllocData) -> Result<()>`（现有 `load_port_alloc` / `save_port_alloc` 改为包装，走默认路径）。
  - 私有 `async fn scan_all_occupied_ports(path: &Path) -> Result<HashSet<u16>>`（锁定范围部分改用 `load_port_alloc_at(path)`；xray/singbox 真实目录扫描不变）。
  - `pub async fn allocate_hysteria2()` 改为包装 `async fn allocate_hysteria2_at(path: &Path) -> Result<(u16, (u16, u16))>`（`pub(crate)` 不可见，测试同文件可访问）。
  - `pub async fn release_hysteria2_range(main_port: u16)` 改为包装 `async fn release_hysteria2_range_at(path: &Path, main_port: u16) -> Result<()>`。
  - `use std::path::Path;` 新增 import。

- [ ] **Step 1: 写失败测试（回归测试，锁定"释放→复用"契约）**

在 `port_allocator.rs` 的 `#[cfg(test)] mod tests` 中添加：

```rust
use std::path::PathBuf;

#[tokio::test]
async fn test_release_then_allocate_restores_port() {
    // 核心回归：删除释放后，重新创建应还原同一端口范围
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");

    let (main_port, hop) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();
    assert_eq!(main_port, 10000, "首个空闲范围应分配 10000");
    assert_eq!(hop, (10001, 10099));

    PortAllocator::release_hysteria2_range_at(&alloc_path, main_port)
        .await
        .unwrap();

    let (main_port2, hop2) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();
    assert_eq!(
        main_port2, main_port,
        "删除释放后重新创建应还原同一端口范围"
    );
    assert_eq!(hop2, hop);
}

#[tokio::test]
async fn test_unreleased_range_is_not_reused() {
    // 机制说明：未释放的锁定范围会被新分配跳过（这正是 bug 的表象）
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");

    let (main_port, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();
    assert_eq!(main_port, 10000);

    // 不调用 release，模拟"配置已删但范围未释放"
    let (main_port2, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();
    assert_eq!(main_port2, 10100, "未释放的范围应被跳过");
}

#[tokio::test]
async fn test_release_removes_range_from_file() {
    // 文件往返：释放后文件里不再有该范围
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");

    let (main_port, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();

    PortAllocator::release_hysteria2_range_at(&alloc_path, main_port)
        .await
        .unwrap();

    let data = load_port_alloc_at(&alloc_path).await.unwrap();
    assert!(
        data.locked_ranges
            .iter()
            .all(|r| r.start != main_port),
        "释放后文件中不应再包含该范围"
    );
    assert_eq!(data.locked_ranges.len(), 0);
}
```

注意：测试模块内已有 `use super::*;`，`load_port_alloc_at` / `allocate_hysteria2_at` / `release_hysteria2_range_at` 均可用。

- [ ] **Step 2: 运行测试确认失败**

Run（在 `rust/aegis` 目录）：`cargo nextest run -p aegis --lib port_allocator --no-fail-fast 2>&1 | tail -20`（若 nextest 不可用则 `cargo test -p aegis --lib port_allocator`）
Expected: 编译失败，报 `allocate_hysteria2_at` / `release_hysteria2_range_at` / `load_port_alloc_at` 未定义（RED）。

- [ ] **Step 3: 实现路径化重构**

修改 `rust/aegis/src/core/xray/port_allocator.rs`：

```rust
use std::path::Path; // 新增

async fn load_port_alloc() -> Result<PortAllocData> {
    load_port_alloc_at(&PathBuf::from(PORT_ALLOC_FILE)).await
}

async fn load_port_alloc_at(path: &Path) -> Result<PortAllocData> {
    if !path.exists() {
        return Ok(PortAllocData::default());
    }
    let content = fs::read_to_string(path).await?;
    let data: PortAllocData = serde_json::from_str(&content).context("解析端口分配数据失败")?;
    Ok(data)
}

async fn save_port_alloc(data: &PortAllocData) -> Result<()> {
    save_port_alloc_at(&PathBuf::from(PORT_ALLOC_FILE), data).await
}

async fn save_port_alloc_at(path: &Path, data: &PortAllocData) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let content = serde_json::to_string_pretty(data)?;
    fs::write(path, content).await?;
    Ok(())
}
```

`scan_all_occupied_ports` 增加路径参数，锁定范围部分使用 `load_port_alloc_at(path)`：

```rust
async fn scan_all_occupied_ports(path: &Path) -> Result<HashSet<u16>> {
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

    if let Ok(data) = load_port_alloc_at(path).await {
        for range in &data.locked_ranges {
            for port in range.start..=range.end {
                occupied.insert(port);
            }
        }
    }

    Ok(occupied)
}
```

`allocate_hysteria2` 改为包装 `allocate_hysteria2_at`：

```rust
pub async fn allocate_hysteria2() -> Result<(u16, (u16, u16))> {
    Self::allocate_hysteria2_at(&PathBuf::from(PORT_ALLOC_FILE)).await
}

async fn allocate_hysteria2_at(path: &Path) -> Result<(u16, (u16, u16))> {
    let occupied = Self::scan_all_occupied_ports(path).await?;
    let main_port = Self::find_consecutive_range(&occupied, HOP_SIZE)?;
    let hop_end = main_port + 99;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut data = load_port_alloc_at(path).await.unwrap_or_default();
    data.locked_ranges.push(LockedRange {
        start: main_port,
        end: hop_end,
        protocol: "hysteria2".to_string(),
        created_at: now,
    });
    save_port_alloc_at(path, &data).await?;

    log::info!(
        "Hysteria2 端口分配: 主端口 {}, 跳跃范围 {}-{}",
        main_port,
        main_port + 1,
        hop_end
    );

    Ok((main_port, (main_port + 1, hop_end)))
}
```

`release_hysteria2_range` 改为包装 `release_hysteria2_range_at`：

```rust
pub async fn release_hysteria2_range(main_port: u16) -> Result<()> {
    Self::release_hysteria2_range_at(&PathBuf::from(PORT_ALLOC_FILE), main_port).await
}

async fn release_hysteria2_range_at(path: &Path, main_port: u16) -> Result<()> {
    let mut data = load_port_alloc_at(path).await.unwrap_or_default();
    let before = data.locked_ranges.len();
    data.locked_ranges
        .retain(|r| !(r.protocol == "hysteria2" && r.start == main_port));

    if data.locked_ranges.len() < before {
        save_port_alloc_at(path, &data).await?;
        log::info!("Hysteria2 端口范围已释放: 主端口 {}", main_port);
    } else {
        log::warn!(
            "Hysteria2 端口范围未找到: 主端口 {} (可能已被释放)",
            main_port
        );
    }
    Ok(())
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo nextest run -p aegis --lib port_allocator 2>&1 | tail -20`
Expected: 全部 PASS（新增 3 个 + 原有单元测试）。

- [ ] **Step 5: 提交**

```bash
git add rust/aegis/src/core/xray/port_allocator.rs
git commit -m "refactor(port-alloc): 增加路径化 _at 变体并测试释放后端口复用"
```

---

### Task 2: 配置解析改为内容检测 + 提取全部 hysteria2 主端口（TDD）

**Files:**
- Modify: `rust/aegis/src/core/singbox/config.rs`

**Interfaces:**
- Consumes: 无（仅 `serde_json`、`tokio::fs` 已有）。
- Produces:
  - 私有 `async fn extract_hysteria2_ports_from_config(path: &str) -> Result<Vec<u16>>` — 读取文件，收集所有 `type == "hysteria2"` inbound 的 `listen_port`；无则返回 Err("配置中未找到 hysteria2 inbound 主端口")。
  - 删除旧的 `extract_main_port_from_config`（Task 3 替换所有调用点）。

- [ ] **Step 1: 写失败测试**

在 `config.rs` 的 `#[cfg(test)] mod port_collection_tests` 中追加：

```rust
#[tokio::test]
async fn test_extract_hysteria2_ports_all_inbounds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch_hy2_test.json");
    let json = serde_json::json!({
        "inbounds": [
            {"type": "hysteria2", "listen_port": 11123},
            {"type": "hysteria2", "listen_port": 11223},
            {"type": "tuic", "listen_port": 30001},
            {"type": "hysteria2", "listen_port": 11323}
        ]
    });
    tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
        .await
        .unwrap();

    let ports = SingBoxConfigManager::extract_hysteria2_ports_from_config(
        path.to_str().unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(
        ports,
        vec![11123u16, 11223, 11323],
        "应提取所有 hysteria2 inbound 的主端口，跳过非 hysteria2 inbound"
    );
}

#[tokio::test]
async fn test_extract_hysteria2_ports_none_returns_err() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch_tuic_test.json");
    let json = serde_json::json!({
        "inbounds": [
            {"type": "tuic", "listen_port": 30001}
        ]
    });
    tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
        .await
        .unwrap();

    let result =
        SingBoxConfigManager::extract_hysteria2_ports_from_config(path.to_str().unwrap()).await;
    assert!(result.is_err(), "无 hysteria2 inbound 时应返回 Err");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo nextest run -p aegis --lib extract_hysteria2_ports --no-fail-fast 2>&1 | tail -10`
Expected: 编译失败，`extract_hysteria2_ports_from_config` 未定义（RED）。

- [ ] **Step 3: 实现函数并删除旧函数**

在 `config.rs` 中，用以下实现**替换**旧的 `extract_main_port_from_config`：

```rust
/// 提取配置文件中所有 hysteria2 inbound 的主端口（按内容检测，跳过其他协议）
async fn extract_hysteria2_ports_from_config(path: &str) -> Result<Vec<u16>> {
    let content = fs::read_to_string(path).await?;
    let json: Value = serde_json::from_str(&content)?;

    let mut ports = Vec::new();
    if let Some(inbounds) = json["inbounds"].as_array() {
        for inbound in inbounds {
            if inbound.get("type").and_then(|v| v.as_str()) != Some("hysteria2") {
                continue;
            }
            if let Some(p) = inbound["listen_port"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
            {
                ports.push(p);
            }
        }
    }
    if ports.is_empty() {
        return Err(anyhow::anyhow!("配置中未找到 hysteria2 inbound 主端口"));
    }
    Ok(ports)
}
```

删除旧 `extract_main_port_from_config` 的整个函数体（Task 3 中调用点全部替换）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo nextest run -p aegis --lib extract_hysteria2_ports 2>&1 | tail -10`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add rust/aegis/src/core/singbox/config.rs
git commit -m "feat(singbox): 按内容提取全部 hysteria2 主端口"
```

---

### Task 3: 三个删除路径统一调用释放逻辑

**Files:**
- Modify: `rust/aegis/src/core/singbox/config.rs`

**Interfaces:**
- Consumes: `extract_hysteria2_ports_from_config`（Task 2）、`PortAllocator::release_hysteria2_range`、`SystemMonitor::get_public_ipv6`。
- Produces:
  - 私有 `async fn cleanup_hysteria2_config(path: &str, has_ipv6: bool) -> Result<()>` — 对每个 hysteria2 主端口：`cleanup_specific_hysteria2_rules(main_port, (main_port+1, main_port+99), has_ipv6)` + `PortAllocator::release_hysteria2_range(main_port)`（均为 `let _` 最佳努力）。
  - `cleanup_specific_hysteria2_rules` 签名增加 `has_ipv6: bool` 参数（去掉函数内部网络探测）。

- [ ] **Step 1: 修改 `cleanup_specific_hysteria2_rules` 签名**

将：

```rust
async fn cleanup_specific_hysteria2_rules(main_port: u16, hop_range: (u16, u16)) -> Result<()> {
    use tokio::process::Command;

    let range_str = format!("{}:{}", hop_range.0, hop_range.1);
    let _ = Command::new("iptables")
        .args([... "-D" ...])
        .output()
        .await;

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
    if has_ipv6 {
        ...
    }
    ...
}
```

改为（仅签名与 IPv6 探测处变动）：

```rust
async fn cleanup_specific_hysteria2_rules(
    main_port: u16,
    hop_range: (u16, u16),
    has_ipv6: bool,
) -> Result<()> {
    use tokio::process::Command;

    let range_str = format!("{}:{}", hop_range.0, hop_range.1);
    let _ = Command::new("iptables")
        .args([
            "-t",
            "nat",
            "-D",
            "PREROUTING",
            "-p",
            "udp",
            "--dport",
            &range_str,
            "-j",
            "REDIRECT",
            "--to-ports",
            &main_port.to_string(),
        ])
        .output()
        .await;

    if has_ipv6 {
        let _ = Command::new("ip6tables")
            .args([
                "-t",
                "nat",
                "-D",
                "PREROUTING",
                "-p",
                "udp",
                "--dport",
                &range_str,
                "-j",
                "REDIRECT",
                "--to-ports",
                &main_port.to_string(),
            ])
            .output()
            .await;

        let _ = MaintenanceManager::remove_port_range_v6(hop_range.0, hop_range.1).await;
    }

    let _ = MaintenanceManager::remove_port_range(main_port, main_port).await;
    let _ = MaintenanceManager::remove_port_range(hop_range.0, hop_range.1).await;

    log::info!(
        "已清理 Hysteria2 端口跳跃规则: 主端口 {}, 范围 {}",
        main_port,
        range_str
    );
    Ok(())
}
```

- [ ] **Step 2: 新增 `cleanup_hysteria2_config` 辅助函数**

在 `cleanup_specific_hysteria2_rules` 之后新增：

```rust
/// 清理单个 hysteria2 配置文件：提取全部主端口，逐端口清理防火墙规则并释放端口分配
async fn cleanup_hysteria2_config(path: &str, has_ipv6: bool) -> Result<()> {
    let ports = Self::extract_hysteria2_ports_from_config(path).await?;
    for main_port in ports {
        let hop_range = (main_port + 1, main_port + 99);
        let _ = Self::cleanup_specific_hysteria2_rules(main_port, hop_range, has_ipv6).await;
        let _ = PortAllocator::release_hysteria2_range(main_port).await;
    }
    Ok(())
}
```

- [ ] **Step 3: 重写 `delete_all_configurations`**

将：

```rust
pub async fn delete_all_configurations() -> Result<usize> {
    let files = Self::list_all_inbound_files().await?;
    let count = files.len();

    for file in &files {
        if (file.contains("hysteria2") || file.contains("hysteria"))
            && let Ok(main_port) = Self::extract_main_port_from_config(file).await
        {
            let hop_range = (main_port + 1, main_port + 99);
            Self::cleanup_specific_hysteria2_rules(main_port, hop_range).await?;
        }

        let _ = fs::remove_file(file).await;
    }

    if count > 0 {
        Self::reload_service().await?;
    }
    Ok(count)
}
```

替换为：

```rust
pub async fn delete_all_configurations() -> Result<usize> {
    let files = Self::list_all_inbound_files().await?;
    let count = files.len();
    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    for file in &files {
        // 按内容检测 hysteria2；非 hysteria2 文件 extract 返回 Err，被 let _ 吞掉不影响删除
        let _ = Self::cleanup_hysteria2_config(file, has_ipv6).await;
        let _ = fs::remove_file(file).await;
    }

    if count > 0 {
        Self::reload_service().await?;
    }
    Ok(count)
}
```

- [ ] **Step 4: 重写 `delete_by_count`**

将：

```rust
pub async fn delete_by_count(count: usize) -> Result<usize> {
    let files = Self::list_all_inbound_files().await?;

    if files.is_empty() {
        return Ok(0);
    }

    let mut sorted_files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    for file in &files {
        let path = std::path::PathBuf::from(file);
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && let Ok(modified) = metadata.modified()
        {
            sorted_files.push((path, modified));
        }
    }

    sorted_files.sort_by_key(|a| a.1);

    let delete_count = count.min(sorted_files.len());
    let mut deleted = 0;

    for (path, _) in sorted_files.iter().take(delete_count) {
        if fs::remove_file(path).await.is_ok() {
            deleted += 1;
        }
    }

    if deleted > 0 {
        Self::reload_service().await?;
    }

    Ok(deleted)
}
```

替换为（删除每个文件前先清理）：

```rust
pub async fn delete_by_count(count: usize) -> Result<usize> {
    let files = Self::list_all_inbound_files().await?;

    if files.is_empty() {
        return Ok(0);
    }

    let mut sorted_files: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    for file in &files {
        let path = std::path::PathBuf::from(file);
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && let Ok(modified) = metadata.modified()
        {
            sorted_files.push((path, modified));
        }
    }

    sorted_files.sort_by_key(|a| a.1);

    let delete_count = count.min(sorted_files.len());
    let mut deleted = 0;
    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();

    for (path, _) in sorted_files.iter().take(delete_count) {
        let path_str = path.to_string_lossy().to_string();
        let _ = Self::cleanup_hysteria2_config(&path_str, has_ipv6).await;
        if fs::remove_file(path).await.is_ok() {
            deleted += 1;
        }
    }

    if deleted > 0 {
        Self::reload_service().await?;
    }

    Ok(deleted)
}
```

- [ ] **Step 5: 重写 `delete_specific_configuration`**

将：

```rust
pub async fn delete_specific_configuration(path: &str) -> Result<()> {
    let main_port = Self::extract_main_port_from_config(path).await?;
    let hop_range = (main_port + 1, main_port + 99);

    fs::remove_file(path).await.context("删除配置文件失败")?;

    Self::cleanup_specific_hysteria2_rules(main_port, hop_range).await?;

    let _ = PortAllocator::release_hysteria2_range(main_port).await;

    let remaining = Self::list_all_inbound_files().await?;
    let has_hysteria2 = remaining.iter().any(|f| f.contains("hysteria2"));

    if !has_hysteria2 {
        let _ = PortAllocator::release_hysteria2_range(main_port).await;
    }

    Self::reload_service().await?;
    Ok(())
}
```

替换为（先清理全部 hysteria2 端口再删文件；移除冗余的二次 release；非 hysteria2 文件保持原删除行为）：

```rust
pub async fn delete_specific_configuration(path: &str) -> Result<()> {
    // 按内容检测并清理 hysteria2 资源；非 hysteria2 文件（如 tuic）跳过清理
    if let Ok(ports) = Self::extract_hysteria2_ports_from_config(path).await {
        let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
        for main_port in ports {
            let hop_range = (main_port + 1, main_port + 99);
            let _ = Self::cleanup_specific_hysteria2_rules(main_port, hop_range, has_ipv6).await;
            let _ = PortAllocator::release_hysteria2_range(main_port).await;
        }
    }

    fs::remove_file(path).await.context("删除配置文件失败")?;

    Self::reload_service().await?;
    Ok(())
}
```

- [ ] **Step 6: 确认无残留引用并全量测试**

Run:
```bash
grep -rn "extract_main_port_from_config\|cleanup_specific_hysteria2_rules(main_port, hop_range)" rust/aegis/src --include="*.rs" | grep -v target
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo test --doc
```
Expected: grep 无旧函数残留引用；fmt/clippy/nextest/doc 全绿。

- [ ] **Step 7: 提交**

```bash
git add rust/aegis/src/core/singbox/config.rs
git commit -m "fix(singbox): 删除 hysteria2 配置时释放全部端口跳跃范围，重建可还原端口"
```

---

### Task 4: 服务器实测验证（删除→重建还原端口）

**Files:** 无代码改动（服务器 102.134.50.23 验证）。

- [ ] **Step 1: 构建并部署修复后的 aegis 到服务器**

先本地构建：
```bash
cd rust/aegis && cargo build --release
```
将产物与当前部署比对后，按服务器现有部署方式（见 `/etc/wwps/aegis` 下 aegis.bak-* 命名约定）替换并重启 aegis 服务。

- [ ] **Step 2: 备份现场状态**

```bash
ssh root@102.134.50.23 "cp /etc/wwps/.port_alloc /etc/wwps/.port_alloc.bak-$(date +%s)"
```

- [ ] **Step 3: 记录修复前状态**

```bash
ssh root@102.134.50.23 "cat /etc/wwps/.port_alloc | grep -c start; iptables -t nat -L PREROUTING -n | grep -c REDIRECT"
```

- [ ] **Step 4: 通过 Telegram 删除全部配置（sb_del_all_exec），观察日志**

Expected: `.port_alloc` 中剩余 0 个 hysteria2 范围（全部释放）；`iptables -t nat -L PREROUTING` 无残留 REDIRECT 规则；aegis 日志出现"已清理 Hysteria2 端口跳跃规则"与"端口范围已释放"。

- [ ] **Step 5: 重新创建批量 hy2（开启端口跳跃）**

Expected: 分配的主端口回到最低空闲段（如 10000），与删除前一致（还原端口）。

- [ ] **Step 6: 恢复现场**

验证完成后按需恢复或保留新配置；清理 `.port_alloc.bak-*` 备份。

---

## Self-Review

- **Spec 覆盖**：三个删除路径（Task 3）全部接上释放逻辑；多 inbound 文件（Task 2）全部端口释放；文件名误判（Task 3 内容检测）修复；端口还原复用回归测试（Task 1）覆盖用户可见行为；服务器实测（Task 4）端到端验证。
- **占位符扫描**：所有步骤含完整代码，无 TBD。
- **类型一致性**：`extract_hysteria2_ports_from_config -> Result<Vec<u16>>`、`cleanup_hysteria2_config(path, has_ipv6) -> Result<()>`、`allocate_hysteria2_at(path: &Path)`、`release_hysteria2_range_at(path, main_port)` 在 Task 1-3 中签名一致；`cleanup_specific_hysteria2_rules(main_port, hop_range, has_ipv6)` 三处调用点均传 has_ipv6。
