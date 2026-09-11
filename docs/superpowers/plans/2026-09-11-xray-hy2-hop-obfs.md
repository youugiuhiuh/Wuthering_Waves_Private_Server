# Xray Hysteria2: Port Hopping + Obfuscation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add QUIC obfuscation (salamander/gecko) and port hopping to the Xray Hysteria2 support that already exists on `main`, matching the feature set sing-box Hy2 already offers.

**Architecture:** Extend the existing `core/xray/hysteria2.rs` inbound builder and link generator with two optional capabilities written into `streamSettings.finalmask`. Add a **separate** Xray-scoped port allocator so Xray and sing-box hopping ranges never share a resource pool or a counter. Add hop-range cleanup to the Xray delete path, which currently does plain file removal. Wire the UI by mirroring sing-box's existing obfs/hop selection flow.

**Tech Stack:** Rust (edition 2024), `serde_json`, `percent-encoding`, `anyhow`, `tokio`, `rand` 0.8. Tests via `cargo nextest`.

**Spec:** Design approved in session 2026-09-11 (brainstorming, architectural path). Binding external authorities:
- FinalMask / quicParams / salamander: https://xtls.github.io/config/transports/finalmask.html
- Hysteria transport: https://xtls.github.io/config/transports/hysteria.html
- Hysteria2 URI scheme: https://v2.hysteria.network/docs/developers/URI-Scheme/

## Global Constraints

- **Xray has NO `gecko` obfs type.** Obfuscation is always `finalmask.udp[0].type = "salamander"`. Gecko is enabled by adding `settings.packetSize` (e.g. `"512-1200"`); omitting `packetSize` gives plain salamander. In the **client link** the distinction is written as `obfs=salamander` vs `obfs=gecko` (client-side vocabulary) — the inbound schema and the link vocabulary deliberately differ.
- **Port hopping goes in `streamSettings.finalmask.quicParams.udpHop = {"ports": "A-B", "interval": 30}`.** This is the transport layer. It is NOT the same as sing-box's client-side `mport`.
- **Reuse the client-link formats from sing-box's `Hy2LinkStyle`** (approved decision A):
  - `Official` → `hysteria2://auth@host:MAIN,A-B?…&hop_interval=30s#name`
  - `V2rayN` → `hysteria2://auth@host:MAIN?…&mport=A-B#name`
- **Reuse `Hysteria2ObfsType` and the gecko packet-size constants** from `core/singbox/hysteria2.rs` (`GECKO_DEFAULT_MIN_PACKET_SIZE = 512`, `GECKO_DEFAULT_MAX_PACKET_SIZE = 1200`). Do not define duplicates.
- **Xray hop ranges use a SEPARATE allocator and a SEPARATE protocol tag** (`"xray-hysteria2"`), written to the same `.port_alloc` file. Xray must NOT call `allocate_hysteria2` / `release_hysteria2_range` (those are sing-box's, tagged `"hysteria2"`). Approved decision A — the two cores must not share a quota or a counter.
- **`PortAllocator::check_hysteria2_limit()` must NOT be called by Xray** — it scans the sing-box conf dir. Task 4 already replaced it with `check_xray_hysteria2_capacity`. Do not reintroduce it.
- **The delete path must release the hop range.** `delete_specific_configuration` is currently a plain `remove_file`. If a hopping config is deleted without releasing, the range stays locked forever and accumulates. This exact bug class has been fixed before in sing-box — do not recreate it.
- **Do NOT modify any sing-box source file.** Reading/reusing its public items is fine.
- **Do not change behaviour for Vision / XHTTP / KCP.** The delete-path change must affect only Hysteria2 configs.
- **Quality gates (rust-lint-format skill, offline variant verified in this repo):**
  ```
  cargo fmt --all --check
  cargo clippy --offline --all-targets --all-features -- -D warnings
  cargo nextest run --offline
  cargo test --doc --offline
  ```
- **TDD is mandatory** (strict mode). RED → verify failure → GREEN → verify pass → REFACTOR. No production code without a failing test first.
- **Every new assertion must be falsifiable.** This plan's predecessor shipped two tautological tests; reviewers now check for them. Do not write an assertion that passes for any input.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `rust/aegis/src/core/xray/hysteria2.rs` | Obfs + hop in inbound JSON; link variants; batch params | Modify |
| `rust/aegis/src/core/xray/port_allocator.rs` | Xray-scoped hop-range alloc/release/query | Modify |
| `rust/aegis/src/core/xray/config.rs` | Hop cleanup on delete | Modify |
| `rust/aegis/src/shared/handlers/xray.rs` | Obfs/hop UI flow, callback encoding | Modify |
| `rust/aegis/src/resources/i18n/{zh,en,ja}.yml` | UI labels | Modify |

---

## Task 1: Add obfuscation to the Xray Hysteria2 inbound

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs`
- Test: inline `#[cfg(test)] mod tests` in the same file

**Interfaces:**
- Consumes: `crate::core::singbox::hysteria2::{Hysteria2ObfsType, GECKO_DEFAULT_MIN_PACKET_SIZE, GECKO_DEFAULT_MAX_PACKET_SIZE}` (all `pub`, already exist).
- Produces: `build_hysteria2_inbound(tag, port, auth, email, sni, ip_version, obfs: Option<(&Hysteria2ObfsType, &str)>) -> Value` — a 7th parameter. Callers in Task 3 update to match.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `rust/aegis/src/core/xray/hysteria2.rs`. Note the existing `build` helper calls the 6-arg form — update it to pass `None` for obfs so existing tests keep compiling:

```rust
    #[test]
    fn test_hysteria2_obfs_salamander_writes_finalmask_udp() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Salamander, "obfspw")),
        );
        let udp = &cfg["streamSettings"]["finalmask"]["udp"][0];
        assert_eq!(udp["type"], "salamander");
        assert_eq!(udp["settings"]["password"], "obfspw");
        // Plain salamander must NOT carry packetSize; that is what makes it
        // gecko. A stray packetSize here would silently enable gecko.
        assert!(udp["settings"].get("packetSize").is_none());
    }

    #[test]
    fn test_hysteria2_obfs_gecko_is_salamander_plus_packetsize() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Gecko, "obfspw")),
        );
        let udp = &cfg["streamSettings"]["finalmask"]["udp"][0];
        // Xray has no "gecko" type: gecko IS salamander with a packetSize.
        assert_eq!(udp["type"], "salamander");
        assert_ne!(udp["type"], "gecko");
        assert_eq!(udp["settings"]["password"], "obfspw");
        assert_eq!(
            udp["settings"]["packetSize"],
            format!(
                "{}-{}",
                GECKO_DEFAULT_MIN_PACKET_SIZE, GECKO_DEFAULT_MAX_PACKET_SIZE
            )
        );
    }

    #[test]
    fn test_hysteria2_no_obfs_omits_finalmask_entirely() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
        );
        // Absent, not an empty object: an empty finalmask is not the same
        // config to Xray and would be a schema surprise.
        assert!(cfg["streamSettings"].get("finalmask").is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_obfs) or test(test_hysteria2_no_obfs)'`
Expected: FAIL to compile — `this function takes 6 arguments but 7 arguments were supplied`

- [ ] **Step 3: Add the parameter and the finalmask block**

In `rust/aegis/src/core/xray/hysteria2.rs`, extend the imports:

```rust
use crate::core::singbox::hysteria2::{
    GECKO_DEFAULT_MAX_PACKET_SIZE, GECKO_DEFAULT_MIN_PACKET_SIZE, Hysteria2ObfsType,
};
```

Change the signature to add the 7th parameter:

```rust
    /// `obfs` carries (type, password). Xray has no separate gecko type: gecko
    /// is salamander plus a `packetSize`, which switches on extra QUIC
    /// long-header fragmentation. Password must match between client and server.
    pub(crate) fn build_hysteria2_inbound(
        tag: &str,
        port: i32,
        auth: &str,
        email: &str,
        sni: &str,
        ip_version: IpVersion,
        obfs: Option<(&Hysteria2ObfsType, &str)>,
    ) -> Value {
```

Build the value, then attach finalmask before returning. Replace the trailing `json!({...})` expression with a binding plus mutation:

```rust
        let mut inbound = json!({
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
        });

        if let Some((obfs_type, obfs_password)) = obfs {
            let udp_mask = Self::build_hysteria2_udp_mask(obfs_type, obfs_password);
            inbound["streamSettings"]["finalmask"] = json!({ "udp": [udp_mask] });
        }

        inbound
    }

    /// Build one UDPMask entry for Hysteria2 obfuscation.
    ///
    /// Xray exposes only the `salamander` type; gecko is chosen by supplying
    /// `packetSize`, which enables extra fragmentation/padding of QUIC
    /// long-header packets. Upper bound must not exceed 2048 per the Xray docs.
    pub(crate) fn build_hysteria2_udp_mask(obfs_type: &Hysteria2ObfsType, password: &str) -> Value {
        let mut settings = serde_json::Map::new();
        settings.insert("password".to_string(), json!(password));
        if *obfs_type == Hysteria2ObfsType::Gecko {
            settings.insert(
                "packetSize".to_string(),
                json!(format!(
                    "{}-{}",
                    GECKO_DEFAULT_MIN_PACKET_SIZE, GECKO_DEFAULT_MAX_PACKET_SIZE
                )),
            );
        }
        json!({
            "type": "salamander",
            "settings": settings
        })
    }
```

- [ ] **Step 4: Update the existing `build` test helper and any other 6-arg callers**

In the same file's `tests` module, the `build(ip_version)` helper currently calls the 6-arg form. Add `None` as the 7th argument. `cargo build` will name any other call site.

Run: `cargo build --offline -p aegis 2>&1 | grep -A2 "this function takes"`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2)'`
Expected: PASS. All pre-existing hysteria2 tests still pass (they now pass `None`).

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): add hysteria2 QUIC obfuscation to the inbound builder"
```

---

## Task 2: Add port hopping to the Xray Hysteria2 inbound

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs`
- Test: inline tests

**Interfaces:**
- Consumes: `build_hysteria2_inbound` (Task 1) and `build_hysteria2_udp_mask` (Task 1).
- Produces: `build_hysteria2_inbound(tag, port, auth, email, sni, ip_version, obfs, hop_range: Option<(u16, u16)>) -> Value` — an 8th parameter.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn test_hysteria2_hop_writes_quicparams_udphop() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            Some((11452, 11551)),
        );
        let hop = &cfg["streamSettings"]["finalmask"]["quicParams"]["udpHop"];
        assert_eq!(hop["ports"], "11452-11551");
        assert_eq!(hop["interval"], 30);
    }

    #[test]
    fn test_hysteria2_hop_and_obfs_coexist_in_one_finalmask() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            Some((&Hysteria2ObfsType::Gecko, "pw")),
            Some((11452, 11551)),
        );
        let fm = &cfg["streamSettings"]["finalmask"];
        // Both keys must live under the SAME finalmask object. Writing either
        // one by replacing `finalmask` wholesale would drop the other.
        assert_eq!(fm["udp"][0]["type"], "salamander");
        assert_eq!(fm["quicParams"]["udpHop"]["ports"], "11452-11551");
    }

    #[test]
    fn test_hysteria2_no_hop_omits_quicparams() {
        let cfg = ConfigManager::build_hysteria2_inbound(
            "t",
            11451,
            AUTH,
            "e",
            "www.bing.com",
            IpVersion::IPv4,
            None,
            None,
        );
        assert!(cfg["streamSettings"].get("finalmask").is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_hop)'`
Expected: FAIL to compile — 8 arguments supplied, 7 expected

- [ ] **Step 3: Implement**

Change the signature to add `hop_range: Option<(u16, u16)>`. Replace the obfs-only block with one that builds a single `finalmask` object and inserts it once:

```rust
        // Both features live under one `finalmask` object. Build it once and
        // insert once: assigning `finalmask` per-feature would replace the
        // whole object and silently drop the earlier feature.
        let mut finalmask = serde_json::Map::new();
        if let Some((obfs_type, obfs_password)) = obfs {
            finalmask.insert(
                "udp".to_string(),
                json!([Self::build_hysteria2_udp_mask(obfs_type, obfs_password)]),
            );
        }
        if let Some((hop_start, hop_end)) = hop_range {
            finalmask.insert(
                "quicParams".to_string(),
                json!({
                    "udpHop": {
                        "ports": format!("{}-{}", hop_start, hop_end),
                        "interval": 30
                    }
                }),
            );
        }
        if !finalmask.is_empty() {
            inbound["streamSettings"]["finalmask"] = Value::Object(finalmask);
        }

        inbound
```

Keep `build_hysteria2_udp_mask` as-is from Task 1.

- [ ] **Step 4: Update all callers to pass the 8th argument**

Run: `cargo build --offline -p aegis 2>&1 | grep -A2 "this function takes"`
Update each named site (including tests) with the appropriate value.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2)'`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): add hysteria2 port hopping to the inbound builder"
```

---

## Task 3: Extend the client link for obfs and hopping

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs`
- Test: inline tests

**Interfaces:**
- Consumes: `Hysteria2ObfsType`, `Hy2LinkStyle` (both in `core::singbox::hysteria2`).
- Produces: `generate_hysteria2_client_link(auth, host, port, sni, email, ip_version, pin, obfs: Option<(&Hysteria2ObfsType, &str)>, hop_range: Option<(u16, u16)>, link_style: Hy2LinkStyle) -> String`

- [ ] **Step 1: Write the failing tests**

```rust
    fn link_with(
        obfs: Option<(&Hysteria2ObfsType, &str)>,
        hop: Option<(u16, u16)>,
        style: Hy2LinkStyle,
    ) -> String {
        ConfigManager::generate_hysteria2_client_link(
            "pw",
            "203.0.113.1",
            11451,
            "www.bing.com",
            "n",
            IpVersion::IPv4,
            "AA:BB",
            obfs,
            hop,
            style,
        )
    }

    #[test]
    fn test_hysteria2_link_obfs_salamander() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Salamander, "obfspw")),
            None,
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs=salamander"));
        assert!(link.contains("obfs-password=obfspw"));
    }

    #[test]
    fn test_hysteria2_link_gecko_is_labelled_gecko_not_salamander() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Gecko, "obfspw")),
            None,
            Hy2LinkStyle::Official,
        );
        // The client vocabulary keeps the gecko distinction even though the
        // inbound schema expresses it as salamander+packetSize.
        assert!(link.contains("obfs=gecko"));
        assert!(!link.contains("obfs=salamander"));
    }

    #[test]
    fn test_hysteria2_link_hopping_official_puts_range_in_port_position() {
        let link = link_with(None, Some((11452, 11551)), Hy2LinkStyle::Official);
        assert!(link.contains("@203.0.113.1:11451,11452-11551?"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("mport="));
    }

    #[test]
    fn test_hysteria2_link_hopping_v2rayn_uses_mport() {
        let link = link_with(None, Some((11452, 11551)), Hy2LinkStyle::V2rayN);
        assert!(link.contains("@203.0.113.1:11451?"));
        assert!(link.contains("mport=11452-11551"));
        assert!(!link.contains("hop_interval"));
    }

    #[test]
    fn test_hysteria2_link_hop_and_obfs_and_pin_all_present() {
        let link = link_with(
            Some((&Hysteria2ObfsType::Gecko, "opw")),
            Some((11452, 11551)),
            Hy2LinkStyle::Official,
        );
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=opw"));
        assert!(link.contains("11451,11452-11551"));
        assert!(link.contains("pinSHA256=AA%3ABB"));
        // The security posture must not regress when features are combined.
        assert!(!link.contains("insecure=1"));
    }

    #[test]
    fn test_hysteria2_link_plain_has_no_obfs_or_hop_params() {
        let link = link_with(None, None, Hy2LinkStyle::Official);
        assert!(!link.contains("obfs="));
        assert!(!link.contains("mport="));
        assert!(!link.contains("hop_interval"));
        assert!(!link.contains(",11452"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2_link_obfs) or test(test_hysteria2_link_gecko) or test(test_hysteria2_link_hopping) or test(test_hysteria2_link_hop_and_obfs) or test(test_hysteria2_link_plain)'`
Expected: FAIL to compile — argument count mismatch

- [ ] **Step 3: Implement**

```rust
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn generate_hysteria2_client_link(
        auth: &str,
        host: &str,
        port: i32,
        sni: &str,
        email: &str,
        ip_version: IpVersion,
        pin: &str,
        obfs: Option<(&Hysteria2ObfsType, &str)>,
        hop_range: Option<(u16, u16)>,
        link_style: Hy2LinkStyle,
    ) -> String {
        let encoded_auth = utf8_percent_encode(auth, NON_ALPHANUMERIC).to_string();
        let encoded_sni = utf8_percent_encode(sni, NON_ALPHANUMERIC).to_string();
        let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();
        let encoded_pin = utf8_percent_encode(pin, NON_ALPHANUMERIC).to_string();

        let fmt_host = match ip_version {
            IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
            IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
        };

        // Official puts the hop range in the port position; v2rayN uses mport.
        let (authority, hop_param) = match (hop_range, link_style) {
            (Some((a, b)), Hy2LinkStyle::Official) => (
                format!("{}:{},{}-{}", fmt_host, port, a, b),
                "&hop_interval=30s".to_string(),
            ),
            (Some((a, b)), Hy2LinkStyle::V2rayN) => (
                format!("{}:{}", fmt_host, port),
                format!("&mport={}-{}", a, b),
            ),
            (None, _) => (format!("{}:{}", fmt_host, port), String::new()),
        };

        let obfs_param = obfs
            .map(|(t, pw)| {
                format!(
                    "&obfs={}&obfs-password={}",
                    t.as_str(),
                    utf8_percent_encode(pw, NON_ALPHANUMERIC)
                )
            })
            .unwrap_or_default();

        format!(
            "hysteria2://{}@{}?sni={}&alpn=h3&pinSHA256={}{}{}#{}",
            encoded_auth, authority, encoded_sni, encoded_pin, hop_param, obfs_param, encoded_email
        )
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hysteria2)'`
Expected: PASS. Note the pre-existing link tests assert the old format — if any asserted `:11451?` before the query string, update them to the new authority form and say so in the commit body.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs
git commit -m "feat(xray): add obfs and hop variants to the hysteria2 client link"
```

---

## Task 4: Add the Xray-scoped hop-range allocator

**Files:**
- Modify: `rust/aegis/src/core/xray/port_allocator.rs`
- Test: inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `LockedRange`, `HOP_SIZE`, `load_port_alloc_at`, `save_port_alloc_at`, `scan_all_occupied_ports`, `find_consecutive_range`, `PORT_ALLOC_FILE` (all already in this file).
- Produces (all parallel to the existing sing-box-tagged functions, differing only in the `protocol` tag):
  - `const XRAY_HY2_PROTOCOL: &str = "xray-hysteria2";`
  - `pub async fn allocate_xray_hysteria2() -> Result<(u16, (u16, u16))>`
  - `pub async fn allocate_xray_hysteria2_at(path: &Path) -> Result<(u16, (u16, u16))>`
  - `pub async fn release_xray_hysteria2_range(main_port: u16) -> Result<()>`
  - `pub async fn release_xray_hysteria2_range_at(path: &Path, main_port: u16) -> Result<()>`
  - `pub async fn get_xray_hysteria2_ranges() -> Vec<(u16, (u16, u16))>`

- [ ] **Step 1: Write the failing tests**

The tests must prove **isolation** from sing-box's ranges, which is the whole point of the separate tag:

```rust
    #[tokio::test]
    async fn test_xray_alloc_uses_own_protocol_tag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".port_alloc");
        let (main, hop) = PortAllocator::allocate_xray_hysteria2_at(&path)
            .await
            .unwrap();
        assert_eq!(hop, (main + 1, main + 99));
        let data: PortAllocData =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data.locked_ranges.len(), 1);
        // Must be the Xray tag, not sing-box's "hysteria2".
        assert_eq!(data.locked_ranges[0].protocol, "xray-hysteria2");
        assert_ne!(data.locked_ranges[0].protocol, "hysteria2");
    }

    #[tokio::test]
    async fn test_xray_release_does_not_touch_singbox_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".port_alloc");
        // Seed a sing-box-tagged range with the SAME main port.
        let data = PortAllocData {
            locked_ranges: vec![LockedRange {
                start: 20000,
                end: 20099,
                protocol: "hysteria2".to_string(),
                created_at: 0,
            }],
            initialized: true,
        };
        save_port_alloc_at(&path, &data).await.unwrap();

        // Releasing an Xray range at that port must be a no-op on the sing-box
        // range: sharing a port number across cores must not cross-release.
        PortAllocator::release_xray_hysteria2_range_at(&path, 20000)
            .await
            .unwrap();
        let after: PortAllocData =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after.locked_ranges.len(), 1);
        assert_eq!(after.locked_ranges[0].protocol, "hysteria2");
    }

    #[tokio::test]
    async fn test_xray_release_removes_only_its_own_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".port_alloc");
        let (main, _) = PortAllocator::allocate_xray_hysteria2_at(&path)
            .await
            .unwrap();
        let before: PortAllocData =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(before.locked_ranges.len(), 1);

        PortAllocator::release_xray_hysteria2_range_at(&path, main)
            .await
            .unwrap();
        let after: PortAllocData =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(after.locked_ranges.is_empty());
    }

    #[tokio::test]
    async fn test_xray_alloc_avoids_singbox_locked_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".port_alloc");
        // sing-box already holds the lowest 100 ports.
        let data = PortAllocData {
            locked_ranges: vec![LockedRange {
                start: XRAY_PORT_MIN,
                end: XRAY_PORT_MIN + 99,
                protocol: "hysteria2".to_string(),
                created_at: 0,
            }],
            initialized: true,
        };
        save_port_alloc_at(&path, &data).await.unwrap();

        let (main, _) = PortAllocator::allocate_xray_hysteria2_at(&path)
            .await
            .unwrap();
        // Must not overlap another core's locked range.
        assert!(
            main > XRAY_PORT_MIN + 99,
            "xray allocator returned {main}, overlapping the sing-box range"
        );
    }
```

Add `tempfile` to `[dev-dependencies]` if it is not already present — check with `grep -n tempfile Cargo.toml` first; if absent, use `cargo add --dev tempfile --offline` (per the dependency-management skill, never hand-edit the dependency file). Other tests in this file already use tempdirs, so it is likely present.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --offline -p aegis -E 'test(test_xray_alloc) or test(test_xray_release)'`
Expected: FAIL to compile — `no function or associated item named 'allocate_xray_hysteria2_at'`

- [ ] **Step 3: Implement**

Add the tag constant near the other consts:

```rust
/// Protocol tag for Xray Hysteria2 hop ranges in `.port_alloc`.
///
/// Deliberately distinct from sing-box's `"hysteria2"`: the two cores keep
/// separate ranges and separate quotas, so releasing one must never release
/// the other even if the port numbers coincide.
const XRAY_HY2_PROTOCOL: &str = "xray-hysteria2";
```

Add the methods inside `impl PortAllocator`, modelled on the sing-box pair but using `XRAY_HY2_PROTOCOL`:

```rust
    /// Allocate a hop range for Xray Hysteria2 (main + 99 ports), tagged so it
    /// is distinct from sing-box's ranges.
    pub async fn allocate_xray_hysteria2() -> Result<(u16, (u16, u16))> {
        Self::allocate_xray_hysteria2_at(&PathBuf::from(PORT_ALLOC_FILE)).await
    }

    /// Same as [`allocate_xray_hysteria2`], with an explicit alloc file for tests.
    pub async fn allocate_xray_hysteria2_at(path: &Path) -> Result<(u16, (u16, u16))> {
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
            protocol: XRAY_HY2_PROTOCOL.to_string(),
            created_at: now,
        });
        save_port_alloc_at(path, &data).await?;

        log::info!(
            "Xray Hysteria2 端口分配: 主端口 {}, 跳跃范围 {}-{}",
            main_port,
            main_port + 1,
            hop_end
        );

        Ok((main_port, (main_port + 1, hop_end)))
    }

    /// Release an Xray Hysteria2 hop range.
    ///
    /// Must be called by the Xray delete path. If it is not, the range stays
    /// locked forever and accumulates — the same class of bug previously fixed
    /// for sing-box.
    pub async fn release_xray_hysteria2_range(main_port: u16) -> Result<()> {
        Self::release_xray_hysteria2_range_at(&PathBuf::from(PORT_ALLOC_FILE), main_port).await
    }

    /// Same as [`release_xray_hysteria2_range`], with an explicit alloc file.
    ///
    /// Retains on BOTH the tag and the start port: a sing-box range that happens
    /// to share the port number must survive.
    pub async fn release_xray_hysteria2_range_at(path: &Path, main_port: u16) -> Result<()> {
        let mut data = load_port_alloc_at(path).await.unwrap_or_default();
        let before = data.locked_ranges.len();
        data.locked_ranges
            .retain(|r| !(r.protocol == XRAY_HY2_PROTOCOL && r.start == main_port));

        if data.locked_ranges.len() < before {
            save_port_alloc_at(path, &data).await?;
            log::info!("Xray Hysteria2 端口范围已释放: 主端口 {}", main_port);
        } else {
            log::warn!("Xray Hysteria2 端口范围未找到: 主端口 {}", main_port);
        }
        Ok(())
    }

    /// All Xray Hysteria2 hop ranges currently locked, for cleanup scans.
    pub async fn get_xray_hysteria2_ranges() -> Vec<(u16, (u16, u16))> {
        let data = load_port_alloc().await.unwrap_or_default();
        data.locked_ranges
            .iter()
            .filter(|r| r.protocol == XRAY_HY2_PROTOCOL)
            .map(|r| (r.start, (r.start + 1, r.end)))
            .collect()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(test_xray_alloc) or test(test_xray_release)'`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/xray/port_allocator.rs Cargo.toml Cargo.lock
git commit -m "feat(xray): add an xray-scoped hysteria2 hop-range allocator"
```

---

## Task 5: Extend the batch function with obfs and hopping

**Files:**
- Modify: `rust/aegis/src/core/xray/hysteria2.rs`
- Test: inline tests

**Interfaces:**
- Consumes: everything from Tasks 1-4.
- Produces: `batch_create_hysteria2_xray(count, ip_version, obfs_type: Option<Hysteria2ObfsType>, enable_hopping: bool, link_style: Hy2LinkStyle) -> Result<BatchCreationResult>`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn test_batch_signature_accepts_obfs_and_hopping() {
        // Compile-time contract: the handler in Task 6 calls it with these
        // five arguments. A signature drift must break here, not at the call site.
        let _f: fn(usize, IpVersion, Option<Hysteria2ObfsType>, bool, Hy2LinkStyle) -> _ =
            |count, ip, obfs, hop, style| {
                ConfigManager::batch_create_hysteria2_xray(count, ip, obfs, hop, style)
            };
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --offline -p aegis -E 'test(test_batch_signature_accepts)'`
Expected: FAIL to compile — signature mismatch

- [ ] **Step 3: Implement**

Change the signature and add the hopping branch. The hop path mirrors sing-box's `batch_create_hysteria2`, but calls the **Xray** allocator and tags the firewall rules as Xray's:

```rust
    pub async fn batch_create_hysteria2_xray(
        count: usize,
        ip_version: IpVersion,
        obfs_type: Option<Hysteria2ObfsType>,
        enable_hopping: bool,
        link_style: Hy2LinkStyle,
    ) -> Result<BatchCreationResult> {
```

Inside the loop, replace the unconditional random-port block:

```rust
            let (port, hop_range) = if enable_hopping {
                // Xray keeps its own ranges so a release in one core can never
                // free the other's ports.
                crate::core::xray::port_allocator::PortAllocator::allocate_xray_hysteria2().await?
            } else {
                let p = loop {
                    let candidate = rng.gen_range(10000..60000);
                    if crate::core::xray::port_allocator::PortAllocator::is_port_in_locked_range(
                        candidate,
                    )
                    .await
                    {
                        continue;
                    }
                    if crate::core::system::maintenance::MaintenanceManager::is_port_available(
                        candidate,
                    )
                    .await
                    {
                        break candidate;
                    }
                };
                (p, (p, p))
            };
```

Generate the obfs password when obfs is on, and pass both new values down:

```rust
            let obfs = obfs_type.map(|t| {
                (
                    t,
                    crate::core::singbox::hysteria2::Hysteria2Config::generate_obfs_password(),
                )
            });

            configs.push(Self::build_hysteria2_inbound(
                &tag,
                port as i32,
                &auth,
                &email,
                &sni,
                ip_version,
                obfs.as_ref().map(|(t, p)| (t, p.as_str())),
                enable_hopping.then_some(hop_range),
            ));

            links.push(Self::generate_hysteria2_client_link(
                &auth,
                &host,
                port as i32,
                &sni,
                &email,
                ip_version,
                &pin,
                obfs.as_ref().map(|(t, p)| (t, p.as_str())),
                enable_hopping.then_some(hop_range),
                link_style,
            ));
```

After the port is chosen, add the firewall + range allowances (only when hopping):

```rust
            if enable_hopping {
                let _ = crate::core::system::maintenance::MaintenanceManager::allow_port_range(
                    hop_range.0, hop_range.1,
                )
                .await;
                let has_ipv6 = crate::core::system::SystemMonitor::get_public_ipv6()
                    .await
                    .is_ok();
                if has_ipv6 {
                    let _ = crate::core::system::maintenance::MaintenanceManager::allow_port_range_v6(
                        hop_range.0, hop_range.1,
                    )
                    .await;
                }
                Self::add_xray_hop_firewall_rules(port, hop_range, has_ipv6).await;
            }
```

Add the firewall helpers. The install half goes here; the removal half goes here too so install and removal stay adjacent and cannot drift apart.

```rust
    /// Install the UDP REDIRECT rule that maps a hop range back to the main port.
    ///
    /// Best-effort: a failure is logged, not fatal, matching the sing-box
    /// counterpart. `-C` checks first so a retry cannot stack duplicate rules.
    async fn add_xray_hop_firewall_rules(main_port: u16, hop_range: (u16, u16), has_ipv6: bool) {
        let range_str = format!("{}:{}", hop_range.0, hop_range.1);
        Self::ensure_hop_redirect("iptables", main_port, &range_str).await;
        if has_ipv6 {
            Self::ensure_hop_redirect("ip6tables", main_port, &range_str).await;
        }
    }

    async fn ensure_hop_redirect(bin: &str, main_port: u16, range_str: &str) {
        use tokio::process::Command;

        let to_ports = main_port.to_string();
        let rule = [
            "-t", "nat", "-p", "udp", "--dport", range_str, "-j", "REDIRECT", "--to-ports",
            to_ports.as_str(),
        ];
        // Idempotency: skip if an identical rule is already installed.
        let exists = Command::new(bin)
            .args(["-t", "nat", "-C", "PREROUTING"])
            .args(rule)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if exists {
            return;
        }
        match Command::new(bin)
            .args(["-t", "nat", "-A", "PREROUTING"])
            .args(rule)
            .output()
            .await
        {
            Ok(o) if o.status.success() => log::info!(
                "已配置 Xray Hysteria2 端口跳跃 ({}): 主端口 {}, 范围 {}",
                bin,
                main_port,
                range_str
            ),
            Ok(o) => log::warn!(
                "{} 规则添加失败: {}",
                bin,
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => log::warn!("{} 执行失败 (可能需要 root 权限): {}", bin, e),
        }
    }

    /// Remove everything the hopping path installed.
    ///
    /// Counterpart to `add_xray_hop_firewall_rules`; kept beside it so the two
    /// cannot drift. Stale REDIRECT rules are not merely cruft: a later config
    /// allocated into the same port range would have its traffic redirected to
    /// a dead main port.
    ///
    /// Mirrors sing-box's `cleanup_specific_hysteria2_rules` (`core/singbox/config.rs:111`):
    /// delete the REDIRECT rules, then drop the firewall allowances for the
    /// main port and the hop range.
    pub(crate) async fn remove_xray_hop_firewall_rules(
        main_port: u16,
        hop_range: (u16, u16),
        has_ipv6: bool,
    ) {
        use crate::core::system::maintenance::MaintenanceManager;
        use tokio::process::Command;

        let range_str = format!("{}:{}", hop_range.0, hop_range.1);
        let to_ports = main_port.to_string();
        let rule = [
            "-t", "nat", "-D", "PREROUTING", "-p", "udp", "--dport", range_str.as_str(),
            "-j", "REDIRECT", "--to-ports", to_ports.as_str(),
        ];

        let _ = Command::new("iptables").args(rule).output().await;
        if has_ipv6 {
            let _ = Command::new("ip6tables").args(rule).output().await;
            let _ = MaintenanceManager::remove_port_range_v6(hop_range.0, hop_range.1).await;
        }

        // The main port was allowed with allow_port; the hop range with
        // allow_port_range. Both must be undone.
        let _ = MaintenanceManager::remove_port_range(main_port, main_port).await;
        let _ = MaintenanceManager::remove_port_range(hop_range.0, hop_range.1).await;

        log::info!(
            "已清理 Xray Hysteria2 端口跳跃规则: 主端口 {}, 范围 {}",
            main_port,
            range_str
        );
    }
```

The non-hopping path keeps its existing `allow_port(port)` allowance. Do not remove it — the hopping branch is what adds the extra range allowances.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis` (full crate — Task 6's call site will break until updated, so expect a compile error naming `handlers/xray.rs`; that is Task 6's edit)
Expected: the new signature test passes once the call site is updated

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/xray/hysteria2.rs rust/aegis/src/shared/handlers/xray.rs
git commit -m "feat(xray): support obfs and port hopping in the hysteria2 batch flow"
```

---

## Task 6: Release hop ranges when an Xray Hysteria2 config is deleted

**Files:**
- Modify: `rust/aegis/src/core/xray/config.rs`
- Test: `rust/aegis/tests/xray_hy2_hop_cleanup.rs` (new integration test)

**Interfaces:**
- Consumes: `PortAllocator::release_xray_hysteria2_range_at` (Task 4), `ConfigManager::remove_xray_hop_firewall_rules` (Task 5).
- Produces:
  - `extract_xray_hy2_hop_ports(path: &str) -> Result<Vec<u16>>` (new, `pub(crate)`)
  - `pub async fn delete_specific_configuration_at(path: &str, alloc_file: Option<&Path>) -> Result<()>`
  - `delete_specific_configuration(path)` becomes a thin wrapper over `_at(path, None)`

**The cleanup must undo BOTH halves of what hopping installed.** Task 5 installs iptables REDIRECT rules and firewall allowances; releasing only the `.port_alloc` range is insufficient. Stale REDIRECT rules are not merely cruft: a later config allocated into the same range would get its traffic redirected to a dead main port. This task therefore removes the rules AND releases the range — exactly as sing-box's `cleanup_hysteria2_ports` does (`core/singbox/config.rs:172-182`).

**Follow the existing sing-box pattern exactly.** `SingBoxConfigManager::delete_specific_configuration_at` (`core/singbox/config.rs:193-209`) is the proven shape — read it before writing this task. It does three things in a deliberate order, and the order is load-bearing:

1. Extract the hop ports while the file still exists.
2. Remove the file.
3. **Only if removal succeeded**, release the ranges and clear the firewall rules.

Its comment states the reason: removing the file first avoids the state where the file is gone but the ports are still allocated. Cleanup before a failed delete would free ports for a config that is still live. Match that ordering, and match the `_at(path, Option<&Path>)` signature convention so the codebase keeps one idiom.

- [ ] **Step 1: Write the failing test**

Create `rust/aegis/tests/xray_hy2_hop_cleanup.rs`. Note it mirrors the existing `tests/hy2_port_hopping.rs`, which locks down the same class of bug for sing-box — read that file first for the house style.

```rust
//! End-to-end contract for Xray Hysteria2 hop-range cleanup on delete.
//!
//! The regression this locks down: if delete removes the config file but never
//! releases the locked hop range, the range stays in `.port_alloc` forever and
//! accumulates. sing-box had exactly this bug; the Xray path must not repeat it.

use aegis::core::xray::config::ConfigManager;
use aegis::core::xray::port_allocator::PortAllocator;
use serde_json::json;
use std::path::Path;

/// Main ports of ranges tagged for Xray Hysteria2 in an alloc file.
fn xray_main_ports(alloc: &Path) -> Vec<u16> {
    let Ok(content) = std::fs::read_to_string(alloc) else {
        return Vec::new();
    };
    let data: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    data["locked_ranges"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|r| r["protocol"].as_str() == Some("xray-hysteria2"))
                .filter_map(|r| r["start"].as_u64().map(|v| v as u16))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn delete_flow_releases_xray_hop_range_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");

    // Arrange: a created hopping config plus its locked range.
    let (main, _hop) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();
    assert_eq!(xray_main_ports(&alloc), vec![main]);

    let cfg_path = dir.path().join("batch_xray_hysteria2_it_inbounds.json");
    let cfg = json!({
        "inbounds": [{
            "protocol": "hysteria",
            "port": main,
            "streamSettings": {
                "finalmask": { "quicParams": { "udpHop": { "ports": format!("{}-{}", main + 1, main + 99) } } }
            }
        }]
    });
    std::fs::write(&cfg_path, cfg.to_string()).unwrap();

    // Act: the real delete path, pointed at this alloc file.
    ConfigManager::delete_specific_configuration_at(
        cfg_path.to_str().unwrap(),
        Some(&alloc),
    )
    .await
    .unwrap();

    // Assert: range released AND file gone. A leak is the bug this test exists for.
    assert!(
        xray_main_ports(&alloc).is_empty(),
        "hop range {main} was not released on delete"
    );
    assert!(!cfg_path.exists(), "config file was not removed");
}

#[tokio::test]
async fn delete_of_a_non_hopping_config_leaves_ranges_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    // A live range that must survive deleting an unrelated plain config.
    let (keep, _) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();

    let cfg_path = dir.path().join("batch_xray_hysteria2_plain_inbounds.json");
    std::fs::write(
        &cfg_path,
        json!({"inbounds":[{"protocol":"hysteria","port":12345}]}).to_string(),
    )
    .unwrap();

    ConfigManager::delete_specific_configuration_at(
        cfg_path.to_str().unwrap(),
        Some(&alloc),
    )
    .await
    .unwrap();

    // The unrelated range must still be locked: cleanup is content-driven, and
    // this config has no udpHop block.
    assert_eq!(xray_main_ports(&alloc), vec![keep]);
    assert!(!cfg_path.exists());
}

#[tokio::test]
async fn delete_of_a_non_hysteria_config_does_not_touch_xray_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let alloc = dir.path().join(".port_alloc");
    let (keep, _) = PortAllocator::allocate_xray_hysteria2_at(&alloc)
        .await
        .unwrap();

    // A Vision-style config must be inert to Hy2 hop cleanup.
    let cfg_path = dir.path().join("batch_reality_abcd1234_inbounds.json");
    std::fs::write(
        &cfg_path,
        json!({"inbounds":[{"protocol":"vless","port":keep}]}).to_string(),
    )
    .unwrap();

    ConfigManager::delete_specific_configuration_at(
        cfg_path.to_str().unwrap(),
        Some(&alloc),
    )
    .await
    .unwrap();

    assert_eq!(xray_main_ports(&alloc), vec![keep]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --offline -p aegis -E 'test(delete_flow_releases_xray) or test(delete_of_a_non_)'`
Expected: FAIL to compile — `no function or associated item named 'delete_specific_configuration_at'`

- [ ] **Step 3: Implement the extractor, the `_at` variant, and the wrapper**

In `rust/aegis/src/core/xray/config.rs`, add to `impl ConfigManager`:

```rust
    /// Main ports of Xray Hysteria2 inbounds in `path` that carry a hop range.
    ///
    /// Detected by CONTENT (`protocol == "hysteria"` AND a
    /// `finalmask.quicParams.udpHop` block), never by filename: cleanup must be
    /// driven by what the config actually is, not what it is called.
    pub(crate) async fn extract_xray_hy2_hop_ports(path: &str) -> Result<Vec<u16>> {
        let content = fs::read_to_string(path).await?;
        let parsed: Value = serde_json::from_str(&content).unwrap_or_default();
        let mut ports = Vec::new();
        if let Some(inbounds) = parsed.get("inbounds").and_then(|v| v.as_array()) {
            for inbound in inbounds {
                let is_hysteria =
                    inbound.get("protocol").and_then(|v| v.as_str()) == Some("hysteria");
                let has_hop = inbound
                    .get("streamSettings")
                    .and_then(|s| s.get("finalmask"))
                    .and_then(|f| f.get("quicParams"))
                    .and_then(|q| q.get("udpHop"))
                    .is_some();
                if is_hysteria
                    && has_hop
                    && let Some(port) = inbound.get("port").and_then(|v| v.as_u64())
                    && port <= u16::MAX as u64
                {
                    ports.push(port as u16);
                }
            }
        }
        Ok(ports)
    }

    /// Delete a config, releasing any Hysteria2 hop ranges it owns.
    ///
    /// `alloc_file: None` means the production alloc file. Split from the
    /// no-argument form so tests can isolate the alloc file.
    ///
    /// Order matters and follows `SingBoxConfigManager::delete_specific_configuration_at`:
    /// read the ports while the file exists, remove it, THEN clean up. Cleaning
    /// up before a failed delete would free ports still in use by a live config.
    pub async fn delete_specific_configuration_at(
        path: &str,
        alloc_file: Option<&Path>,
    ) -> Result<()> {
        // Extract while the file is still readable; a non-hy2 file yields an
        // empty list rather than an error.
        let hop_ports = Self::extract_xray_hy2_hop_ports(path).await.unwrap_or_default();

        fs::remove_file(path).await.context("❌ 删除配置文件失败")?;

        // Only now that the file is gone do we clean up. Best-effort: a cleanup
        // failure must not surface as a failed delete to the user.
        if !hop_ports.is_empty() {
            let alloc = alloc_file
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/etc/wwps/.port_alloc"));
            let has_ipv6 = crate::core::system::SystemMonitor::get_public_ipv6()
                .await
                .is_ok();
            for main_port in hop_ports {
                // Firewall rules first, then the range: a rule left referring
                // to a released port could redirect a future config's traffic.
                Self::remove_xray_hop_firewall_rules(main_port, (main_port + 1, main_port + 99), has_ipv6)
                    .await;
                let _ = crate::core::xray::port_allocator::PortAllocator::release_xray_hysteria2_range_at(
                    &alloc,
                    main_port,
                )
                .await;
            }
        }

        crate::core::system::maintenance::MaintenanceManager::reload_core().await?;
        Ok(())
    }

    pub async fn delete_specific_configuration(path: &str) -> Result<()> {
        Self::delete_specific_configuration_at(path, None).await
    }
```

Add `Path`/`PathBuf` to this file's imports if absent (`use std::path::{Path, PathBuf};`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --offline -p aegis -E 'test(delete_flow_releases_xray) or test(delete_of_a_non_)'`
Expected: PASS (3 tests)

- [ ] **Step 5: Verify no regression for the other protocols**

Run: `cargo nextest run --offline -p aegis -E 'test(del) or test(delete)'`
Expected: PASS. The existing delete tests must be unaffected — including the sing-box `tests/hy2_port_hopping.rs` suite, which locks the analogous behaviour for the other core.

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/core/xray/config.rs rust/aegis/tests/xray_hy2_hop_cleanup.rs
git commit -m "fix(xray): release hysteria2 hop ranges when a config is deleted"
```

---

## Task 7: Wire the obfs and hopping UI

**Files:**
- Modify: `rust/aegis/src/shared/handlers/xray.rs`
- Modify: `rust/aegis/src/resources/i18n/zh.yml`, `en.yml`, `ja.yml`
- Test: inline `#[cfg(test)] mod tests` in the handler, if one exists; otherwise assertions on the callback-encoding parser

**Interfaces:**
- Consumes: `Hysteria2ObfsType`, `Hy2LinkStyle`, `batch_create_hysteria2_xray` (Task 5).
- Produces: callback data `<ip>:<count>:<obfs>:<hop>:<style>` where `obfs` ∈ `{0,1,2}` (none/salamander/gecko) — the same encoding sing-box uses, so the two flows stay learnable together.

- [ ] **Step 1: Write the failing test**

Add a pure parser so the encoding is testable without a harness. In `rust/aegis/src/shared/handlers/xray.rs`:

```rust
    #[test]
    fn test_hy2_exec_params_parse_all_combinations() {
        // Mirrors the sing-box encoding so users see one convention.
        assert_eq!(
            parse_hy2_exec_params("4:3:0:0:official"),
            Some((IpVersion::IPv4, 3, None, false, Hy2LinkStyle::Official))
        );
        assert_eq!(
            parse_hy2_exec_params("4:3:1:0:v2rayn"),
            Some((
                IpVersion::IPv4,
                3,
                Some(Hysteria2ObfsType::Salamander),
                false,
                Hy2LinkStyle::V2rayN
            ))
        );
        assert_eq!(
            parse_hy2_exec_params("6:5:2:1:official"),
            Some((
                IpVersion::IPv6,
                5,
                Some(Hysteria2ObfsType::Gecko),
                true,
                Hy2LinkStyle::Official
            ))
        );
        // Malformed input must be rejected, not silently defaulted to a
        // surprising combination.
        assert_eq!(parse_hy2_exec_params("4:3"), None);
        assert_eq!(parse_hy2_exec_params(""), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --offline -p aegis -E 'test(test_hy2_exec_params_parse)'`
Expected: FAIL to compile — `cannot find function 'parse_hy2_exec_params'`

- [ ] **Step 3: Implement the parser**

```rust
/// Parse `u_hy2_batch_exec:<ip>:<count>:<obfs>:<hop>:<style>` callback data.
///
/// `obfs`: 0 none, 1 salamander, 2 gecko. `hop`: "0"/"1". `style`: "v2rayn"
/// selects the v2rayN link form, anything else the official form.
fn parse_hy2_exec_params(
    body: &str,
) -> Option<(IpVersion, usize, Option<Hysteria2ObfsType>, bool, Hy2LinkStyle)> {
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() != 5 {
        return None;
    }
    let ip_version = match parts[0] {
        "6" => IpVersion::IPv6,
        "4" => IpVersion::IPv4,
        _ => return None,
    };
    let count: usize = parts[1].parse().ok()?;
    let obfs = match parts[2] {
        "1" => Some(Hysteria2ObfsType::Salamander),
        "2" => Some(Hysteria2ObfsType::Gecko),
        "0" => None,
        _ => return None,
    };
    let enable_hopping = match parts[3] {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    let link_style = match parts[4] {
        "v2rayn" => Hy2LinkStyle::V2rayN,
        _ => Hy2LinkStyle::Official,
    };
    Some((ip_version, count, obfs, enable_hopping, link_style))
}
```

- [ ] **Step 4: Rewire `handle_hy2_batch_exec` to use the parser**

Replace the current 2-part parsing with the parser, and pass the extra arguments:

```rust
    let prefix = "u_hy2_batch_exec:";
    let Some((ip_version, n, obfs_type, enable_hopping, link_style)) =
        parse_hy2_exec_params(data.strip_prefix(prefix).unwrap_or(""))
    else {
        return Ok(HandlerAction::Done);
    };
```

and the dispatch becomes:

```rust
    match ConfigManager::batch_create_hysteria2_xray(
        n,
        ip_version,
        obfs_type,
        enable_hopping,
        link_style,
    )
    .await
```

- [ ] **Step 5: Add the obfs/hop choice step to the prompt chain**

Mirror sing-box's `sb_h2_obfs` / `sb_h2_hop` flow: between the count prompt and exec, offer obfs (none/salamander/gecko) and hopping (off/on). The count buttons must stop emitting the 2-part `u_hy2_batch_exec:<ip>:<n>` form and instead emit the 5-part form after the two choices. Add handler arms `handle_hy2_batch_obfs` and `handle_hy2_batch_hop` and register routes `u_hy2_batch_obfs:` and `u_hy2_batch_hop:` in the dispatcher next to the existing `u_hy2_*` arms.

Required properties for the UI text: show the currently selected obfs and hop state in the prompt so the user can see what they are about to create; keep a back button that returns to the previous step; and keep the final confirm button disabled-labelled until both choices are made (or default both to off and let the user proceed immediately — sing-box defaults to off and proceeds, so match that).

i18n keys to add to all three locales (values may be shared across locales since they are proper nouns/emoji):
```yaml
  hy2_obfs_title: "Choose Hysteria2 obfuscation"
  hy2_obfs_none: "🚫 None"
  hy2_obfs_salamander: "🦎 Salamander"
  hy2_obfs_gecko: "🦎 Gecko (fragmented)"
  hy2_hop_title: "Enable port hopping?"
  hy2_hop_off: "🚫 Off"
  hy2_hop_on: "🎲 On (100-port range)"
```

- [ ] **Step 6: Run tests and the full gate**

Run: `cargo nextest run --offline -p aegis`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add rust/aegis/src/shared/handlers/xray.rs rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml
git commit -m "feat(xray): wire hysteria2 obfs and port-hopping into the batch UI"
```

---

## Task 8: Quality gates and end-to-end verification

**Files:** none; verification only.

- [ ] **Step 1: Run the full rust-lint-format gate**

```bash
cargo fmt --all --check
cargo clippy --offline --all-targets --all-features -- -D warnings
cargo nextest run --offline
cargo test --doc --offline
```
Expected: all clean. Note `cargo test --doc` is required by the skill and was missed on the previous branch.

- [ ] **Step 2: Verify the obfs schema against the Xray authority**

Confirm in the generated inbound that:
- `finalmask.udp[0].type == "salamander"` (never `"gecko"`)
- gecko carries `settings.packetSize` in `"MIN-MAX"` form with MAX ≤ 2048
- hopping writes `finalmask.quicParams.udpHop.ports == "A-B"` and `interval == 30`
- when both are on they coexist in ONE `finalmask` object

Cross-check against https://xtls.github.io/config/transports/finalmask.html

- [ ] **Step 3: Verify no hop-range leak**

Run the integration test and confirm `xray_main_ports` is empty after cleanup:
`cargo nextest run --offline -p aegis -E 'test(xray_hy2_hop_cleanup)'`

- [ ] **Step 4: Confirm sing-box is untouched**

```bash
git diff --stat main..HEAD -- rust/aegis/src/core/singbox/
```
Expected: empty.

- [ ] **Step 5: Commit any final fixes, then hand to review**

```bash
git add -A
git commit -m "chore(xray): verify hysteria2 obfs and hopping against the Xray reference"
```

---

## Self-Review

**1. Spec coverage**

| Approved design decision | Task |
|---|---|
| Reuse `Hy2LinkStyle` (decision A) | Task 3 |
| Reuse `Hysteria2ObfsType`; gecko = salamander + packetSize (decision A) | Task 1 |
| Separate Xray allocator with `"xray-hysteria2"` tag (decision A) | Task 4 |
| Hop ranges AND their firewall rules released on delete (delete-after-extract order, `_at` variant) | Task 5 (removal helper) + Task 6 (invocation) |
| UI mirrors sing-box's obfs/hop flow | Task 7 |
| Obligation to implement obfs + hop at all | Tasks 1-3, 5, 7 |

No gaps.

**2. Placeholder scan**

No TBD/TODO. Every code step carries the actual code. Task 7 Step 5 is the one step that describes UI requirements rather than pasting the full widget tree, because the surrounding handler code is long and its exact shape must be read before editing; it lists the required properties and the exact i18n keys so the result is still verifiable.

**3. Type consistency**

- `build_hysteria2_inbound(..., obfs: Option<(&Hysteria2ObfsType, &str)>, hop_range: Option<(u16,u16)>)` — Task 1 adds arg 7, Task 2 adds arg 8; Task 5 passes both.
- `generate_hysteria2_client_link(..., obfs, hop_range, link_style)` — Task 3; Task 5 passes all.
- `allocate_xray_hysteria2() -> Result<(u16,(u16,u16))>` / `release_xray_hysteria2_range_at(&Path, u16)` — Task 4; consumed by Task 5 and Task 6.
- `remove_xray_hop_firewall_rules(main_port: u16, hop_range: (u16,u16), has_ipv6: bool)` — Task 5 defines, Task 6 calls.
- `delete_specific_configuration_at(path: &str, alloc_file: Option<&Path>) -> Result<()>` — Task 6; `delete_specific_configuration(path)` is a wrapper passing `None`. Mirrors the sing-box `_at` convention (`core/singbox/config.rs:193`).
- `extract_xray_hy2_hop_ports(path: &str) -> Result<Vec<u16>>` — Task 6, `pub(crate)`.
- `parse_hy2_exec_params(&str) -> Option<(IpVersion, usize, Option<Hysteria2ObfsType>, bool, Hy2LinkStyle)>` — Task 7.
- Callback encoding `<ip>:<count>:<obfs>:<hop>:<style>` is introduced in Task 7 and parsed by the same task; the sing-box reference uses the identical 5-part shape.

**4. Risk register (carried into dispatches)**

- **Task 6 is the highest risk:** it edits a delete function shared by all four protocols. The guard is content-based detection (`protocol == "hysteria"` + `udpHop` present), and `delete_of_a_non_hopping_config_leaves_ranges_untouched` / `delete_of_a_non_hysteria_config_does_not_touch_xray_ranges` pin that unrelated configs cause no side effect.
- **Cleanup is two halves, not one.** Releasing the `.port_alloc` range without deleting the iptables REDIRECT rule leaves a rule that can hijack a future config's traffic. Task 5 places install and removal side by side; Task 6 calls removal before release. A reviewer should check both happen.
- **Task 4 must not reuse the sing-box tag.** `test_xray_release_does_not_touch_singbox_range` seeds a sing-box range at the same port number and asserts it survives — this is the isolation contract.
- **Task 2 must build one `finalmask` object.** Assigning `finalmask` per feature would drop the earlier one; `test_hysteria2_hop_and_obfs_coexist_in_one_finalmask` pins it.
