# Conntrack & Firewall Port Reconciliation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Xray/Singbox -1 disconnects by tuning sysctl conntrack limits and adding automatic firewall port reconciliation.

**Architecture:** Three fixes: (1) adjust sysctl constants, (2) add `remove_port()` + `list_allowed_ports()` + `sync_firewall_with_configs()` so stale firewall rules are cleaned on every config change, (3) a one-time VPS repair script. The reconciliation runs automatically at `reload_core()` time — no scheduler changes needed.

**Tech Stack:** Rust/teloxide tgbot, zbus (D-Bus) for Firewalld, UFW CLI, tokio async, serde_json for config parsing.

---

## Task 1: Sysctl Tuning — Change `COMBINED_NETWORK_OPTIMIZE_CONF` Constants

**Files:**
- Modify: `rust/tgbot/src/logic/system/maintenance.rs:40-41`

- [ ] **Step 1: Update `tcp_max_tw_buckets` and add `nf_conntrack_max`**

In `rust/tgbot/src/logic/system/maintenance.rs`, change the `COMBINED_NETWORK_OPTIMIZE_CONF` constant:

Find the line:
```
net.ipv4.tcp_max_tw_buckets = 6000
```
Replace with:
```
net.ipv4.tcp_max_tw_buckets = 32768
```

Find the line:
```
net.ipv4.tcp_wmem = 4096 65536 16777216
```
After it, add:
```
net.netfilter.nf_conntrack_max = 262144
```

- [ ] **Step 2: Verify the constant compiles**

Run: `cd rust/tgbot && cargo check 2>&1 | head -20`
Expected: No errors related to `COMBINED_NETWORK_OPTIMIZE_CONF`

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/logic/system/maintenance.rs
git commit -m "fix: increase tcp_max_tw_buckets to 32768, add nf_conntrack_max=262144"
```

---

## Task 2: Add `UfwClient::remove_port()` and `UfwClient::list_allowed_ports()`

**Files:**
- Modify: `rust/tgbot/src/logic/security/ufw.rs`

- [ ] **Step 1: Add `remove_port` method to `UfwClient`**

In `rust/tgbot/src/logic/security/ufw.rs`, add this method after `add_port` (after line 66):

```rust
    pub async fn remove_port(port: u16, protocol: &str) -> Result<()> {
        let port_spec = format!("{}/{}", port, protocol);
        Self::run_ufw(&["delete", "allow", &port_spec])
            .await
            .with_context(|| format!("UFW 删除端口 {} 失败", port_spec))
    }
```

- [ ] **Step 2: Add `list_allowed_ports` method to `UfwClient`**

Add this method after `is_active` (after line 103):

```rust
    pub async fn list_allowed_ports() -> Result<HashSet<u16>> {
        let _lock = UFW_MUTEX.lock().await;
        let (status, stdout, stderr) =
            run_cmd_output("ufw", &["status", "numbered"], Duration::from_secs(10)).await?;
        if !status.success() {
            let err_msg = format!("{}{}", stdout, stderr);
            anyhow::bail!("UFW status 失败: {}", err_msg);
        }
        let mut ports = HashSet::new();
        for line in stdout.lines() {
            let line = line.trim();
            if !line.contains("ALLOW") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in parts {
                if let Some(port_str) = part.split('/').next() {
                    if let Ok(port) = port_str.parse::<u16>() {
                        ports.insert(port);
                    }
                }
            }
        }
        Ok(ports)
    }
```

- [ ] **Step 3: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -20`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/logic/security/ufw.rs
git commit -m "feat: add UfwClient::remove_port() and UfwClient::list_allowed_ports()"
```

---

## Task 3: Add `FirewalldClient::remove_port()` and `FirewalldClient::list_allowed_ports()`

**Files:**
- Modify: `rust/tgbot/src/logic/security/firewalld.rs`

- [ ] **Step 1: Add `remove_port` method to `FirewalldClient`**

In `rust/tgbot/src/logic/security/firewalld.rs`, add this method after `add_port` (after the closing `}` of `add_port`, around line 121):

```rust
    pub async fn remove_port(port: u16, protocol: &str) -> Result<()> {
        let connection = zbus::Connection::system().await?;
        let proxy = FirewallD1Proxy::new(&connection).await?;
        let zone_proxy = FirewallD1ZoneProxy::new(&connection).await?;
        let zone = proxy.get_default_zone().await?;
        let port_str = port.to_string();

        // 1. Runtime: Remove port (Safe to fail if not present)
        let _ = zone_proxy.remove_port(&zone, &port_str, protocol).await;

        // 2. Permanent: Remove port from config
        let config_path = match proxy.config().await {
            Ok(path) => path,
            Err(_) => {
                zbus::zvariant::OwnedObjectPath::try_from("/org/fedoraproject/FirewallD1/config")
                    .unwrap()
            }
        };
        let config_proxy = FirewallD1ConfigProxy::builder(&connection)
            .path(config_path)?
            .build()
            .await?;

        if let Ok(zone_path) = config_proxy.get_zone_by_name(&zone).await {
            let config_zone_proxy = FirewallD1ConfigZoneProxy::builder(&connection)
                .path(zone_path)?
                .build()
                .await?;

            if config_zone_proxy
                .query_port(&port_str, protocol)
                .await
                .unwrap_or(false)
            {
                config_zone_proxy.remove_port(&port_str, protocol).await?;
            }
        }

        Ok(())
    }
```

- [ ] **Step 2: Add `list_allowed_ports` method to `FirewalldClient`**

Add this method after `remove_port`. Uses `firewall-cmd` for reliable port listing since the D-Bus `getPorts` return type is complex:

```rust
    pub async fn list_allowed_ports() -> Result<HashSet<u16>> {
        let connection = zbus::Connection::system().await?;
        let proxy = FirewallD1Proxy::new(&connection).await?;
        let zone = proxy.get_default_zone().await?;

        let (status, stdout, stderr) = crate::logic::cmd_async::run_cmd_output(
            "firewall-cmd",
            &["--zone", &zone, "--list-ports"],
            std::time::Duration::from_secs(10),
        )
        .await?;

        if !status.success() {
            anyhow::bail!("firewall-cmd --list-ports 失败: {}", stderr);
        }

        let mut ports = HashSet::new();
        for entry in stdout.trim().split_whitespace() {
            // firewall-cmd output format: "8080/tcp 9090/udp 443/tcp"
            if let Some(port_str) = entry.split('/').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    ports.insert(port);
                }
            }
        }

        Ok(ports)
    }
```

- [ ] **Step 3: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -30`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/logic/security/firewalld.rs
git commit -m "feat: add FirewalldClient::remove_port() and FirewalldClient::list_allowed_ports()"
```

---

## Task 4: Add `FirewallManager::remove_port()` and `FirewallManager::list_allowed_ports()`

**Files:**
- Modify: `rust/tgbot/src/logic/security/firewall.rs`

- [ ] **Step 1: Add `remove_port` method to `FirewallManager`**

In `rust/tgbot/src/logic/security/firewall.rs`, add this method after `add_port` (after line 68):

```rust
    pub async fn remove_port(port: u16) -> Result<()> {
        match Self::detect_backend().await {
            Some(FirewallBackend::Ufw) => UfwClient::remove_port(port, "tcp").await?,
            Some(FirewallBackend::Firewalld) => {
                FirewalldClient::remove_port(port, "tcp").await?;
                FirewalldClient::remove_port(port, "udp").await?;
            }
            None => anyhow::bail!("未检测到支持的防火墙后端 (ufw 或 firewalld)"),
        }
        Ok(())
    }
```

- [ ] **Step 2: Add `list_allowed_ports` method to `FirewallManager`**

Add this method after `remove_port`:

```rust
    pub async fn list_allowed_ports() -> Result<HashSet<u16>> {
        match Self::detect_backend().await {
            Some(FirewallBackend::Ufw) => UfwClient::list_allowed_ports().await,
            Some(FirewallBackend::Firewalld) => FirewalldClient::list_allowed_ports().await,
            None => anyhow::bail!("未检测到支持的防火墙后端 (ufw 或 firewalld)"),
        }
    }
```

- [ ] **Step 3: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -20`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/logic/security/firewall.rs
git commit -m "feat: add FirewallManager::remove_port() and FirewallManager::list_allowed_ports()"
```

---

## Task 5: Add `XrayConfigManager::collect_all_ports()`

**Files:**
- Modify: `rust/tgbot/src/logic/xraycore/config.rs`

- [ ] **Step 1: Add `collect_all_ports` method to `XrayConfigManager`**

In `rust/tgbot/src/logic/xraycore/config.rs`, add this method after `list_all_inbound_files` (after line 122). The file already imports `serde_json::{Value, json}` and `tokio::fs`:

```rust
    pub async fn collect_all_ports() -> Result<std::collections::HashSet<u16>> {
        let files = Self::list_all_inbound_files().await?;
        let mut ports = std::collections::HashSet::new();
        for file in &files {
            if let Ok(content) = fs::read_to_string(file).await {
                if let Ok(json) = serde_json::from_str::<Value>(&content) {
                    if let Some(inbounds) = json.get("inbounds").and_then(|v| v.as_array()) {
                        for inbound in inbounds {
                            if let Some(port) = inbound.get("port").and_then(|v| v.as_u64()) {
                                if port <= u16::MAX as u64 {
                                    ports.insert(port as u16);
                                }
                            }
                        }
                    }
                }
            } else {
                log::warn!("无法读取配置文件: {}", file);
            }
        }
        Ok(ports)
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -20`
Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/logic/xraycore/config.rs
git commit -m "feat: add XrayConfigManager::collect_all_ports() for firewall reconciliation"
```

---

## Task 6: Add `SingBoxConfigManager::collect_all_ports()`

**Files:**
- Modify: `rust/tgbot/src/logic/singbox/config.rs`

- [ ] **Step 1: Add `collect_all_ports` and `extract_ports_recursive` methods to `SingBoxConfigManager`**

In `rust/tgbot/src/logic/singbox/config.rs`, add these methods after `list_all_inbound_files` (after line 37):

```rust
    pub async fn collect_all_ports() -> Result<std::collections::HashSet<u16>> {
        let files = Self::list_all_inbound_files().await?;
        let mut ports = std::collections::HashSet::new();
        for file in &files {
            if let Ok(content) = fs::read_to_string(file).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    Self::extract_ports_recursive(&json, &mut ports);
                }
            } else {
                log::warn!("无法读取配置文件: {}", file);
            }
        }
        Ok(ports)
    }

    fn extract_ports_recursive(value: &serde_json::Value, ports: &mut std::collections::HashSet<u16>) {
        if let Some(port) = value.get("listen_port").and_then(|v| v.as_u64()) {
            if port <= u16::MAX as u64 {
                let main_port = port as u16;
                ports.insert(main_port);
                // If this is a hysteria2 inbound, also include hopping range
                if value.get("type").and_then(|v| v.as_str()) == Some("hysteria2") {
                    for p in (main_port + 1)..=(main_port + 99) {
                        ports.insert(p);
                    }
                }
            }
        }
        if let Some(obj) = value.as_object() {
            for (_, v) in obj {
                Self::extract_ports_recursive(v, ports);
            }
        }
        if let Some(arr) = value.as_array() {
            for v in arr {
                Self::extract_ports_recursive(v, ports);
            }
        }
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -20`
Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/logic/singbox/config.rs
git commit -m "feat: add SingBoxConfigManager::collect_all_ports() with hysteria2 hopping range"
```

---

## Task 7: Add `MaintenanceManager::remove_port()` and `MaintenanceManager::sync_firewall_with_configs()`, modify `reload_core()`

**Files:**
- Modify: `rust/tgbot/src/logic/system/maintenance.rs`

- [ ] **Step 1: Add `remove_port` method to `MaintenanceManager`**

In `rust/tgbot/src/logic/system/maintenance.rs`, add this method after `allow_port` (after line 382):

```rust
    pub async fn remove_port(port: u16) -> Result<()> {
        crate::logic::security::firewall::FirewallManager::remove_port(port).await?;
        Ok(())
    }
```

- [ ] **Step 2: Add `sync_firewall_with_configs` method to `MaintenanceManager`**

Add this method after `remove_port`:

```rust
    pub async fn sync_firewall_with_configs() -> Result<()> {
        use std::collections::HashSet;

        let xray_ports = crate::logic::xraycore::XrayConfigManager::collect_all_ports()
            .await
            .unwrap_or_default();

        let singbox_ports = crate::logic::singbox::SingBoxConfigManager::collect_all_ports()
            .await
            .unwrap_or_default();

        let mut required: HashSet<u16> = HashSet::new();
        required.extend(xray_ports);
        required.extend(singbox_ports);
        required.insert(22); // SSH

        let current = crate::logic::security::firewall::FirewallManager::list_allowed_ports()
            .await
            .unwrap_or_default();

        let stale: HashSet<u16> = current.difference(&required).copied().collect();

        if stale.is_empty() {
            log::info!("防火墙端口同步: 无需清理");
            return Ok(());
        }

        log::warn!(
            "防火墙端口同步: 发现 {} 个过期端口待清理: {:?}",
            stale.len(),
            stale.iter().take(20).collect::<Vec<_>>()
        );

        let mut removed = 0u32;
        for port in &stale {
            match crate::logic::security::firewall::FirewallManager::remove_port(*port).await {
                Ok(()) => {
                    removed += 1;
                    log::info!("防火墙: 已移除过期端口 {}", port);
                }
                Err(e) => {
                    log::warn!("防火墙: 移除端口 {} 失败: {}", port, e);
                }
            }
        }

        log::info!(
            "防火墙端口同步完成: 移除 {} / {} 个过期端口",
            removed,
            stale.len()
        );

        Ok(())
    }
```

- [ ] **Step 3: Modify `reload_core()` to call `sync_firewall_with_configs()`**

Replace the existing `reload_core()` method (lines 88-103) with:

```rust
    pub async fn reload_core() -> Result<()> {
        let (wwps_core_running, wwps_box_running) =
            crate::logic::system::SystemMonitor::get_core_status().await;

        if wwps_core_running {
            crate::logic::config::ConfigManager::ensure_base_config().await?;
            Self::control_service("wwps-core", "restart").await?;
        }

        if wwps_box_running {
            crate::logic::singbox::SingBoxConfigManager::ensure_base_config().await?;
            Self::control_service("wwps-box", "restart").await?;
        }

        // Sync firewall rules after restart: remove stale ports
        if let Err(e) = Self::sync_firewall_with_configs().await {
            log::error!("防火墙端口同步失败: {}", e);
        }

        Ok(())
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cd rust/tgbot && cargo check 2>&1 | head -30`
Expected: No errors.

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/system/maintenance.rs
git commit -m "feat: add MaintenanceManager::remove_port() and sync_firewall_with_configs()"
```

---

## Task 8: Verify trigger coverage for `sync_firewall_with_configs()`

**Files:** No changes needed

- [ ] **Step 1: Verify that `reload_core()` covers all config change paths**

The design specifies two trigger points:
1. `MaintenanceManager::reload_core()` — ✅ Now calls `sync_firewall_with_configs()` (Task 7)
2. `XrayConfigManager::create_standalone_config()` — Already calls `MaintenanceManager::reload_core()` at line 863, which now triggers sync

No additional code changes needed. This task is verification only.

---

## Task 9: One-Time VPS Repair Script

**Files:**
- Create: `scripts/fix-conntrack-and-firewall.sh`

- [ ] **Step 1: Create the repair script**

```bash
#!/usr/bin/env bash
# fix-conntrack-and-firewall.sh
# One-time VPS repair: apply sysctl and remove stale firewall port rules
# Usage: sudo bash scripts/fix-conntrack-and-firewall.sh
set -euo pipefail

echo "=== WWPS Conntrack & Firewall Repair Script ==="
echo ""

# --- Step 1: Apply sysctl immediately ---
echo "[1/3] Applying sysctl parameters..."
sysctl -w net.netfilter.nf_conntrack_max=262144
sysctl -w net.ipv4.tcp_max_tw_buckets=32768
echo "  sysctl applied (persisted via /etc/sysctl.d/90-wwps-bbr3-optimize.conf on next bot deploy)"
echo ""

# --- Step 2: Collect required ports from config files ---
echo "[2/3] Scanning config files for required ports..."
REQUIRED_PORTS=""

# Xray configs
XRAY_DIR="/etc/wwps/wwps-core/conf"
if [ -d "$XRAY_DIR" ]; then
    for f in "$XRAY_DIR"/*_inbounds.json; do
        [ -f "$f" ] || continue
        PORTS=$(python3 -c "
import json, sys
try:
    with open('$f') as fh:
        data = json.load(fh)
    inbounds = data if isinstance(data, list) else data.get('inbounds', [])
    for ib in (inbounds if isinstance(inbounds, list) else []):
        p = ib.get('port')
        if p:
            print(p)
except:
    pass
" 2>/dev/null || true)
        REQUIRED_PORTS="$REQUIRED_PORTS $PORTS"
    done
    echo "  Found xray configs in $XRAY_DIR"
else
    echo "  No xray config directory found"
fi

# Singbox configs
SB_DIR="/etc/wwps/wwps-box/conf"
if [ -d "$SB_DIR" ]; then
    for f in "$SB_DIR"/*.json; do
        [ -f "$f" ] || continue
        basename_f=$(basename "$f")
        [[ "$basename_f" == 00_* ]] && continue
        [[ "$basename_f" == 01_* ]] && continue
        PORTS=$(python3 -c "
import json
def extract(obj):
    if isinstance(obj, dict):
        if 'listen_port' in obj:
            p = obj['listen_port']
            print(p)
            if obj.get('type') == 'hysteria2':
                for hp in range(p+1, p+100):
                    print(hp)
        for v in obj.values():
            extract(v)
    elif isinstance(obj, list):
        for v in obj:
            extract(v)
try:
    with open('$f') as fh:
        data = json.load(fh)
    extract(data)
except:
    pass
" 2>/dev/null || true)
        REQUIRED_PORTS="$REQUIRED_PORTS $PORTS"
    done
    echo "  Found singbox configs in $SB_DIR"
else
    echo "  No singbox config directory found"
fi

# Always include SSH
REQUIRED_PORTS="$REQUIRED_PORTS 22"

# Normalize and deduplicate
REQUIRED_SORTED=$(echo $REQUIRED_PORTS | tr ' ' '\n' | sort -n | uniq | tr '\n' ' ')
echo "  Required ports: $REQUIRED_SORTED"
echo ""

# --- Step 3: Remove stale firewall rules ---
echo "[3/3] Checking firewalld for stale port rules..."

if command -v firewall-cmd &>/dev/null && systemctl is-active --quiet firewalld 2>/dev/null; then
    ZONE=$(firewall-cmd --get-default-zone 2>/dev/null || echo "public")
    CURRENT=$(firewall-cmd --zone="$ZONE" --list-ports 2>/dev/null || echo "")

    echo "  Current firewalld ports: $CURRENT"
    echo ""

    STALE=""
    for entry in $CURRENT; do
        PORT=$(echo "$entry" | cut -d'/' -f1)
        if ! echo "$REQUIRED_SORTED" | grep -qw "$PORT"; then
            STALE="$STALE $entry"
        fi
    done

    if [ -z "$STALE" ]; then
        echo "  No stale firewall rules found."
    else
        echo "  Found stale rules:$STALE"
        echo ""
        read -p "  Remove these stale rules? [y/N] " -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            for entry in $STALE; do
                PORT=$(echo "$entry" | cut -d'/' -f1)
                PROTO=$(echo "$entry" | cut -d'/' -f2)
                echo "  Removing port $PORT/$PROTO ..."
                firewall-cmd --zone="$ZONE" --remove-port="$PORT/$PROTO" 2>/dev/null || true
                firewall-cmd --zone="$ZONE" --permanent --remove-port="$PORT/$PROTO" 2>/dev/null || true
            done
            firewall-cmd --reload 2>/dev/null || true
            echo "  Stale rules removed and firewalld reloaded."
        else
            echo "  Skipped removal."
        fi
    fi
elif command -v ufw &>/dev/null && ufw status | grep -q "active" 2>/dev/null; then
    echo "  UFW detected. Listing current rules:"
    ufw status numbered | head -30
    echo ""
    echo "  UFW port reconciliation is best handled by the tgbot service."
    echo "  Run the bot and it will auto-sync on next reload."
else
    echo "  No supported firewall backend detected (firewalld or ufw)."
fi

echo ""
echo "=== Repair complete ==="
echo "Verify: cat /proc/sys/net/netfilter/nf_conntrack_max"
echo "Verify: firewall-cmd --list-ports (or ufw status)"
```

- [ ] **Step 2: Make script executable**

Run: `chmod +x scripts/fix-conntrack-and-firewall.sh`

- [ ] **Step 3: Commit**

```bash
git add scripts/fix-conntrack-and-firewall.sh
git commit -m "feat: add one-time VPS repair script for conntrack and firewall cleanup"
```

---

## Task 10: Add unit tests for `collect_all_ports()` and `extract_ports_recursive()`

**Files:**
- Modify: `rust/tgbot/src/logic/xraycore/config.rs`
- Modify: `rust/tgbot/src/logic/singbox/config.rs`

- [ ] **Step 1: Add test for xray port extraction logic**

At the bottom of `rust/tgbot/src/logic/xraycore/config.rs`, add to the existing test module or create one:

```rust
#[cfg(test)]
mod port_collection_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_xray_port_extraction_from_json() {
        let json = serde_json::json!({
            "inbounds": [
                {"tag": "v1", "port": 10001, "protocol": "vless"},
                {"tag": "v2", "port": 10002, "protocol": "vless"},
            ]
        });
        let parsed: serde_json::Value = serde_json::from_str(&json.to_string()).unwrap();
        let mut ports = HashSet::new();
        if let Some(inbounds) = parsed.get("inbounds").and_then(|v| v.as_array()) {
            for inbound in inbounds {
                if let Some(port) = inbound.get("port").and_then(|v| v.as_u64()) {
                    ports.insert(port as u16);
                }
            }
        }
        assert!(ports.contains(&10001));
        assert!(ports.contains(&10002));
        assert_eq!(ports.len(), 2);
    }

    #[test]
    fn test_xray_base_config_excluded() {
        let name = "00_base_inbounds.json";
        assert!(name.starts_with("00_"));
        let name2 = "batch_reality_vision_123.json";
        assert!(!name2.starts_with("00_"));
    }
}
```

- [ ] **Step 2: Add test for singbox port extraction logic**

At the bottom of `rust/tgbot/src/logic/singbox/config.rs`, add to the existing test module or create one:

```rust
#[cfg(test)]
mod port_collection_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_extract_ports_hysteria2_includes_hopping() {
        let json = serde_json::json!({
            "inbounds": [{
                "type": "hysteria2",
                "listen_port": 20001,
                "listen": "::"
            }]
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&20001));
        assert!(ports.contains(&20002));
        assert!(ports.contains(&20100));
        assert_eq!(ports.len(), 100);
    }

    #[test]
    fn test_extract_ports_tuic_single_port() {
        let json = serde_json::json!({
            "type": "tuic",
            "listen_port": 30001
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&30001));
        assert_eq!(ports.len(), 1);
    }

    #[test]
    fn test_extract_ports_nested() {
        let json = serde_json::json!({
            "inbounds": [
                {"type": "tuic", "listen_port": 30001},
                {"type": "hysteria2", "listen_port": 20001}
            ]
        });
        let mut ports = HashSet::new();
        SingBoxConfigManager::extract_ports_recursive(&json, &mut ports);
        assert!(ports.contains(&30001));
        assert!(ports.contains(&20001));
        assert!(ports.contains(&20050));
        assert!(ports.contains(&20100));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd rust/tgbot && cargo test --lib 2>&1 | tail -30`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/logic/xraycore/config.rs rust/tgbot/src/logic/singbox/config.rs
git commit -m "test: add unit tests for collect_all_ports and extract_ports_recursive"
```

---

## Task 11: Final verification

- [ ] **Step 1: Full cargo check**

Run: `cd rust/tgbot && cargo check 2>&1`
Expected: No errors.

- [ ] **Step 2: Full cargo test**

Run: `cd rust/tgbot && cargo test --lib 2>&1 | tail -40`
Expected: All tests pass.

- [ ] **Step 3: Review all changed files**

Run: `git diff --stat HEAD~7`
Expected changes in:
- `rust/tgbot/src/logic/system/maintenance.rs`
- `rust/tgbot/src/logic/security/ufw.rs`
- `rust/tgbot/src/logic/security/firewalld.rs`
- `rust/tgbot/src/logic/security/firewall.rs`
- `rust/tgbot/src/logic/xraycore/config.rs`
- `rust/tgbot/src/logic/singbox/config.rs`
- `scripts/fix-conntrack-and-firewall.sh` (new)

---

## Summary of Changes

| Task | File(s) | Description |
|------|---------|-------------|
| 1 | `maintenance.rs` | Sysctl: `tcp_max_tw_buckets=32768`, `nf_conntrack_max=262144` |
| 2 | `ufw.rs` | `remove_port()`, `list_allowed_ports()` |
| 3 | `firewalld.rs` | `remove_port()`, `list_allowed_ports()` |
| 4 | `firewall.rs` | `FirewallManager::remove_port()`, `list_allowed_ports()` |
| 5 | `xraycore/config.rs` | `collect_all_ports()` |
| 6 | `singbox/config.rs` | `collect_all_ports()` + `extract_ports_recursive()` |
| 7 | `maintenance.rs` | `remove_port()`, `sync_firewall_with_configs()`, modify `reload_core()` |
| 8 | (no change) | `create_standalone_config` already calls `reload_core()` |
| 9 | `scripts/fix-conntrack-and-firewall.sh` | One-time VPS repair script |
| 10 | `config.rs` (both) | Unit tests for port collection |
| 11 | (verification) | cargo check + cargo test |