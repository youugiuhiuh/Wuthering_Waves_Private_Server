# Hysteria2 Gecko Obfuscation Design

## Scope

Add `gecko` as a selectable QUIC obfuscation type for Hysteria2 nodes created
by `rust/aegis`. Today the Telegram flow only offers enable/disable of the
hardcoded `salamander` obfuscation. This design introduces a type choice
(salamander / gecko) while keeping `salamander` as the default.

## Background

sing-box 1.14.0 added `gecko` as a second `obfs.type` value for the Hysteria2
inbound/outbound, alongside `salamander` (sources: sing-box docs, changelog
v1.14.0-alpha.26, SagerNet/sing-box issue #4171, apernet/hysteria PROTOCOL.md).

Facts established during research:

- Config-wise, `obfs.type` accepts exactly one of `salamander` / `gecko`.
  They are mutually exclusive; there is no stacked config.
- Mechanism-wise, gecko builds on salamander: QUIC long-header (handshake)
  packets are fragmented into 2-8 randomly sized chunks with random padding,
  each chunk wrapped in salamander; short-header (data) packets pass through
  plain salamander. Selecting gecko therefore requires no separate salamander
  config; both share `obfs.password`.
- gecko-only fields: `obfs.min_packet_size` (default 512) and
  `obfs.max_packet_size` (default 1200), sing-box 1.14.0+. No community
  guidance exists for tuning them; all tutorials use the defaults.
- Client compatibility for gecko links is version-dependent (sing-box
  >= 1.14.0, mihomo >= 1.19.26, official hysteria >= 2.9.2; Xray-family
  support is evolving). The result message must carry a note when gecko nodes
  are generated.

## Decisions (approved)

1. **UI form**: keep the enable/disable toggle; after enabling, ask for the
   type (salamander / gecko). Salamander is the recommended/first button.
2. **Version gate**: none. The project always installs the latest sing-box
   (installer caps to the latest 5 releases), so 1.14.0+ is assumed present.
3. **Packet sizes**: not exposed. Emit the defaults 512 / 1200 as constants.
4. **Implementation approach**: replace the `enable_obfs: bool` with an
   `Option<Hysteria2ObfsType>` enum (no stringly-typed API).

## Design

### 1. Data model — `rust/aegis/src/core/singbox/hysteria2.rs`

- Add `pub enum Hysteria2ObfsType { Salamander, Gecko }` with derives
  `Debug, Clone, Copy, PartialEq`.
- Add constants:
  `GECKO_DEFAULT_MIN_PACKET_SIZE: usize = 512`,
  `GECKO_DEFAULT_MAX_PACKET_SIZE: usize = 1200`.
- Change `Hysteria2Config.obfs_type` from `Option<String>` to
  `Option<Hysteria2ObfsType>`.
- Change `with_obfs` to take `obfs_type: Hysteria2ObfsType` instead of
  `String`.
- `to_inbound_json`: when obfs is set, emit
  `obfs.type = "salamander" | "gecko"` and `obfs.password`; for gecko also
  emit `obfs.min_packet_size = 512` and `obfs.max_packet_size = 1200`.
- Add `impl Hysteria2ObfsType` helper returning the JSON/URI string value
  (`"salamander"` / `"gecko"`) to keep string conversion in one place.

### 2. Batch creation — `rust/aegis/src/core/singbox/hy2_batch.rs`

- Change `batch_create_hysteria2(count, ip_version, enable_obfs: bool,
  enable_hopping: bool)` to
  `batch_create_hysteria2(count, ip_version, obfs_type:
  Option<Hysteria2ObfsType>, enable_hopping: bool)`.
- Remove the hardcoded `"salamander"`; pass `obfs_type` through to
  `with_obfs`.
- Client link selection branches on `obfs_type` (Some) and `enable_hopping`
  as today.

### 3. Client links — `rust/aegis/src/core/singbox/hysteria2.rs`

- `to_client_link_with_obfs` and `to_client_link_with_hopping_and_obfs`
  currently hardcode `&obfs=salamander`. Parameterize the `obfs=` value from
  `self.obfs_type` (`"salamander"` / `"gecko"`).
- Non-obfs link methods unchanged.

### 4. UI flow — `rust/aegis/src/shared/handlers/singbox.rs`

New callback chain:

```
sb_h2_obfs:{ip}:{count}          # enable / disable (unchanged entry)
  ├─ disable → sb_h2_hop:{ip}:{count}:0        # 0 = none
  └─ enable  → sb_h2_obfs_type:{ip}:{count}    # NEW step
                ├─ salamander → sb_h2_hop:{ip}:{count}:1
                └─ gecko      → sb_h2_hop:{ip}:{count}:2
sb_h2_hop:{ip}:{count}:{obfs}   # hop enable/disable (obfs now 0|1|2)
  └─ sb_h2_exec:{ip}:{count}:{obfs}:{hop}
```

- `sb_h2_obfs_type` screen: two buttons (Salamander first, then Gecko) plus
  Back. Title reflects the ip/count context.
- `sb_h2_hop` title: map obfs value to "disabled" / "salamander" / "gecko".
- `sb_h2_exec`: parse obfs as `0|1|2` and map to
  `Option<Hysteria2ObfsType>`; gecko result appends a client-compatibility
  note (i18n key).

### 5. i18n — `rust/aegis/src/resources/i18n/{zh,en,ja}.yml`

Add keys:

- `singbox_h2_obfs_type_title` — type selection screen title
- `singbox_h2_obfs_type_salamander` / `singbox_h2_obfs_type_gecko` — buttons
- `singbox_h2_obfs_gecko` — "gecko" status label
- `singbox_h2_gecko_note` — client compatibility note shown in results

Existing `singbox_h2_obfs_enable/disable/enabled/disabled/title` keys remain.

### 6. Tests — `rust/aegis/src/core/singbox/hysteria2.rs`

Follow existing `#[cfg(test)] mod tests` pattern (`test-arrange-act-assert`,
descriptive names). New cases:

- gecko JSON: `obfs.type == "gecko"`, `min_packet_size == 512`,
  `max_packet_size == 1200`, password present.
- salamander JSON regression: `obfs.type == "salamander"`, no
  min/max_packet_size keys.
- gecko client link contains `obfs=gecko` and `obfs-password=...`
  (plain and hopping variants).
- salamander client link regression contains `obfs=salamander`.
- No-obfs config emits no `obfs` object (existing behavior preserved).

No new unit tests for `hy2_batch.rs` or the handler (no baseline exists; they
depend on system services).

### 7. Error handling / edge cases

- `sb_h2_exec` with an unrecognized obfs value falls back to "none" rather
  than failing the whole batch (defensive parse, mirroring the existing
  `unwrap_or` style); invalid callback payloads still answer
  `menu.singbox_param_error`.
- Gecko links are only produced when the caller chose gecko; the
  compatibility note is attached to the batch result message for gecko runs.
- No version check at runtime (decision 2); no packet-size exposure
  (decision 3).

## Files Touched

| File | Change |
|---|---|
| `rust/aegis/src/core/singbox/hysteria2.rs` | enum, constants, field type, JSON gen, links |
| `rust/aegis/src/core/singbox/hy2_batch.rs` | signature, remove hardcode |
| `rust/aegis/src/shared/handlers/singbox.rs` | callback chain, type screen, exec parsing |
| `rust/aegis/src/resources/i18n/zh.yml` | new keys |
| `rust/aegis/src/resources/i18n/en.yml` | new keys |
| `rust/aegis/src/resources/i18n/ja.yml` | new keys |

## Verification

From the `rust/aegis` crate root (`rust/aegis` and `rust/version-sync` are independent crates, not a Cargo workspace), before commit:

```bash
cargo fmt && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo nextest run && \
cargo test --doc
```

All four commands must pass with zero Clippy warnings.

## Non-Goals

- Exposing `min_packet_size` / `max_packet_size` in the UI.
- Runtime sing-box version checking or installer minimum-version pins.
- Changing the no-obfs and hop-only paths.
- Persisting obfs-type choices across sessions.
