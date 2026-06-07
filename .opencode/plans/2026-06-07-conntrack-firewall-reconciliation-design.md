# Conntrack & Firewall Port Reconciliation Design

**Date**: 2026-06-07
**Status**: Approved
**Root Cause**: Xray/Singbox proxy services showing -1 (connection reset) after long runtime periods.

## Diagnosis Summary

VPS live diagnostics confirmed:

1. **`net.netfilter.nf_conntrack_max = 7680`** — catastrophically low for a proxy server with ~40 listening ports. Connection tracking table exhausts under load, new connections are dropped.
2. **`net.ipv4.tcp_max_tw_buckets = 6000`** — too low for busy proxy; TIME_WAIT sockets overflow.
3. **Firewalld port rules accumulated to 600+** (TCP+UDP each), while only ~43 ports are actually in use. Old ports from deleted configs are never removed.
4. Both services are currently running fine (just rebooted), but the above conditions guarantee degradation over time.

## Fix 1: Sysctl Tuning

**File**: `rust/tgbot/src/logic/system/maintenance.rs`

Modify `COMBINED_NETWORK_OPTIMIZE_CONF` constant:

```
- net.ipv4.tcp_max_tw_buckets = 6000
+ net.ipv4.tcp_max_tw_buckets = 32768

+ net.netfilter.nf_conntrack_max = 262144
```

Existing systems need `sysctl -p /etc/sysctl.d/90-wwps-bbr3-optimize.conf` or a reboot to apply. The VPS fix script (Fix 3) handles immediate application.

## Fix 2: Firewall Port Reconciliation Mechanism

### Core Concept

Replace the current additive-only firewall pattern with a **reconciliation model**: after every config change (create/delete/reload), automatically read all xray + singbox config files, extract ports, compare with current firewall rules, and remove any ports that are no longer in use.

This is a long-term self-healing mechanism — it runs automatically on every configuration change, not a one-time cleanup.

### New Methods

#### `src/logic/security/ufw.rs`

- `UfwClient::remove_port(port: u16, protocol: &str) -> Result<()>` — calls `ufw delete allow {port}/{protocol}`

#### `src/logic/security/firewalld.rs`

- `FirewalldClient::remove_port(port: u16, protocol: &str) -> Result<()>` — uses existing `removePort` D-Bus method for both runtime and permanent zones

#### `src/logic/security/firewall.rs`

- `FirewallManager::remove_port(port: u16) -> Result<()>` — delegates to detected backend (UFW: tcp only, Firewalld: tcp+udp), mirrors `add_port()` pattern
- `FirewallManager::list_allowed_ports() -> Result<HashSet<u16>>` — lists currently open ports from firewall rules

#### `src/logic/system/maintenance.rs`

- `MaintenanceManager::remove_port(port: u16) -> Result<()>` — delegates to `FirewallManager::remove_port()`
- `MaintenanceManager::sync_firewall_with_configs() -> Result<()>` — the reconciliation function:

  1. Call `XrayConfigManager::collect_all_ports()` to get set A
  2. Call `SingBoxConfigManager::collect_all_ports()` to get set B
  3. Compute required ports = A ∪ B ∪ {22} (SSH always needed)
  4. Call `FirewallManager::list_allowed_ports()` to get current ports C
  5. Stale ports = C - required ports
  6. For each port in stale ports: call `FirewallManager::remove_port()`
  7. Log removed ports for admin visibility

- Modify `reload_core()` to call `sync_firewall_with_configs()` after restart

#### `src/logic/xraycore/config.rs`

- `XrayConfigManager::collect_all_ports() -> Result<HashSet<u16>>` — reads all config JSON files, parses `inbounds` array, extracts all `port` fields. Skips `00_base.json` (base config, port not user-managed).

#### `src/logic/singbox/config.rs`

- `SingBoxConfigManager::collect_all_ports() -> Result<HashSet<u16>>` — reads all config JSON files, recursively extracts all `listen_port` fields. For hysteria2 configs, also includes hopping range ports (main_port+1 through main_port+99).

### Trigger Points

`sync_firewall_with_configs()` is called:
1. At the end of `MaintenanceManager::reload_core()` — catches all reload scenarios
2. After `create_standalone_config()` — immediately cleans up any stale ports from previous deletions

### What About Existing Deletion Functions?

The existing `delete_all_configurations()`, `delete_configurations_by_count()`, `delete_specific_configuration()` in both xray and singbox do NOT need to be modified to call `remove_port()`. The reconciliation mechanism handles cleanup automatically when `reload_core()` is called after deletion.

However, for singbox's `delete_specific_configuration()`, which already calls `cleanup_specific_hysteria2_rules()`, that code remains — it handles iptables NAT rules which are outside the firewall port model.

## Fix 3: One-Time VPS Repair Script

**File**: `scripts/fix-conntrack-and-firewall.sh`

A bash script that:
1. Applies sysctl immediately: `sysctl -w net.netfilter.nf_conntrack_max=262144` and `sysctl -w net.ipv4.tcp_max_tw_buckets=32768`
2. Reads actual config files to determine required ports
3. Queries firewalld for current port rules
4. Computes stale ports and removes them
5. Prompts for confirmation before removing

Safety measures:
- Never removes port 22 (SSH)
- Lists stale ports before removal
- Requires explicit confirmation

## Out of Scope

The following were considered but deliberately excluded:

- **HealthCheck scheduled task** — systemd `Restart=always` already handles process crashes; the root cause is conntrack exhaustion, not service crashes
- **AppState expired entry sweep** — only a single admin user, memory impact negligible
- **TLS cache TTL eviction** — tgbot uses only 32MB RSS, cache growth is not the problem
- **Port registry/allocator reconciliation** — the firewall reconciliation mechanism covers the operational need

## Files Changed

| File | Change |
|------|--------|
| `src/logic/system/maintenance.rs` | Sysctl constants, `remove_port()`, `sync_firewall_with_configs()` |
| `src/logic/security/ufw.rs` | `remove_port()` |
| `src/logic/security/firewalld.rs` | `remove_port()` |
| `src/logic/security/firewall.rs` | `remove_port()`, `list_allowed_ports()` |
| `src/logic/xraycore/config.rs` | `collect_all_ports()` |
| `src/logic/singbox/config.rs` | `collect_all_ports()` |
| `scripts/fix-conntrack-and-firewall.sh` | New file: one-time VPS repair |

## Testing Strategy

- Unit tests for `collect_all_ports()` with sample JSON configs
- Unit tests for `remove_port()` in UFW/Firewalld backends (mock subprocess calls)
- Integration test for `sync_firewall_with_configs()` (mock config files + firewall backend)
- Manual test on VPS: run fix script, verify conntrack values, verify firewall rules reduced to actual ports only