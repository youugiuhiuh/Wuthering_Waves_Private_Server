# Hysteria2 Gecko Obfuscation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `gecko` as a selectable Hysteria2 obfuscation type in `rust/aegis`, replacing the hardcoded `salamander` toggle with a three-state choice (none / salamander / gecko).

**Architecture:** An enum `Hysteria2ObfsType { Salamander, Gecko }` replaces the `obfs_type: Option<String>` field and the `enable_obfs: bool` batch flag. The Telegram callback chain gains one step (`sb_h2_obfs_type`) between enable and hop selection. Gecko emits `min_packet_size: 512` / `max_packet_size: 1200` (constants) in the inbound JSON and `obfs=gecko` in client links; the batch result appends a client-compatibility note for gecko runs.

**Tech Stack:** Rust (edition 2024), serde_json, teloxide bot adapter, rust_i18n (zh/en/ja), cargo nextest.

**Spec:** `docs/superpowers/specs/2026-08-30-hy2-gecko-obfs-design.md`

## Global Constraints

- **No runtime version check** (spec decision 2) — assume sing-box is always latest.
- **Packet sizes fixed**: emit exactly `min_packet_size: 512`, `max_packet_size: 1200` via constants `GECKO_DEFAULT_MIN_PACKET_SIZE` / `GECKO_DEFAULT_MAX_PACKET_SIZE`; never exposed in UI (spec decision 3).
- **Type is an enum, not a string** (spec decision 4): `Hysteria2ObfsType` with `as_str()` helper; no stringly-typed API.
- **Callback encoding**: `0` = none, `1` = salamander, `2` = gecko, throughout the `sb_h2_*` chain.
- **UI order**: Salamander button always first / recommended (spec decision 1).
- **i18n**: every new user-facing string must exist in `zh.yml`, `en.yml`, `ja.yml`.
- **Quality gate** (rust-lint-format), run from `rust/aegis` before every commit and again at the end:
  `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run && cargo test --doc`
- Rust skill rules in force: `err-no-unwrap-prod` (defensive `unwrap_or` for callback parsing, mirroring existing style), `own-copy-small` + `api-common-traits` (enum derives `Debug, Clone, Copy, PartialEq`), `test-arrange-act-assert` / `test-descriptive-names` in `#[cfg(test)] mod tests`.

---

### Task 1: Obfuscation type enum + constants + struct field change

**Files:**
- Modify: `rust/aegis/src/core/singbox/hysteria2.rs` (struct + tests)

**Interfaces:**
- Consumes: nothing new (existing `Hysteria2Config` struct).
- Produces: `pub enum Hysteria2ObfsType { Salamander, Gecko }` (derives `Debug, Clone, Copy, PartialEq`); `impl Hysteria2ObfsType { pub fn as_str(self) -> &'static str }`; `pub const GECKO_DEFAULT_MIN_PACKET_SIZE: usize = 512`; `pub const GECKO_DEFAULT_MAX_PACKET_SIZE: usize = 1200`; `Hysteria2Config::with_obfs(port: u16, password: String, sni: String, obfs_type: Hysteria2ObfsType, obfs_password: String) -> Self`; field `pub obfs_type: Option<Hysteria2ObfsType>`.

- [ ] **Step 1: Update tests to the new API and add enum tests**

In `hysteria2.rs` `#[cfg(test)] mod tests`, replace the body of `test_hysteria2_config_with_obfs`:

```rust
    #[test]
    fn test_hysteria2_config_with_obfs() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "test_password".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs_secret".to_string(),
        );
        assert_eq!(config.port, 8443);
        assert_eq!(config.obfs_type, Some(Hysteria2ObfsType::Salamander));
        assert_eq!(config.obfs_password, Some("obfs_secret".to_string()));
        assert!(config.pin_sha256.is_none());
    }
```

Add two new tests next to it:

```rust
    #[test]
    fn test_obfs_type_as_str() {
        assert_eq!(Hysteria2ObfsType::Salamander.as_str(), "salamander");
        assert_eq!(Hysteria2ObfsType::Gecko.as_str(), "gecko");
    }

    #[test]
    fn test_obfs_type_copy() {
        let t = Hysteria2ObfsType::Gecko;
        let t2 = t; // Copy, not move
        assert_eq!(t, t2);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: compile error — `Hysteria2ObfsType` not found, and `with_obfs` type mismatch.

- [ ] **Step 3: Implement the enum, constants, and field change**

Add above `pub struct Hysteria2Config`:

```rust
/// Hysteria2 QUIC traffic obfuscation type (`obfs.type`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hysteria2ObfsType {
    Salamander,
    Gecko,
}

impl Hysteria2ObfsType {
    /// JSON and client URI value for this obfuscation type.
    pub fn as_str(self) -> &'static str {
        match self {
            Hysteria2ObfsType::Salamander => "salamander",
            Hysteria2ObfsType::Gecko => "gecko",
        }
    }
}

/// Default minimum on-wire packet size in bytes for gecko (sing-box default).
pub const GECKO_DEFAULT_MIN_PACKET_SIZE: usize = 512;

/// Default maximum on-wire packet size in bytes for gecko (sing-box default).
pub const GECKO_DEFAULT_MAX_PACKET_SIZE: usize = 1200;
```

Change the struct field:

```rust
    pub obfs_type: Option<Hysteria2ObfsType>,
```

Change `with_obfs`:

```rust
    pub fn with_obfs(
        port: u16,
        password: String,
        sni: String,
        obfs_type: Hysteria2ObfsType,
        obfs_password: String,
    ) -> Self {
        Self {
            port,
            password,
            sni,
            obfs_type: Some(obfs_type),
            obfs_password: Some(obfs_password),
            pin_sha256: None,
        }
    }
```

`new()` already sets `obfs_type: None` — unchanged.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: PASS (all hysteria2 tests including the two new ones).

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/hysteria2.rs
git commit -m "feat: add Hysteria2ObfsType enum and gecko packet-size constants"
```

---

### Task 2: Gecko fields in `to_inbound_json`

**Files:**
- Modify: `rust/aegis/src/core/singbox/hysteria2.rs` (JSON generation + tests)

**Interfaces:**
- Consumes: `Hysteria2ObfsType`, `GECKO_DEFAULT_MIN_PACKET_SIZE`, `GECKO_DEFAULT_MAX_PACKET_SIZE` (Task 1).
- Produces: inbound JSON with `obfs.type` = `"salamander"` or `"gecko"`; for gecko additionally `obfs.min_packet_size: 512` and `obfs.max_packet_size: 1200`; salamander and none emit no extra keys.

- [ ] **Step 1: Write failing tests**

Replace the body of existing `test_hysteria2_to_inbound_json_with_obfs` (it must now use the enum and assert no packet-size keys for salamander):

```rust
    #[test]
    fn test_hysteria2_to_inbound_json_with_obfs() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "pw".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Salamander,
            "obfs123".to_string(),
        );
        let json = config.to_inbound_json("test-tag");
        assert!(json["obfs"].is_object());
        assert_eq!(json["obfs"]["type"], "salamander");
        assert_eq!(json["obfs"]["password"], "obfs123");
        assert!(json["obfs"].get("min_packet_size").is_none());
        assert!(json["obfs"].get("max_packet_size").is_none());
    }
```

Add a new test:

```rust
    #[test]
    fn test_hysteria2_to_inbound_json_with_gecko() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "pw".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        );
        let json = config.to_inbound_json("test-tag");
        assert!(json["obfs"].is_object());
        assert_eq!(json["obfs"]["type"], "gecko");
        assert_eq!(json["obfs"]["password"], "obfs123");
        assert_eq!(json["obfs"]["min_packet_size"], 512);
        assert_eq!(json["obfs"]["max_packet_size"], 1200);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: FAIL — `min_packet_size` / `max_packet_size` missing for gecko.

- [ ] **Step 3: Implement**

Replace the obfs block in `to_inbound_json`:

```rust
        if let Some(ref obfs_type) = self.obfs_type
            && let Some(ref obfs_password) = self.obfs_password
        {
            let mut obfs_map = serde_json::Map::new();
            obfs_map.insert("type".to_string(), serde_json::json!(obfs_type.as_str()));
            obfs_map.insert("password".to_string(), serde_json::json!(obfs_password));
            if *obfs_type == Hysteria2ObfsType::Gecko {
                obfs_map.insert(
                    "min_packet_size".to_string(),
                    serde_json::json!(GECKO_DEFAULT_MIN_PACKET_SIZE),
                );
                obfs_map.insert(
                    "max_packet_size".to_string(),
                    serde_json::json!(GECKO_DEFAULT_MAX_PACKET_SIZE),
                );
            }
            map.insert("obfs".to_string(), serde_json::json!(obfs_map));
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/hysteria2.rs
git commit -m "feat: emit gecko min/max packet size in hysteria2 inbound JSON"
```

---

### Task 3: Parameterize obfs value in client links

**Files:**
- Modify: `rust/aegis/src/core/singbox/hysteria2.rs` (links + tests)

**Interfaces:**
- Consumes: `self.obfs_type: Option<Hysteria2ObfsType>` (Task 1).
- Produces: `to_client_link_with_obfs` / `to_client_link_with_hopping_and_obfs` emit `obfs=salamander` or `obfs=gecko` matching the selected type (fallback `"salamander"` if unset). Signatures unchanged.

- [ ] **Step 1: Write failing tests**

Update `test_hysteria2_to_client_link_with_obfs_no_hopping` and `test_hysteria2_to_client_link_with_hopping_and_obfs` to pass `Hysteria2ObfsType::Salamander` instead of `"salamander".to_string()`, and add:

```rust
    #[test]
    fn test_hysteria2_to_client_link_with_gecko_no_hopping() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_obfs("1.2.3.4", "MyNode");
        assert!(link.starts_with("hysteria2://"));
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(!link.contains("obfs=salamander"));
        assert!(!link.contains("hop_interval=30s"));
    }

    #[test]
    fn test_hysteria2_to_client_link_with_gecko_hopping() {
        let config = Hysteria2Config::with_obfs(
            8443,
            "mypassword".to_string(),
            "sni.example.com".to_string(),
            Hysteria2ObfsType::Gecko,
            "obfs123".to_string(),
        )
        .with_pin_sha256("AA:BB:CC".to_string());
        let link = config.to_client_link_with_hopping_and_obfs("1.2.3.4", "MyNode", (8444, 8543));
        assert!(link.contains("obfs=gecko"));
        assert!(link.contains("obfs-password=obfs123"));
        assert!(link.contains("hop_interval=30s"));
        assert!(!link.contains("obfs=salamander"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: FAIL — links contain `obfs=salamander` even for gecko.

- [ ] **Step 3: Implement**

In `to_client_link_with_obfs`, after `let obfs_password = ...`, add the type value and use it in the format string:

```rust
        let obfs_value = self.obfs_type.map(|t| t.as_str()).unwrap_or("salamander");
```

Replace the `format!` with:

```rust
        format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&obfs={}&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            pin_param,
            obfs_value,
            encoded_obfs_password,
            encoded_name
        )
```

Do the same in `to_client_link_with_hopping_and_obfs` (same `obfs_value` line; the `format!` keeps `hop_interval=30s` and gains `obfs={}`):

```rust
        format!(
            "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3{}&hop_interval=30s&obfs={}&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            hop_range.0,
            hop_range.1,
            encoded_sni,
            pin_param,
            obfs_value,
            encoded_obfs_password,
            encoded_name
        )
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run core::singbox::hysteria2`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/hysteria2.rs
git commit -m "feat: emit obfs type in hysteria2 client links"
```

---

### Task 4: Batch creation takes `Option<Hysteria2ObfsType>`

**Files:**
- Modify: `rust/aegis/src/core/singbox/hy2_batch.rs`
- Modify: `rust/aegis/src/shared/handlers/singbox.rs` (minimal call-site fix so the crate compiles)

**Interfaces:**
- Consumes: `Hysteria2ObfsType` (Task 1).
- Produces: `SingBoxConfigManager::batch_create_hysteria2(count: usize, ip_version: IpVersion, obfs_type: Option<Hysteria2ObfsType>, enable_hopping: bool) -> Result<BatchCreationResult>`. Hardcoded `"salamander"` removed. This is a `Copy` enum so the loop can reuse it per iteration.

- [ ] **Step 1: Update the implementation**

In `hy2_batch.rs`, change the import:

```rust
use super::hysteria2::{Hysteria2Config, Hysteria2ObfsType};
```

Change the signature:

```rust
    pub async fn batch_create_hysteria2(
        count: usize,
        ip_version: IpVersion,
        obfs_type: Option<Hysteria2ObfsType>,
        enable_hopping: bool,
    ) -> Result<BatchCreationResult> {
```

Replace the config construction block:

```rust
            let config = if let Some(obfs_type) = obfs_type {
                let obfs_password = Hysteria2Config::generate_obfs_password();
                Hysteria2Config::with_obfs(
                    main_port,
                    password.clone(),
                    sni.clone(),
                    obfs_type,
                    obfs_password,
                )
                .with_pin_sha256(pin_sha256.clone())
            } else {
                Hysteria2Config::new(main_port, password.clone(), sni.clone())
                    .with_pin_sha256(pin_sha256.clone())
            };
```

Replace the link selection (obfs presence is now `obfs_type.is_some()`):

```rust
            let link = if obfs_type.is_some() && enable_hopping {
                config.to_client_link_with_hopping_and_obfs(&host, &tag, hop_range)
            } else if obfs_type.is_some() {
                config.to_client_link_with_obfs(&host, &tag)
            } else if enable_hopping {
                config.to_client_link_with_hopping(&host, &tag, hop_range)
            } else {
                config.to_client_link(&host, &tag)
            };
```

In `singbox.rs` handler, add the import:

```rust
use crate::core::singbox::hysteria2::Hysteria2ObfsType;
```

In the `sb_h2_exec:` block (currently `let obfs_enabled: bool = parts[2] == "1";`), replace with:

```rust
            let obfs_type = if parts[2] == "1" {
                Some(Hysteria2ObfsType::Salamander)
            } else {
                None
            };
```

and change the call:

```rust
                match SingBoxConfigManager::batch_create_hysteria2(
                    count,
                    ip_version,
                    obfs_type,
                    hopping_enabled,
                )
                .await
```

(`sb_h2_exec` full UI rework happens in Task 6; this keeps the crate compiling now. `parts[2] == "2"` is not produced yet.)

- [ ] **Step 2: Run tests to verify the crate compiles and tests pass**

Run: `cargo nextest run`
Expected: compile OK, 673+ tests pass (1 skip).

- [ ] **Step 3: Commit**

```bash
git add rust/aegis/src/core/singbox/hy2_batch.rs rust/aegis/src/shared/handlers/singbox.rs
git commit -m "refactor: batch_create_hysteria2 takes Option<Hysteria2ObfsType>"
```

---

### Task 5: i18n keys for type selection, status labels, and gecko note

**Files:**
- Modify: `rust/aegis/src/resources/i18n/zh.yml` (after line 82, inside `menu:`)
- Modify: `rust/aegis/src/resources/i18n/en.yml` (after line 72, inside `menu:`)
- Modify: `rust/aegis/src/resources/i18n/ja.yml` (after line 72, inside `menu:`)

**Interfaces:**
- Produces keys consumed by Task 6/7: `menu.singbox_h2_obfs_type_title` (args 0=IP, 1=count), `menu.singbox_h2_obfs_type_salamander`, `menu.singbox_h2_obfs_type_gecko`, `menu.singbox_h2_obfs_salamander`, `menu.singbox_h2_obfs_gecko`, `menu.singbox_h2_gecko_note`.

- [ ] **Step 1: Add keys to zh.yml**

Append after the `singbox_h2_hop_disable` line (line 82):

```yaml
  singbox_h2_obfs_type_title: "🚀 <b>Hysteria2 批量创建</b>\n\n🌐 网络协议: %{0}\n📊 生成数量: %{1}\n\n请选择混淆类型:"
  singbox_h2_obfs_type_salamander: "🟢 Salamander (推荐)"
  singbox_h2_obfs_type_gecko: "🟦 Gecko (实验性，抗 DPI 更强)"
  singbox_h2_obfs_salamander: "🟢 混淆: Salamander"
  singbox_h2_obfs_gecko: "🟦 混淆: Gecko"
  singbox_h2_gecko_note: "⚠️ Gecko 为实验性混淆，需客户端支持：sing-box ≥1.14.0 / mihomo ≥1.19.26 / 官方 hysteria ≥2.9.2。v2rayN/Xray 系客户端可能无法连接。"
```

- [ ] **Step 2: Add keys to en.yml**

Append after the `singbox_h2_hop_disable` line (line 72):

```yaml
  singbox_h2_obfs_type_title: "🚀 <b>Hysteria2 Batch Create</b>\n\n🌐 Protocol: %{0}\n📊 Quantity: %{1}\n\nSelect obfuscation type:"
  singbox_h2_obfs_type_salamander: "🟢 Salamander (Recommended)"
  singbox_h2_obfs_type_gecko: "🟦 Gecko (Experimental, stronger anti-DPI)"
  singbox_h2_obfs_salamander: "🟢 Obfs: Salamander"
  singbox_h2_obfs_gecko: "🟦 Obfs: Gecko"
  singbox_h2_gecko_note: "⚠️ Gecko is experimental; requires client support: sing-box ≥1.14.0 / mihomo ≥1.19.26 / official hysteria ≥2.9.2. v2rayN/Xray-based clients may fail to connect."
```

- [ ] **Step 3: Add keys to ja.yml**

Append after the `singbox_h2_hop_disable` line (line 72):

```yaml
  singbox_h2_obfs_type_title: "🚀 <b>Hysteria2 バッチ作成</b>\n\n🌐 プロトコル: %{0}\n📊 数量: %{1}\n\n難読化タイプを選択:"
  singbox_h2_obfs_type_salamander: "🟢 Salamander (推奨)"
  singbox_h2_obfs_type_gecko: "🟦 Gecko (実験的、DPI耐性が高い)"
  singbox_h2_obfs_salamander: "🟢 難読化: Salamander"
  singbox_h2_obfs_gecko: "🟦 難読化: Gecko"
  singbox_h2_gecko_note: "⚠️ Geckoは実験的な難読化です。クライアント対応が必要: sing-box ≥1.14.0 / mihomo ≥1.19.26 / 公式 hysteria ≥2.9.2。v2rayN/Xray系では接続できない場合があります。"
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles (rust_i18n resolves keys at compile time via `t!`).

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml rust/aegis/src/resources/i18n/ja.yml
git commit -m "feat: add i18n keys for hysteria2 gecko obfuscation"
```

---

### Task 6: Telegram UI flow — type selection screen and status mapping

**Files:**
- Modify: `rust/aegis/src/shared/handlers/singbox.rs` (callback chain)

**Interfaces:**
- Consumes: keys from Task 5; `Hysteria2ObfsType` import from Task 4.
- Produces: `sb_h2_obfs_type:{ip}:{count}` handler; `sb_h2_obfs` enable button now routes to it; `sb_h2_hop` title maps obfs `0|1|2`; `sb_h2_exec` parses obfs `1|2` into `Option<Hysteria2ObfsType>` (and `0` → `None`).

- [ ] **Step 1: Route the enable button to the type screen**

In the `sb_h2_obfs:` block, change the enable button's data from `sb_h2_hop:...:1` to:

```rust
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_enable").into(),
                    data: format!("sb_h2_obfs_type:{}:{}", ip_ver, count),
                }],
```

(Disable button stays `sb_h2_hop:{}:{}:0`; Back stays `sb_h2_init`.)

- [ ] **Step 2: Add the `sb_h2_obfs_type` handler block**

Insert this block between the `sb_h2_obfs:` block and the `sb_h2_hop:` block:

```rust
        d if d.starts_with("sb_h2_obfs_type:") => {
            let parts: Vec<&str> = d
                .strip_prefix("sb_h2_obfs_type:")
                .unwrap_or("")
                .split(':')
                .collect();
            if parts.len() != 2 {
                event
                    .adapter
                    .answer_callback(
                        &event.target,
                        &event.callback_id,
                        Some(t!("menu.singbox_param_error").into_owned()),
                    )
                    .await?;
                return Ok(HandlerAction::Done);
            }
            let ip_ver = parts[0];
            let count = parts[1];
            let ip_display = if ip_ver == "4" { "IPv4" } else { "IPv6" };

            let rows = vec![
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_type_salamander").into(),
                    data: format!("sb_h2_hop:{}:{}:1", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.singbox_h2_obfs_type_gecko").into(),
                    data: format!("sb_h2_hop:{}:{}:2", ip_ver, count),
                }],
                vec![InlineButton {
                    text: t!("menu.back_user").into(),
                    data: format!("sb_h2_obfs:{}:{}", ip_ver, count),
                }],
            ];

            event
                .adapter
                .edit_message(
                    &event.target,
                    &event.msg_id,
                    MessageContent {
                        text: t!(
                            "menu.singbox_h2_obfs_type_title",
                            "0" => ip_display,
                            "1" => count
                        )
                        .into_owned(),
                        markup: Some(Markup { buttons: rows }),
                    },
                )
                .await?;

            Ok(HandlerAction::Done)
        }
```

- [ ] **Step 3: Map obfs status in the `sb_h2_hop` title**

In the `sb_h2_hop:` block, replace the `title` format's obfs part with a match:

```rust
            let obfs_status = match obfs_enabled {
                "2" => t!("menu.singbox_h2_obfs_gecko").to_string(),
                "1" => t!("menu.singbox_h2_obfs_salamander").to_string(),
                _ => t!("menu.singbox_h2_obfs_disabled").to_string(),
            };
            let title = format!(
                "⚡ {} | {} {}\n\n{}",
                ip_display,
                t!("menu.singbox_h2_qty", "0" => count),
                obfs_status,
                t!("menu.singbox_h2_hop_title"),
            );
```

(`let obfs_enabled = parts[2];` stays a `&str`; it is forwarded unchanged into `sb_h2_exec`.)

- [ ] **Step 4: Parse `0|1|2` in `sb_h2_exec`**

In the `sb_h2_exec:` block, replace the Task 4 minimal mapping with the full three-state mapping (keep `hopping_enabled: bool = parts[3] == "1";`):

```rust
            let obfs_type = match parts[2] {
                "1" => Some(Hysteria2ObfsType::Salamander),
                "2" => Some(Hysteria2ObfsType::Gecko),
                _ => None,
            };
```

- [ ] **Step 5: Verify**

Run: `cargo nextest run`
Expected: compile OK, all tests pass. (No handler unit-test baseline exists.)

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/shared/handlers/singbox.rs
git commit -m "feat: add hysteria2 obfuscation type selection to telegram flow"
```

---

### Task 7: Gecko compatibility note in batch result

**Files:**
- Modify: `rust/aegis/src/shared/handlers/singbox.rs` (`send_singbox_batch_result` + both call sites)

**Interfaces:**
- Consumes: `menu.singbox_h2_gecko_note` (Task 5), `obfs_type` in `sb_h2_exec` (Task 6).
- Produces: `send_singbox_batch_result(adapter, target, protocol_name, result, note: Option<&str>)` — appends `note` to the header message when present. TUIC call site passes `None`.

- [ ] **Step 1: Change the signature and append the note**

In `send_singbox_batch_result`, change the signature:

```rust
pub async fn send_singbox_batch_result(
    adapter: Arc<dyn BotAdapter>,
    target: &TargetId,
    protocol_name: &str,
    result: &BatchCreationResult,
    note: Option<&str>,
) -> anyhow::Result<()> {
```

Change the header construction to a `let mut` String and append the note:

```rust
    let mut header_msg = format!(
        "✅ <b>{} 批量创建完成</b>\n\n已创建 {} 个配置:\n📁 配置文件: <code>{}</code>\n\n",
        protocol_name,
        result.created_count,
        result.config_file.as_deref().unwrap_or("未知")
    );
    if let Some(note) = note {
        header_msg.push_str(note);
        header_msg.push_str("\n\n");
    }
```

- [ ] **Step 2: Wire the note in the hysteria2 exec handler**

In the `sb_h2_exec:` handler, **before** the `let adapter = event.adapter.clone();` / `tokio::spawn` block (so it is computed before `obfs_type` is captured by the async move), add the flag:

```rust
            let is_gecko = matches!(obfs_type, Some(Hysteria2ObfsType::Gecko));
```

and replace the `Ok(result)` arm body:

```rust
                    Ok(result) => {
                        let note = if is_gecko {
                            Some(t!("menu.singbox_h2_gecko_note").to_string())
                        } else {
                            None
                        };
                        if let Err(e) = send_singbox_batch_result(
                            adapter.clone(),
                            &target,
                            "Hysteria2",
                            &result,
                            note.as_deref(),
                        )
                        .await
                        {
                            log::warn!("发送批量创建结果失败: {}", e);
                        }
                    }
```

- [ ] **Step 3: Update the TUIC call site**

The TUIC path (later in the same file) passes `None`:

```rust
                            send_singbox_batch_result(adapter.clone(), &target, "TUIC", &result, None)
```

- [ ] **Step 4: Verify**

Run: `cargo nextest run`
Expected: compile OK, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/shared/handlers/singbox.rs
git commit -m "feat: append gecko client-compatibility note to batch result"
```

---

### Task 8: Full quality gate and review

**Files:**
- None (verification + wrap-up)

- [ ] **Step 1: Run the complete rust-lint-format gate**

Run from `rust/aegis`:

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run && \
cargo test --doc
```

Expected: all four commands succeed; zero Clippy warnings; all tests pass (673+3 new hysteria2 tests, 1 skip).

- [ ] **Step 2: Diff review against the spec**

Review `git diff main..HEAD --stat` — must be exactly the 6 files in the spec's Files Touched table. Verify in the diff:
- No stringly-typed obfs values outside `Hysteria2ObfsType::as_str()`.
- `sb_h2_exec` obfs encoding 0/1/2 matches the spec's callback chain.
- Gecko JSON contains exactly `min_packet_size: 512` / `max_packet_size: 1200`.
- No version check added, no packet-size UI added (spec non-goals).

- [ ] **Step 3: Commit any remaining formatting fixes**

```bash
git add -A
git commit -m "chore: formatting fixes"   # only if Step 1 changed files
```

- [ ] **Step 4: Final state**

Run: `git log --oneline main..HEAD`
Expected: 8 commits (Tasks 1–7 + optional formatting), no merge commits, feature branch `feat/hy2-gecko-obfs` ready for `requesting-code-review` then `finishing-a-development-branch`.
