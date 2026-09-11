# Xray Hysteria2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Hysteria2 support to the Xray core so `aegis` can batch-create Xray `hysteria` inbounds and emit working `hysteria2://` share links, alongside the existing sing-box Hy2 support.

**Architecture:** A new `Proto::Hysteria2` variant joins `Vision`/`XHTTP`/`Kcp` in `core::xray::config::Proto` (the single source of truth for Xray protocol identity). A new `core::xray::hysteria2` module owns the inbound builder and link generator. The batch flow reuses the existing generic machinery: `ConfigManager::create_standalone_config`, `PortAllocator::check_hysteria2_limit`/`is_port_in_locked_range`, `SingBoxConfigManager::ensure_tls_certificates`/`compute_cert_sha256_pin` (shared cert at `singbox::TLS_CERT`), and `MaintenanceManager::allow_port`. Because scope is core-only (no obfs, no port hopping), it reuses the existing shared `u_xray_batch_init` UI flow rather than adding a KCP-style bespoke flow.

**Tech Stack:** Rust (edition 2024), `serde_json`, `percent-encoding`, `anyhow`, `tokio`, `rand` 0.8. Tests via `cargo nextest`.

**Spec:** Design approved in session 2026-09-02 (brainstorming, architectural path). Key external references:
- Xray inbound: https://xtls.github.io/en/config/inbounds/hysteria.html
- Xray transport: https://xtls.github.io/config/transports/hysteria.html
- FinalMask (QUIC params): https://xtls.github.io/config/transports/finalmask.html
- Reference config: https://github.com/XTLS/Xray-examples (`Hysteria2/server.jsonc`)

## Global Constraints

- **Xray inbound protocol string is `"hysteria"`, NOT `"hysteria2"`.** Xray rejects sing-box's schema.
- **Auth lives at `settings.users[].auth`** (`level`, `email` siblings). NOT `users[].password`.
- **Transport must be `"network": "hysteria"` + `"security": "tls"` with `alpn: ["h3"]` written explicitly.**
- **Settings/transport coupling is mandatory** for Xray: `settings` alone (protocol layer) is inert without the `hysteria` transport layer, and vice versa.
- **Link password MUST equal the inbound `auth` value.** A mismatch means clients cannot connect. This is the single highest-risk invariant.
- **Never emit `insecure=1` or `allowInsecure` in generated links.** Use `pinSHA256` (existing project convention, mirroring sing-box Hy2).
- **Reuse the shared TLS certificate** at `singbox::TLS_CERT` / `singbox::TLS_KEY` (i.e. `/etc/wwps/wwps-box/certs/tls.cer` / `.key`), exactly as `kcp.rs` does. Do not generate a second cert.
- **Do not modify sing-box code.** `core/singbox/hysteria2.rs` and `core/singbox/hy2_batch.rs` stay byte-identical.
- **Scope is core-only:** no `finalmask.udp` (obfs/salamander/gecko), no `quicParams.udpHop` (port hopping). Those are explicit follow-ups.
- **`Proto::Hysteria2` must NOT reuse a sing-box file prefix.** sing-box writes `batch_hy2_{ts}_{rand}.json` for hysteria2 (verified at `core/singbox/config.rs:511`). Xray must use `batch_xray_hysteria2` so the two cores' config lists stay disjoint under `starts_with` filtering. (An earlier draft of this plan said sing-box used `batch_hysteria2`; that was wrong — corrected during Task 1 review.)
- Quality gates (rust-lint-format skill): `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo nextest run`.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `rust/aegis/src/core/xray/hysteria2.rs` | Inbound builder + client link generator + batch orchestration for Xray Hysteria2 | **Create** |
| `rust/aegis/src/core/xray/mod.rs` | Register `pub mod hysteria2;` | Modify |
| `rust/aegis/src/core/xray/config.rs` | Add `Proto::Hysteria2`; fill the 4 `match proto` sites | Modify |
| `rust/aegis/src/shared/handlers/xray.rs` | UI: filter label, type label, filter match arms, batch dispatch, batch button | Modify |
| `rust/aegis/src/resources/i18n/zh.yml` | `xray.batch_hysteria2`, `xray.filter_hysteria2`, `xray.type_hysteria2` | Modify |
| `rust/aegis/src/resources/i18n/en.yml` | same keys | Modify |
| `rust/aegis/src/resources/i18n/ja.yml` | same keys | Modify |

### Why `Proto::Hysteria2` does not touch `generate_enhanced_config` / `generate_client_link`

`generate_enhanced_config` (config.rs:394) and `generate_client_link` (config.rs:~500) are the Reality/XHTTP/KCP code paths. They generate UUID + X25519 keypairs + short IDs — none of which Hysteria2 uses (it authenticates with a password, not a UUID). Hysteria2 has its own batch function in `hysteria2.rs`, exactly like KCP has `batch_create_kcp` in `kcp.rs`.

This means the `match proto` arms in `generate_enhanced_config` (config.rs:453, 461) and `generate_client_link` (config.rs:513-570) get `Proto::Hysteria2 => unreachable!(...)` arms, mirroring the existing `Proto::Kcp => unreachable!("Kcp should use generate_kcp_client_link instead")` at config.rs:570. The compiler will force us to find every one.

---

## Task 1: Add `Proto::Hysteria2` variant and fill `match proto` sites

**Files:**
- Modify: `rust/aegis/src/core/xray/config.rs:18-22` (enum), `:115-121`, `:181-186`, `:452-457`, `:460-467`, `:570-572`
- Test: `rust/aegis/src/core/xray/config.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing (foundation task).
- Produces:
  - `crate::core::xray::Proto::Hysteria2` — a new unit variant, `Copy`.
  - `Proto::Hysteria2` ⇒ filename prefix `"batch_xray_hysteria2"`, so `list_inbound_files_by_proto(Proto::Hysteria2)` returns only Xray-Hy2 files.
  - `Proto::Hysteria2` ⇒ config email suffix `"hysteria2"` and tag prefix `"HY2"` in the shared `generate_enhanced_config` path (unused at runtime but required to compile).

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `rust/aegis/src/core/xray/config.rs`:

```rust
    #[test]
    fn test_proto_hysteria2_variant_exists_and_is_copy() {
        let p = Proto::Hysteria2;
        let q = p; // Copy, not move
        assert_eq!(p, q);
    }

    #[test]
    fn test_hysteria2_filename_prefix_is_distinct_from_singbox() {
        // sing-box uses "batch_hysteria2"; Xray must not collide with it,
        // otherwise list_inbound_files_by_proto returns sing-box files too.
        assert_ne!(Proto::Hysteria2, Proto::Kcp);
        assert_ne!(Proto::Hysteria2, Proto::Vision);
        assert_ne!(Proto::Hysteria2, Proto::XHTTP);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --offline -p aegis -E 'test(test_proto_hysteria2_variant_exists)'`
Expected: FAIL to compile — `error[E0599]: no variant or associated item named 'Hysteria2' found for enum 'Proto'`

- [ ] **Step 3: Add the enum variant**

In `rust/aegis/src/core/xray/config.rs`, change:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Vision,
    XHTTP,
    Kcp,
}
```

to:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Vision,
    XHTTP,
    Kcp,
    /// Xray `hysteria` inbound (Hysteria2). Distinct from sing-box Hy2, which
    /// uses `"type": "hysteria2"` and `users[].password`; Xray uses
    /// `"protocol": "hysteria"` and `settings.users[].auth`.
    Hysteria2,
}
```

- [ ] **Step 4: Build to enumerate every non-exhaustive match**

Run: `cargo build --offline -p aegis 2>&1 | grep -E "not covered|^error" | head -20`
Expected: compile errors listing each `match proto` missing the `Hysteria2` arm. This is the compiler giving us the checklist.

- [ ] **Step 5: Fill the five match sites**

> **Revised during execution (Task 1 finding).** The brief originally said "four match sites".
> The real count is **five** — the fifth is the `network` match inside `build_reality_vless_inbound`.
> Task 1 fills all five. Sites A–D below are as written; the fifth is:
>
> Site E — `build_reality_vless_inbound` (`network` match):
>
> ```rust
>             Proto::Hysteria2 => unreachable!("Hysteria2 should use build_hysteria2_inbound"),
> ```

Site A — `list_inbound_files_by_proto` (config.rs:~115):

```rust
        let prefix = match proto {
            Proto::Vision => "batch_reality",
            Proto::XHTTP => "batch_xhttp",
            Proto::Kcp => "batch_kcp",
            Proto::Hysteria2 => "batch_xray_hysteria2",
        };
```

Site B — `generate_secure_batch_filename` (config.rs:~181):

```rust
        let prefix = match proto {
            Proto::Vision => "batch_reality",
            Proto::XHTTP => "batch_xhttp",
            Proto::Kcp => "batch_kcp",
            Proto::Hysteria2 => "batch_xray_hysteria2",
        };
```

Site C — `generate_enhanced_config` (config.rs:~452), two arms:

```rust
        let suffix = match proto {
            Proto::Vision => "vless_reality_vision",
            Proto::XHTTP => "vless_xhttp_reality",
            Proto::Kcp => "vless_kcp",
            Proto::Hysteria2 => "hysteria2",
        };
        let email = format!("{}-{}", uuid_short, suffix);
        let tag = format!(
            "{}-{}-{}",
            match proto {
                Proto::Vision => "VLESS",
                Proto::XHTTP => "XHTTP",
                Proto::Kcp => "KCP",
                Proto::Hysteria2 => "HY2",
            },
            uuid_short,
            index
        );
```

Site D — `generate_client_link` (config.rs:~570):

```rust
            Proto::Kcp => {
                unreachable!("Kcp should use generate_kcp_client_link instead")
            }
            Proto::Hysteria2 => {
                unreachable!("Hysteria2 should use generate_hysteria2_client_link instead")
            }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_proto_hysteria2)'`
Expected: PASS (2 tests)

- [ ] **Step 7: Commit**

```bash
git add rust/aegis/src/core/xray/config.rs
git commit -m "feat(xray): add Proto::Hysteria2 variant with distinct batch prefix"
```

---

## Task 2: Build the Hysteria2 inbound JSON

**Files:**
- Create: `rust/aegis/src/core/xray/hysteria2.rs`
- Modify: `rust/aegis/src/core/xray/mod.rs`
- Test: inline `#[cfg(test)] mod tests` in `rust/aegis/src/core/xray/hysteria2.rs`

**Interfaces:**
- Consumes: nothing from Task 1 at compile time beyond the module existing.
- Produces:
  - `pub(crate) fn build_hysteria2_inbound(tag: &str, port: i32, auth: &str, email: &str, sni: &str, ip_version: IpVersion) -> serde_json::Value`
  - Returns a JSON object with exactly: `listen`, `port`, `protocol="hysteria"`, `tag`, `settings.{version=2, users[0].{auth,level,email}}`, `streamSettings.{network="hysteria", security="tls", tlsSettings.{serverName, alpn=["h3"], certificates[0].{certificateFile, keyFile}}, hysteriaSettings.{version=2, auth}}`, `sniffing`.

- [ ] **Step 1: Register the module so the test can compile**

In `rust/aegis/src/core/xray/mod.rs`, add `pub mod hysteria2;` in alphabetical order (after `config`):

```rust
pub mod config;
pub mod hysteria2;
pub mod installer;
```

- [ ] **Step 2: Create the module with a failing test**

Create `rust/aegis/src/core/xray/hysteria2.rs`:

```rust
use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::types::IpVersion;

impl ConfigManager {
    /// Build a single Xray `hysteria` inbound (Hysteria2).
    ///
    /// Xray splits Hysteria2 configuration across two layers and BOTH must be
    /// present: the protocol layer (`protocol: "hysteria"` + `settings.users`)
    /// and the transport layer (`network: "hysteria"` + `hysteriaSettings`).
    /// Either alone produces an inbound that never authenticates.
    pub(crate) fn build_hysteria2_inbound(
        tag: &str,
        port: i32,
        auth: &str,
        email: &str,
        sni: &str,
        ip_version: IpVersion,
    ) -> Value {
        let listen_ip = match ip_version {
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => "0.0.0.0",
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => "::",
        };

        json!({
            "listen": listen_ip,
            "port": port,
            "protocol": "hysteria",
            "tag": tag,
            "settings": {
                "version": 2,
                "users": [{
                    "auth": auth,
                    "level": 0,
                    "email": email
                }]
            },
            "streamSettings": {
                "network": "hysteria",
                "security": "tls",
                "tlsSettings": {
                    "serverName": sni,
                    "alpn": ["h3"],
                    "certificates": [{
                        "usage": "encipherment",
                        "certificateFile": singbox::TLS_CERT,
                        "keyFile": singbox::TLS_KEY
                    }]
                },
                "hysteriaSettings": {
                    "version": 2,
                    "auth": auth
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "metadataOnly": false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH: &str = "TestAuth1234567890abcdefghijklmn";

    fn build(ip_version: IpVersion) -> Value {
        ConfigManager::build_hysteria2_inbound(
            "HY2-test-1",
            11451,
            AUTH,
            "abcd1234-hysteria2",
            "www.bing.com",
            ip_version,
        )
    }

    #[test]
    fn test_hysteria2_inbound_uses_xray_protocol_name_not_singbox() {
        let cfg = build(IpVersion::IPv4);
        // Xray protocol string is "hysteria"; sing-box's "hysteria2" is rejected.
        assert_eq!(cfg["protocol"], "hysteria");
        assert_ne!(cfg["protocol"], "hysteria2");
    }

    #[test]
    fn test_hysteria2_inbound_has_both_protocol_and_transport_layers() {
        let cfg = build(IpVersion::IPv4);
        // Both layers are mandatory; either alone never authenticates.
        assert_eq!(cfg["settings"]["version"], 2);
        assert_eq!(cfg["streamSettings"]["network"], "hysteria");
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["version"], 2);
    }

    #[test]
    fn test_hysteria2_inbound_auth_matches_across_both_layers() {
        let cfg = build(IpVersion::IPv4);
        // A mismatch between these two means clients cannot connect.
        assert_eq!(cfg["settings"]["users"][0]["auth"], AUTH);
        assert_eq!(cfg["streamSettings"]["hysteriaSettings"]["auth"], AUTH);
    }

    #[test]
    fn test_hysteria2_inbound_uses_auth_not_password_field() {
        let cfg = build(IpVersion::IPv4);
        let user = &cfg["settings"]["users"][0];
        assert_eq!(user["auth"], AUTH);
        // sing-box uses users[].password; Xray must not.
        assert!(user.get("password").is_none());
    }

    #[test]
    fn test_hysteria2_inbound_alpn_is_explicitly_h3() {
        let cfg = build(IpVersion::IPv4);
        assert_eq!(cfg["streamSettings"]["security"], "tls");
        assert_eq!(cfg["streamSettings"]["tlsSettings"]["alpn"], json!(["h3"]));
        assert_eq!(
            cfg["streamSettings"]["tlsSettings"]["serverName"],
            "www.bing.com"
        );
    }

    #[test]
    fn test_hysteria2_inbound_reuses_shared_tls_certificate() {
        let cfg = build(IpVersion::IPv4);
        let cert = &cfg["streamSettings"]["tlsSettings"]["certificates"][0];
        assert_eq!(cert["certificateFile"], singbox::TLS_CERT);
        assert_eq!(cert["keyFile"], singbox::TLS_KEY);
    }

    #[test]
    fn test_hysteria2_inbound_listen_ip_per_version() {
        assert_eq!(build(IpVersion::IPv4)["listen"], "0.0.0.0");
        assert_eq!(build(IpVersion::SplitStackV4Primary)["listen"], "0.0.0.0");
        assert_eq!(build(IpVersion::IPv6)["listen"], "::");
        assert_eq!(build(IpVersion::SplitStackV6Primary)["listen"], "::");
    }

    #[test]
    fn test_hysteria2_inbound_records_tag_port_email() {
        let cfg = build(IpVersion::IPv4);
        assert_eq!(cfg["tag"], "HY2-test-1");
        assert_eq!(cfg["port"], 11451);
        assert_eq!(cfg["settings"]["users"][0]["email"], "abcd1234-hysteria2");
        assert_eq!(cfg["settings"]["users"][0]["level"], 0);
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_inbound)'`
Expected: PASS (8 tests)

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/xray/mod.rs rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): build hysteria2 inbound with dual protocol+transport layers"
```

---

## Task 3: Generate the `hysteria2://` client link

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs` (append to the `impl ConfigManager` block and to `mod tests`)
- Test: inline tests

**Interfaces:**
- Consumes: `build_hysteria2_inbound` (Task 2, same file) — link and inbound must share the same `auth`.
- Produces:
  - `pub(crate) fn generate_hysteria2_client_link(auth: &str, host: &str, port: i32, sni: &str, email: &str, ip_version: IpVersion, pin: &str) -> String`
  - Format: `hysteria2://<pct-encoded auth>@<host>:<port>?sni=<pct-encoded sni>&alpn=h3&pinSHA256=<pct-encoded pin>#<pct-encoded email>`
  - IPv6 hosts are bracketed `[::1]`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `rust/aegis/src/core/xray/hysteria2.rs`:

```rust
    #[test]
    fn test_hysteria2_link_uses_hysteria2_uri_scheme() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "abcd-hysteria2",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        assert!(link.starts_with("hysteria2://pw123@203.0.113.1:11451?"));
        assert!(link.contains("sni=www.bing.com"));
        assert!(link.contains("alpn=h3"));
        assert!(link.ends_with("#abcd-hysteria2"));
    }

    #[test]
    fn test_hysteria2_link_never_enables_insecure_bypass() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        assert!(!link.contains("insecure=1"));
        assert!(!link.contains("allowInsecure"));
        assert!(!link.contains("security=none"));
    }

    #[test]
    fn test_hysteria2_link_pins_certificate_with_percent_encoding() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB:CC:DD",
        );
        // Colons must be escaped so they do not terminate the query value.
        assert!(link.contains("pinSHA256=AA%3ABB%3ACC%3ADD"));
    }

    #[test]
    fn test_hysteria2_link_encodes_special_characters_in_auth() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "p@ss!word",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA",
        );
        assert!(link.contains("@203.0.113.1:11451"));
        assert!(link.contains("p%40ss%21word"));
        assert!(!link.contains("p@ss!word@"));
    }

    #[test]
    fn test_hysteria2_link_brackets_ipv6_host() {
        let link = ConfigManager::generate_hysteria2_client_link(
            "pw123",
            "2001:db8::1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv6,
            "AA",
        );
        assert!(link.contains("@[2001:db8::1]:11451?"));
    }

    #[test]
    fn test_hysteria2_link_auth_matches_generated_inbound() {
        // The link password and the inbound auth must be the same value,
        // otherwise the client cannot authenticate.
        let auth = "SharedAuthValue0123456789abcdefg";
        let inbound = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            auth,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
        );
        let link = ConfigManager::generate_hysteria2_client_link(
            auth,
            "203.0.113.1",
            11451,
            "www.bing.com",
            "e",
            IpVersion::IPv4,
            "AA",
        );
        assert_eq!(inbound["settings"]["users"][0]["auth"], auth);
        assert!(link.contains(&format!("hysteria2://{auth}@")));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_link)'`
Expected: FAIL to compile — `error[E0599]: no function or associated item named 'generate_hysteria2_client_link' found`

- [ ] **Step 3: Implement the link generator**

In `rust/aegis/src/core/xray/hysteria2.rs`, add `percent_encoding` to the imports and append the function to the `impl ConfigManager` block:

```rust
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

use super::config::ConfigManager;
use crate::core::paths::singbox;
use crate::core::types::IpVersion;
```

```rust
    /// Build a `hysteria2://` share link for an Xray `hysteria` inbound.
    ///
    /// `auth` MUST be the same value used in `build_hysteria2_inbound`'s
    /// `settings.users[].auth`, otherwise the client cannot authenticate.
    pub(crate) fn generate_hysteria2_client_link(
        auth: &str,
        host: &str,
        port: i32,
        sni: &str,
        email: &str,
        ip_version: IpVersion,
        pin: &str,
    ) -> String {
        let encoded_auth = utf8_percent_encode(auth, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();
        let encoded_pin = utf8_percent_encode(pin, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };

        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3&pinSHA256={}#{}",
            encoded_auth, fmt_host, port, encoded_sni, encoded_pin, encoded_email
        )
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2)'`
Expected: PASS (14 tests total: 8 inbound + 6 link)

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): generate hysteria2:// link with pinned cert and shared auth"
```

---

## Task 4: Batch orchestration `batch_create_hysteria2_xray`

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs` (append to `impl ConfigManager` and `mod tests`)
- Test: inline tests

**Interfaces:**
- Consumes:
  - `ConfigManager::build_hysteria2_inbound` (Task 2)
  - `ConfigManager::generate_hysteria2_client_link` (Task 3)
  - `Proto::Hysteria2` (Task 1)
  - `ConfigManager::generate_wwps_uuid() -> Result<String>` (existing, config.rs:127)
  - `ConfigManager::uuid_short_prefix(&str) -> String` (existing, config.rs:188)
  - `ConfigManager::resolve_public_hosts(IpVersion, Result<String>, Result<String>) -> Result<(String, Option<String>)>` (existing, config.rs:383)
  - `ConfigManager::create_standalone_config(Vec<Value>, Vec<String>, Proto) -> Result<BatchCreationResult>` (existing, config.rs:576)
  - `PortAllocator::check_hysteria2_limit() -> Result<bool>` (existing, xray/port_allocator.rs:60)
  - `PortAllocator::is_port_in_locked_range(u16) -> bool` (existing, xray/port_allocator.rs:167)
  - `SingBoxConfigManager::ensure_tls_certificates() -> Result<()>` (existing, singbox/config.rs:407)
  - `SingBoxConfigManager::compute_cert_sha256_pin(&str) -> Result<String>` (existing, singbox/config.rs:528)
  - `MaintenanceManager::is_port_available(u16) -> bool` (existing)
  - `MaintenanceManager::allow_port(u16) -> Result<()>` (existing, maintenance.rs:615)
- Produces:
  - `pub async fn batch_create_hysteria2_xray(count: usize, ip_version: IpVersion) -> Result<BatchCreationResult>`

- [ ] **Step 1: Write the failing test**

Xray Hy2 uses a password, not a UUID. The password generator lives in the sing-box module (`Hysteria2Config::generate_password`, 32 alphanumeric chars). Verify it is usable from Xray code, and that the batch function's signature exists.

Append to the `tests` module in `rust/aegis/src/core/xray/hysteria2.rs`:

```rust
    #[test]
    fn test_hysteria2_batch_signature_is_constructible() {
        // Compile-time check: the batch entry point has the shape the handler
        // will call. `let _ = ...` avoids invoking the async body.
        let _f: fn(usize, IpVersion) -> _ =
            |count, ip_version| ConfigManager::batch_create_hysteria2_xray(count, ip_version);
    }

    #[test]
    fn test_reused_password_generator_is_32_alphanumeric() {
        use crate::core::singbox::hysteria2::Hysteria2Config;
        let pw = Hysteria2Config::generate_password();
        assert_eq!(pw.len(), 32);
        assert!(pw.chars().all(|c| c.is_ascii_alphanumeric()));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_batch_signature)'`
Expected: FAIL to compile — `no function or associated item named 'batch_create_hysteria2_xray' found`

- [ ] **Step 3: Implement the batch function**

Append to the `impl ConfigManager` block in `rust/aegis/src/core/xray/hysteria2.rs`, and extend the imports:

```rust
use anyhow::Result;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
```

```rust
    /// Batch-create `count` Xray Hysteria2 inbounds with matching share links.
    ///
    /// Scope is deliberately core-only: no `finalmask.udp` obfuscation and no
    /// `quicParams.udpHop` port hopping. Both are additive follow-ups.
    pub async fn batch_create_hysteria2_xray(
        count: usize,
        ip_version: IpVersion,
    ) -> Result<BatchCreationResult> {
        use crate::core::singbox::config::SingBoxConfigManager;

        if !crate::core::xray::port_allocator::PortAllocator::check_hysteria2_limit().await? {
            return Err(anyhow::anyhow!("已达到最大 Hysteria2 配置数量限制（50个）"));
        }

        // Shared certificate, same one sing-box Hy2 and Xray KCP use.
        SingBoxConfigManager::ensure_tls_certificates().await?;
        let pin = SingBoxConfigManager::compute_cert_sha256_pin(singbox::TLS_CERT).await?;

        let (ip, ipv6) = tokio::join!(
            crate::core::system::SystemMonitor::get_public_ip(),
            crate::core::system::SystemMonitor::get_public_ipv6(),
        );
        let (host, _) = Self::resolve_public_hosts(ip_version, ip, ipv6)?;

        let geoip = crate::core::network::geoip::GeoIPService::new();
        let country_code = geoip.get_country_code().await;
        let mut selector =
            crate::core::sni::selector::SNISelector::get_for_country(&country_code).await;

        let mut rng = StdRng::from_entropy();
        let mut links = Vec::with_capacity(count);
        let mut configs = Vec::with_capacity(count);

        for i in 0..count {
            let port = loop {
                let p = rng.gen_range(10000..60000);
                if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(p).await
                {
                    continue;
                }
                if crate::core::system::maintenance::MaintenanceManager::is_port_available(p).await
                {
                    break p as i32;
                }
            };

            // Reuse the existing 32-char alphanumeric generator rather than
            // duplicating it. Xray Hysteria2 authenticates with a password.
            let auth = crate::core::singbox::hysteria2::Hysteria2Config::generate_password();
            let uuid = Self::generate_wwps_uuid().await?;
            let uuid_short = Self::uuid_short_prefix(&uuid);
            let sni = selector.get_next();

            let email = format!("{}-hysteria2", uuid_short);
            let tag = format!("HY2-{}-{}", i + 1, uuid_short);

            configs.push(Self::build_hysteria2_inbound(
                &tag,
                port,
                &auth,
                &email,
                &sni,
                ip_version,
            ));

            links.push(Self::generate_hysteria2_client_link(
                &auth,
                &host,
                port,
                &sni,
                &email,
                ip_version,
                &pin,
            ));

            let _ =
                crate::core::system::maintenance::MaintenanceManager::allow_port(port as u16).await;
        }

        Self::create_standalone_config(configs, links, super::config::Proto::Hysteria2).await
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2)'`
Expected: PASS (16 tests)

- [ ] **Step 5: Run the full crate build to confirm no regressions**

Run: `cargo build --offline -p aegis`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): batch-create hysteria2 inbounds with matching share links"
```

---

## Task 5: Add i18n keys

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml` (near line 314/330/352)
- Modify: `rust/aegis/src/resources/i18n/en.yml` (near line 294/303/361)
- Modify: `rust/aegis/src/resources/i18n/ja.yml` (near line 292/301/359)

**Interfaces:**
- Consumes: nothing.
- Produces: three translation keys per locale — `xray.batch_hysteria2`, `xray.filter_hysteria2`, `xray.type_hysteria2`.

- [ ] **Step 1: Add the keys**

In `rust/aegis/src/resources/i18n/zh.yml`, alongside the existing `filter_kcp` / `type_kcp` / `batch_kcp` keys, add:

```yaml
  batch_hysteria2: "🚀 Hysteria2 (Xray)"
  filter_hysteria2: "📡 HY2 (Xray)"
  type_hysteria2: "Hysteria2 (batch_xray_hysteria2)"
```

In `rust/aegis/src/resources/i18n/en.yml`:

```yaml
  batch_hysteria2: "🚀 Hysteria2 (Xray)"
  filter_hysteria2: "📡 HY2 (Xray)"
  type_hysteria2: "Hysteria2 (batch_xray_hysteria2)"
```

In `rust/aegis/src/resources/i18n/ja.yml`:

```yaml
  batch_hysteria2: "🚀 Hysteria2 (Xray)"
  filter_hysteria2: "📡 HY2 (Xray)"
  type_hysteria2: "Hysteria2 (batch_xray_hysteria2)"
```

Keep each key indented at the same level as the neighbouring `filter_kcp` key (two spaces under the `xray:` map).

- [ ] **Step 2: Verify the keys parse**

Run: `cargo nextest run --offline -p aegis -E 'test(i18n)'`
Expected: PASS. If locale files are validated by a test, this catches YAML/duplicate-key errors. If no such test exists, run `cargo build --offline -p aegis` and confirm no `t!` macro panic at compile time.

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml
git commit -m "i18n(xray): add hysteria2 batch/filter/type labels"
```

---

## Task 6: Wire the UI — filter, type label, dispatch, and batch button

**Files:**
- Modify: `rust/aegis/src/shared/handlers/xray.rs`

**Interfaces:**
- Consumes: `Proto::Hysteria2` (Task 1), `ConfigManager::batch_create_hysteria2_xray` (Task 4), i18n keys (Task 5).
- Produces: a working `cfg_filter:hysteria2` filter and a `u_xray_batch_init` path that creates Xray Hysteria2 configs.

**The delete/filter call sites (all inside `src/shared/handlers/xray.rs`):**

| Handler | Current `match` | Add arm |
|---|---|---|
| `handle_cfg_filter` (~712) | filter label | `"hysteria2" => t!("xray.filter_hysteria2")` |
| `handle_cfg_del_all_confirm` (~773) | type label | `"hysteria2" => t!("xray.type_hysteria2")` |
| `handle_cfg_del_all_exec` (~813) | filter → Proto | `"hysteria2" => Proto::Hysteria2` |
| `handle_cfg_del_count` (~858) | filter label | `"hysteria2" => t!("xray.filter_hysteria2")` |
| `handle_cfg_del_exec_count` (~917) | filter → Proto | `"hysteria2" => Proto::Hysteria2` |
| `handle_cfg_del_select` (~970, label ~980) | Proto + label | `"hysteria2" => Proto::Hysteria2` / `t!("xray.filter_hysteria2")` |
| `handle_cfg_del_file` (~1025) | filter → Proto | `"hysteria2" => Proto::Hysteria2` |
| `handle_cfg_del_confirm` (~1085) | filter → Proto | `"hysteria2" => Proto::Hysteria2` |
| `handle_cfg_filter` rows (~715) | filter buttons | add `cfg_filter:hysteria2` button |

### The verified batch flow (do NOT invent an encoding)

Ground truth, confirmed by reading the handlers. The shared batch flow is **prompt-chained**, not a single call:

```
button "u_batch_init"                → handle_batch_init          (hardcodes Proto::Vision)
button "u_xhttp_batch_init"          → handle_xhttp_batch_init    (hardcodes Proto::XHTTP)
         ↓ show_reality_batch_prompt(proto) builds ip_prefix from proto
button "<ip_prefix><4|6|s6|s4>"      → handle_batch_ip_init / handle_xhttp_batch_ip_init
         ↓ show_reality_qty_prompt(proto) builds exec_prefix from proto
button "<exec_prefix><ip>:<n>"       → handle_batch_exec / handle_xhttp_batch_exec
         ↓ match proto { ... } dispatch
```

The prefixes live in two `match proto` blocks inside `show_reality_batch_prompt` (~163) and `show_reality_qty_prompt` (~295):

```rust
// show_reality_batch_prompt
let (ip_prefix, title) = match proto {
    Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
    Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
    Proto::Kcp => unreachable!("KCP uses separate UI flow"),
};

// show_reality_qty_prompt
let (exec_prefix, title) = match proto {
    Proto::Vision => ("u_batch_exec:", "Reality"),
    Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
    Proto::Kcp => unreachable!("KCP uses separate UI flow"),
};
```

Because a `Proto::Hysteria2` needs its own prefix pair, it needs its own thin init + ip-init + exec handlers (each is ~10 lines, hardcoding `let proto = Proto::Hysteria2;` and a distinct prefix). This mirrors `handle_xhttp_batch_init` / `handle_xhttp_batch_ip_init` / `handle_xhttp_batch_exec` exactly. Routing uses `starts_with` guards, so prefixes must be mutually non-prefixing (`u_hy2_*` is safe: it is not a prefix of `u_kcp_*`, `u_batch_*`, or `u_xhttp_*`, and vice versa).

**Batch call sites (both dispatch handlers):**

| Site | Add arm |
|---|---|
| `handle_batch_exec` label (~1263) | `Proto::Hysteria2 => "Hysteria2"` |
| `handle_batch_exec` dispatch (~1283) | `Proto::Hysteria2 => ConfigManager::batch_create_hysteria2_xray(n, ip_version).await` |
| `handle_xhttp_batch_exec` label (~1441) | `Proto::Hysteria2 => "Hysteria2"` |
| `handle_xhttp_batch_exec` dispatch (~1461) | `Proto::Hysteria2 => ConfigManager::batch_create_hysteria2_xray(n, ip_version).await` |

- [ ] **Step 1: Add the filter / type / proto arms**

> **Revised during execution (Task 1 finding).** Task 1 left **6 placeholder arms** in this file,
> each tagged `// TODO(Task 6):` and each currently `unreachable!("Hysteria2 handler wiring lands
> in Task 6")`. They are at approximately `show_reality_batch_prompt` (~162),
> `show_reality_qty_prompt` (~294), `handle_batch_exec` label (~1260) and dispatch (~1278),
> `handle_xhttp_batch_exec` label (~1438) and dispatch (~1456). Line numbers will have shifted
> slightly once Task 1's placeholders are in — locate them by searching for the string
> `Hysteria2 handler wiring lands in Task 6` and for `TODO(Task 6)`. **Every one of these six
> placeholders must be replaced by this task.** None may remain in the final tree: an unreplaced
> placeholder is a runtime `unreachable!()` panic on the Hysteria2 path.
>
> Verification step for the end of this task: `rg -n "Hysteria2 handler wiring lands in Task 6"`
> must return **zero** matches.

In every `match` listed above, add the `"hysteria2"` arm **before** the `_ =>` fallback. For the two `Proto`-returning helpers, the arm is `"hysteria2" => Proto::Hysteria2,`. For the label helpers, it is `"hysteria2" => t!("xray.filter_hysteria2"),` (filters) or `"hysteria2" => t!("xray.type_hysteria2"),` (the `del_all_confirm` type label).

For the two `match proto { ... }` sites that carry `Proto::Kcp => unreachable!("KCP uses separate batch handler")`, add:

```rust
        Proto::Hysteria2 => "Hysteria2",
```

for the label sites, and:

```rust
        Proto::Hysteria2 => ConfigManager::batch_create_hysteria2_xray(n, ip_version).await,
```

for the dispatch sites — placed before the `Proto::Kcp => unreachable!(...)` arm, since Hysteria2 uses the shared flow (unlike KCP).

- [ ] **Step 2: Add the batch flow (prefixes, handlers, routes, button)**

**2a.** In `show_reality_batch_prompt` (~163), add the Hysteria2 arm:

```rust
    let (ip_prefix, title) = match proto {
        Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
        Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
        Proto::Hysteria2 => ("u_hy2_batch_ip_init:", "Hysteria2 (Xray)"),
    };
```

Also confirm whether the `meta` struct / split-stack button block below the `match` treats `Proto::XHTTP` specially (it does, for split-stack IPv6 upload). Hysteria2 in core scope needs only IPv4/IPv6, so leave that XHTTP-only branch untouched.

**2b.** In `show_reality_qty_prompt` (~295), add the arm:

```rust
    let (exec_prefix, title) = match proto {
        Proto::Vision => ("u_batch_exec:", "Reality"),
        Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
        Proto::Kcp => unreachable!("KCP uses separate UI flow"),
        Proto::Hysteria2 => ("u_hy2_batch_exec:", "Hysteria2"),
    };
```

**2c.** Add three handlers, modelled line-for-line on the XHTTP trio. Place them directly after `handle_xhttp_batch_exec`:

```rust
async fn handle_hy2_batch_init(event: &CallbackEvent) -> HandlerResult {
    if MaintenanceManager::is_reality_base_ready().await {
        show_reality_batch_prompt(
            &*event.adapter,
            &event.target,
            &event.msg_id,
            Proto::Hysteria2,
        )
        .await?;
    } else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.preparing_reality").into_owned()),
            )
            .await?;
        event
            .adapter
            .edit_message(
                &event.target,
                &event.msg_id,
                MessageContent {
                    text: t!("xray.init_reality").into_owned(),
                    markup: None,
                },
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
    }
    Ok(HandlerAction::Done)
}

async fn handle_hy2_batch_ip_init(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_hy2_batch_ip_init:";
    let proto = Proto::Hysteria2;
    let ip_ver_code = data.strip_prefix(prefix).unwrap_or("");
    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };
    show_reality_qty_prompt(
        &*event.adapter,
        &event.target,
        &event.msg_id,
        ip_version,
        proto,
    )
    .await?;
    Ok(HandlerAction::Done)
}

async fn handle_hy2_batch_exec(event: &CallbackEvent) -> HandlerResult {
    let data = event.data.as_str();
    let prefix = "u_hy2_batch_exec:";
    let proto = Proto::Hysteria2;
    let parts: Vec<&str> = data
        .strip_prefix(prefix)
        .unwrap_or(data)
        .split(':')
        .collect();
    if parts.len() != 2 {
        return Ok(HandlerAction::Done);
    }
    let ip_ver_code = parts[0];
    let n: usize = parts[1].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        "s6" => IpVersion::SplitStackV6Primary,
        "s4" => IpVersion::SplitStackV4Primary,
        _ => IpVersion::IPv4,
    };

    if !MaintenanceManager::is_reality_base_ready().await {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("xray.base_missing").into_owned()),
            )
            .await?;
        trigger_reality_auto_init(
            event.adapter.clone(),
            event.target.clone(),
            event.msg_id.clone(),
        );
        return Ok(HandlerAction::Done);
    }

    let ip_str: String = match ip_version {
        IpVersion::IPv4 => "IPv4".into(),
        IpVersion::IPv6 => "IPv6".into(),
        IpVersion::SplitStackV6Primary => t!("xray.split_v6_up").into(),
        IpVersion::SplitStackV4Primary => t!("xray.split_v4_up").into(),
    };

    event
        .adapter
        .answer_callback(
            &event.target,
            &event.callback_id,
            Some(
                t!("xray.gen_progress", "0" => n, "1" => "Hysteria2", "2" => ip_str.as_str())
                    .into_owned(),
            ),
        )
        .await?;

    let adapter = event.adapter.clone();
    let target = event.target.clone();

    match ConfigManager::batch_create_hysteria2_xray(n, ip_version).await {
        Ok(result) => {
            let mut combined_links = String::new();
            for link in &result.links {
                combined_links.push_str(link);
                combined_links.push_str("\n\n");
            }
            if !combined_links.is_empty() {
                let _ = adapter
                    .send_message(
                        &target,
                        MessageContent {
                            text: combined_links,
                            markup: None,
                        },
                    )
                    .await;
            }
        }
        Err(e) => {
            let _ = adapter
                .send_message(
                    &target,
                    MessageContent {
                        text: t!("xray.gen_fail", "0" => e.to_string()).into_owned(),
                        markup: None,
                    },
                )
                .await;
        }
    }

    Ok(HandlerAction::Redirect("m_xray_mgmt".to_string()))
}
```

**Important:** `handle_xhttp_batch_exec` builds its message (including any backup/restore notice) differently from `handle_batch_exec`. Before writing `handle_hy2_batch_exec`, read `handle_xhttp_batch_exec` in full and mirror its actual result-message and redirect behaviour, substituting only the function name and proto label. The code above shows the minimum; if `handle_xhttp_batch_exec` sends a header/summary or auto-deletes, replicate that so Hysteria2 behaves consistently with its sibling proto. Do not copy `handle_batch_exec`'s Vision-specific Reality-base gating verbatim without checking that XHTTP does the same.

**Edit hazard (found by the Task 1 implementer — read before editing):** `handle_batch_exec` and `handle_xhttp_batch_exec` contain **textually identical** `proto_str` and `res` match blocks. A naive `edit` with `oldText` copied from one of those blocks will match in BOTH functions, or fail as non-unique. To edit each safely, anchor `oldText` on something unique to that function — either the function signature line (`async fn handle_batch_exec(` / `async fn handle_xhttp_batch_exec(`) or the preceding `let prefix = "u_batch_exec:"` / `let prefix = "u_xhttp_batch_exec:"` line — and include enough following context to disambiguate. Verify after each edit that the other function was not touched: `rg -n "Proto::Hysteria2" rust/aegis/src/shared/handlers/xray.rs` should show the arms in the intended places only.

Similarly, the two proto-label helpers at ~1260 and ~1438 and the two dispatch blocks at ~1278 and ~1456 are near-identical. Treat all four as a single coordinated edit rather than four independent ones.

**2d.** Register the three routes in the dispatcher, next to the XHTTP routes (~2511):

```rust
        "u_hy2_batch_init" => handle_hy2_batch_init(event).await,
        d if d.starts_with("u_hy2_batch_ip_init:") => handle_hy2_batch_ip_init(event).await,
        d if d.starts_with("u_hy2_batch_exec:") => handle_hy2_batch_exec(event).await,
```

**2e.** Add the menu button in the Xray management menu builder (~440), next to the KCP button:

```rust
            InlineButton {
                text: t!("xray.batch_hysteria2").into(),
                data: "u_hy2_batch_init".into(),
            },
```

- [ ] **Step 3: Add the filter button**

In `handle_cfg_filter`'s `rows` (around line 715), add a Hysteria2 filter button next to the KCP one:

```rust
            InlineButton {
                text: t!("xray.filter_hysteria2").into(),
                data: "cfg_filter:hysteria2".into(),
            },
```

- [ ] **Step 4: Build to catch non-exhaustive matches**

Run: `cargo build --offline -p aegis 2>&1 | grep -E "^error|not covered" | head -30`
Expected: no errors. Any remaining error names a `match proto` we missed.

- [ ] **Step 5: Run the full test suite**

Run: `cargo nextest run --offline -p aegis`
Expected: all tests pass (baseline 749 + 18 new = 767)

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/shared/handlers/xray.rs
git commit -m "feat(xray): wire hysteria2 filter, delete paths, and batch UI"
```

---

## Task 7: Quality gates and end-to-end verification

**Files:** none created; verification only.

**Interfaces:**
- Consumes: everything from Tasks 1-6.
- Produces: a verified, lint-clean branch.

- [ ] **Step 1: Run rustfmt**

Run: `cargo fmt --all --check`
Expected: no output (clean). If it reports diffs, run `cargo fmt --all` and re-check.

- [ ] **Step 2: Run clippy with warnings denied**

Run: `cargo clippy --offline --all-targets --all-features -- -D warnings`
Expected: no warnings, no errors.

- [ ] **Step 3: Run the full test suite via nextest**

Run: `cargo nextest run --offline`
Expected: all tests pass, 0 failures.

- [ ] **Step 4: Verify the generated inbound against the Xray reference config**

Write a focused test that serialises a built inbound and asserts it round-trips through `serde_json`, then manually compare the shape against the reference at `https://github.com/XTLS/Xray-examples/blob/main/Hysteria2/server.jsonc`:

Reference shape (Xray-examples):
```jsonc
{
  "protocol": "hysteria",
  "settings": { "version": 2 },
  "streamSettings": {
    "network": "hysteria",
    "security": "tls",
    "tlsSettings": { "alpn": ["h3"], "certificates": [...] },
    "hysteriaSettings": { "version": 2, "auth": "..." }
  }
}
```

Our shape adds `settings.users` (Xray's documented user-object array, which the reference omits because it relies on the transport-level `auth` alone). Both are documented as valid; we set both so that `auth` is consistent regardless of which layer the client keys off. Confirm in review that this is intentional and note it in the commit message.

- [ ] **Step 5: Bump the version (project convention)**

The repo versions via `rust/version-sync` and ships a version bump commit per release (see git log: `1.3.10`, `1.3.9`). Follow the existing convention — if the project requires a version bump for features, update `rust/aegis/Cargo.toml` and run the version-sync tool. Confirm with the user before bumping; do not bump unprompted.

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "chore(xray): verify hysteria2 against Xray reference config"
```

---

## Self-Review

**1. Spec coverage**

| Design requirement | Task |
|---|---|
| `Proto::Hysteria2` + 5 match sites (6 match expressions) | Task 1 |
| 6 handler placeholder arms replaced with real wiring | Task 6 (verified by the `rg` check in Task 6 Step 1) |
| Xray `"protocol": "hysteria"` (not `hysteria2`) | Task 2 |
| `settings.users[].auth` (not `password`) | Task 2 |
| Dual protocol + transport layers | Task 2 |
| `alpn: ["h3"]` explicit | Task 2 |
| Shared TLS cert reuse | Task 2 |
| `hysteria2://` link, auth == inbound auth | Task 3 |
| `pinSHA256`, no `insecure=1` | Task 3 |
| Batch flow, port allocation, cert pin | Task 4 |
| i18n keys (zh/en/ja) | Task 5 |
| UI filter / type / dispatch / button | Task 6 |
| No sing-box changes | All tasks (no sing-box file appears in any Files block) |
| Distinct file prefix (no collision with sing-box) | Task 1 |
| Quality gates | Task 7 |

No gaps. Out-of-scope items (obfs/salamander, `udpHop` hopping) are explicitly excluded per the approved (B) design.

**2. Placeholder scan**

Task 6 was **revised after verification**. The first draft assumed the shared `u_batch_init` button could be reused with a bare callback. Reading the handlers disproved that: `handle_batch_init` hardcodes `Proto::Vision`, and each proto owns a prefix pair plus thin init/ip-init/exec handlers. Task 6 now specifies the verified flow, the three new handlers with full code, the route arms, and the exact prefix names. One residual conditional remains in Step 2c (mirror `handle_xhttp_batch_exec`'s result-message behaviour rather than inventing it) — that is a real read-then-match instruction, not a placeholder.

**4. Prefix-collision check (verified)**

The dispatcher uses `starts_with` guards. Existing prefixes: `u_batch_ip_init:`, `u_batch_exec:`, `u_xhttp_batch_ip_init:`, `u_xhttp_batch_exec:`, `u_kcp_*` (nine of them), `u_l:`, `u_d:`. New prefixes `u_hy2_batch_ip_init:` / `u_hy2_batch_exec:` share no prefix relation with any of them. Note `u_l:` and `u_d:` are short but distinct (`u_hy2...` does not start with `u_l:` or `u_d:`). No collision.

**5. i18n key check (verified)**

All keys referenced by Task 6 already exist in zh/en/ja: `xray.gen_progress`, `xray.gen_fail`, `xray.base_missing`, `xray.init_reality`, `xray.preparing_reality`, `xray.split_v6_up`, `xray.split_v4_up`. Task 5's three new keys (`batch_hysteria2`, `filter_hysteria2`, `type_hysteria2`) are the only additions.

**3. Type consistency**

- `ConfigManager::batch_file_prefix(Proto) -> &'static str` — added in Task 1 fix round 1; used by
  `list_inbound_files_by_proto` and `generate_secure_batch_filename`. Tested directly.
- `build_hysteria2_inbound(tag: &str, port: i32, auth: &str, email: &str, sni: &str, ip_version: IpVersion) -> Value` — used identically in Task 2 tests, Task 4 batch.
- `generate_hysteria2_client_link(auth: &str, host: &str, port: i32, sni: &str, email: &str, ip_version: IpVersion, pin: &str) -> String` — used identically in Task 3 tests, Task 4 batch.
- `batch_create_hysteria2_xray(count: usize, ip_version: IpVersion) -> Result<BatchCreationResult>` — Task 4 defines, Task 6 calls with `(n, ip_version)`.
- `Proto::Hysteria2` — Task 1 defines, Tasks 4 and 6 use.
- `singbox::TLS_CERT` — string constant, used in Tasks 2 and 4.
- Test count: Task 1 (2) + Task 2 (8) + Task 3 (6) + Task 4 (2) = 18 new tests; 749 + 18 = 767.
