# One-Click Deploy: TLS CDN Reality Bonus Padding

> **Goal:** When one-click deploy uses TLS mode with CDN (Cloudflare/Route53), automatically pad with Reality XHTTP ports so total always reaches 20.

**Architecture:** Route53's `cdn_ports()` gets `[443]` (was empty). In `run_one_click()`, after `batch_create_xhttp_tls_enhanced` succeeds, always pad with `batch_create_xhttp_reality_enhanced(20 - tls_created)`. Dead `else` branch removed from `batch_create_xhttp_tls_enhanced` since both providers now have non-empty `cdn_ports`.

**Tech Stack:** Rust, rust_i18n YAML locales

## Design

### Current Behavior

| Provider | cdn_ports | TLS ports | Reality padding | Total |
|----------|-----------|-----------|----------------|-------|
| Cloudflare | `[443, 8443, 2053, 2083, 2087, 2096]` | 6 | none | **6** |
| Route53 | `[]` → 20 random TLS ports | 20 | none | 20 |

Route53's 20 random TLS ports are unreachable (only 443 is firewalled).

### New Behavior

| Provider | cdn_ports | TLS ports | Reality padding | Total |
|----------|-----------|-----------|----------------|-------|
| Cloudflare | 6 | 6 | `20 - 6 = 14` Reality XHTTP | **20** |
| Route53 | `[443]` | 1 | `20 - 1 = 19` Reality XHTTP | **20** |

### Files (7 total)

| File | Change |
|------|--------|
| `rust/aegis/src/core/types.rs:31` | `Route53 => &[443]` (was `&[]`) |
| `rust/aegis/src/core/types.rs:131` | Update test |
| `rust/aegis/src/core/xray/xhttp.rs:111-182` | Remove dead `else` branch (both providers now non-empty `cdn_ports`) |
| `rust/aegis/src/shared/handlers/ops.rs:634-668` | Add Reality bonus padding after TLS creation, unconditional |
| `rust/aegis/src/resources/i18n/zh.yml` | New key `ops.deploy_created_xhttp_bonus` |
| `rust/aegis/src/resources/i18n/en.yml` | New key `ops.deploy_created_xhttp_bonus` |
| `rust/aegis/src/resources/i18n/ja.yml` | New key `ops.deploy_created_xhttp_bonus` |

### xhttp.rs: Remove else branch

```rust
// Before (lines 111-182):
if !cdn_ports.is_empty() {
    for (i, &cdn_port) in cdn_ports.iter().enumerate() { ... }
} else {
    for i in 0..20 { ... }  // ← dead code, delete
}

// After:
for (i, &cdn_port) in cdn_ports.iter().enumerate() { ... }
```

Both callers (one-click `ops.rs:638` and standalone `xray.rs:59`) go through provider selection (CF or Route53), so `cdn_ports` always non-empty.

### ops.rs: Add Reality padding

After `batch_create_xhttp_tls_enhanced` succeeds:

```rust
let tls_created = tls_result.created_count;
all_links.extend(tls_result.links);
// send existing TLS message ...

// Pad with Reality XHTTP to reach 20 total
let reality_count = 20_usize.saturating_sub(tls_created);
let reality_result =
    ConfigManager::batch_create_xhttp_reality_enhanced(reality_count, ip_version).await?;
all_links.extend(reality_result.links);
// send bonus message with ops.deploy_created_xhttp_bonus
```

No if-guard needed—TLS mode always has a CDN provider with non-empty `cdn_ports`.

### i18n Key: `ops.deploy_created_xhttp_bonus`

| Lang | Value |
|------|-------|
| zh | `✅ 额外创建了 %{1} 个 Reality XHTTP (%{0}) 配置\n📁 %{2}` |
| en | `✅ Additionally created %{1} Reality XHTTP (%{0}) config(s)\n📁 %{2}` |
| ja | `✅ 追加で %{1} 個の Reality XHTTP（%{0}）を作成しました\n📁 %{2}` |

### No Changes

- Progress steps stay 1-10
- Reality-only (no domain) path — already creates 20, no change
- Standalone TLS caller — gets CDN-only ports, no padding (by design)
