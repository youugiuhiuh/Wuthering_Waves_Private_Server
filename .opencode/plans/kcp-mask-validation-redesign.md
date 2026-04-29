# KCP Mask Stacking Validation Redesign

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Redesign KCP mask stacking validation to match Xray-core source code constraints, with automatic canonical ordering so users don't need to understand layer ordering.

**Architecture:** Replace current strict incremental validation (`is_compatible_add`) with a two-phase system: (1) compatibility checks that only enforce mutual exclusivity and hard Xray-core constraints, and (2) a `canonical_order()` function that automatically sorts masks into the correct Xray-core layer order. UI displays masks without sequence numbers, showing "outer → inner" labels instead.

**Tech Stack:** Rust, Teloxide (Telegram bot framework), serde_json

---

## Background

### Current Problems

The current `is_compatible_add()` and `validate_stack()` methods in `config.rs` have several bugs and missing constraints based on analysis of Xray-core source code (`transport/internet/finalmask/`):

1. **Bug: Sudoku can only be added to empty stack** — `is_compatible_add()` line 380 rejects Sudoku when `stack_len > 0`, preventing valid stacks like `[noise, sudoku]` or `[srtp, aes128gcm, sudoku]`
2. **Missing: xicmp must be outermost** — Xray-core enforces `level == 0` for xicmp, will return runtime error otherwise
3. **Missing: xdns must be outermost** — xdns replaces the entire transport, logically requires position 0
4. **Missing: xdns/xicmp mutual exclusion** — Both require position 0, cannot coexist
5. **Missing: Encryption must be last in header group** — In `headerManagerConn`, encryption headers process the remaining buffer after prior headers. If encryption precedes appearance headers, it corrupts those header bytes
6. **Missing: Header total size overflow check** — UDPSize=4096 byte buffer limit; excessive headers cause silent packet drops
7. **Incorrect: Hard 5-layer limit** — Xray-core has no fixed layer limit; practical limit is header size overflow
8. **Incorrect: Salamander classification** — Salamander is a `headerConn` (aggregated with headers), not a standalone wrapper like Sudoku

### Xray-core Source Code Enforced Constraints

| Constraint | Source | Enforcement |
|---|---|---|
| Sudoku must be innermost (`level == levelCount-1`) | `sudoku/config.go` | Hard runtime error |
| xicmp must be outermost (`level == 0`) | `xicmp/config.go` | Hard runtime error |
| xdns should be outermost | `xdns/config.go` (commented out) | Logical requirement |
| Salamander is headerConn (Size=8) | `salamander/salamander.go` | Architecture |
| mkcp-original is headerConn (Size=6) | `mkcp/original/conn.go` | Architecture |
| mkcp-aes128gcm is headerConn (Size=12+16) | `mkcp/aes128gcm/conn.go` | Architecture |
| All header-* are headerConn | `header/*/conn.go` | Architecture |
| UDPSize = 4096 byte buffer | `finalmask.go` | Silent drop on overflow |
| Encryption must be last in header group | `finalmask.go` headerManagerConn | Data corruption if violated |

### Xray-core Stacking Architecture

The `UdpmaskManager` processes masks in array order (index 0 = outermost). Masks fall into two categories:

**Category A: Header Aggregation** — masks implementing `headerConn` interface. Grouped into a single `headerManagerConn` that aggregates all header bytes into a contiguous block. Processed forward for reading (stripping from front), reverse for writing (prepending to front).

**Category B: Standalone Wrapping** — masks NOT implementing `headerConn`. These wrap the entire PacketConn independently.

The correct canonical order (outermost to innermost):

```
Position 0: Transport replacement (xdns OR xicmp, at most 1)
Position 1: Noise (standalone wrapper)
Position 2: Disguise headers (header-dns, header-wechat, etc.)
Position 3: Obfuscation header (salamander)
Position 4: Encryption headers (mkcp-original, mkcp-aes128gcm)
Position 5: Sudoku (standalone wrapper, MUST be innermost)
```

---

## Design

### Phase 1: New KcpMask Methods (config.rs)

#### 1.1 Classification Methods

```rust
pub fn is_transport_replacement(&self) -> bool {
    matches!(self, KcpMask::Xdns { .. } | KcpMask::Xicmp { .. })
}

pub fn is_xdns(&self) -> bool {
    matches!(self, KcpMask::Xdns { .. })
}

pub fn is_xicmp(&self) -> bool {
    matches!(self, KcpMask::Xicmp { .. })
}

pub fn is_header_conn(&self) -> bool {
    matches!(
        self,
        KcpMask::MkcpOriginal
        | KcpMask::MkcpAes128Gcm { .. }
        | KcpMask::Salamander { .. }
        | KcpMask::HeaderDns { .. }
        | KcpMask::HeaderWechat
        | KcpMask::HeaderSrtp
        | KcpMask::HeaderUtp
        | KcpMask::HeaderDtls
        | KcpMask::HeaderWireguard
        | KcpMask::HeaderCustom
    )
}

pub fn is_disguise_header(&self) -> bool {
    matches!(
        self,
        KcpMask::HeaderDns { .. }
        | KcpMask::HeaderWechat
        | KcpMask::HeaderSrtp
        | KcpMask::HeaderUtp
        | KcpMask::HeaderDtls
        | KcpMask::HeaderWireguard
        | KcpMask::HeaderCustom
    )
}

pub fn header_size(&self) -> Option<usize> {
    match self {
        KcpMask::MkcpOriginal => Some(6),         // FNV1a(4) + length(2)
        KcpMask::MkcpAes128Gcm { .. } => Some(28), // nonce(12) + GCM tag(16)
        KcpMask::Salamander { .. } => Some(8),     // salt
        KcpMask::HeaderDns { .. } => Some(30),      // DNS header ~12B + domain
        KcpMask::HeaderWechat => Some(13),
        KcpMask::HeaderSrtp => Some(4),
        KcpMask::HeaderUtp => Some(4),
        KcpMask::HeaderDtls => Some(13),
        KcpMask::HeaderWireguard => Some(4),
        KcpMask::HeaderCustom => Some(4),  // minimum estimate
        _ => None,  // noise, sudoku, xdns, xicmp are not headerConn
    }
}
```

#### 1.2 Canonical Ordering

```rust
fn sort_priority(&self) -> u8 {
    match self {
        KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } => 0,  // transport replacement (outermost)
        KcpMask::Noise => 10,                                  // noise padding
        KcpMask::HeaderDns { .. }
        | KcpMask::HeaderWechat
        | KcpMask::HeaderSrtp
        | KcpMask::HeaderUtp
        | KcpMask::HeaderDtls
        | KcpMask::HeaderWireguard
        | KcpMask::HeaderCustom => 20,                         // disguise headers
        KcpMask::Salamander { .. } => 30,                      // obfuscation header
        KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. } => 40,  // encryption (last in header group)
        KcpMask::Sudoku { .. } => 50,                          // innermost
    }
}

pub fn canonical_order(masks: &[KcpMask]) -> Vec<KcpMask> {
    let mut ordered: Vec<KcpMask> = masks.to_vec();
    ordered.sort_by_key(|m| m.sort_priority());
    ordered
}
```

#### 1.3 Compatibility Check (replaces `is_compatible_add`)

```rust
pub fn is_compatible_with(&self, existing: &[KcpMask]) -> Result<(), String> {
    // xdns and xicmp are mutually exclusive
    if self.is_transport_replacement() {
        if existing.iter().any(|m| m.is_transport_replacement()) {
            let name = if self.is_xdns() { "XDNS" } else { "XICMP" };
            let other = if self.is_xdns() { "XICMP" } else { "XDNS" };
            return Err(format!("{}和{}不能同时使用", name, other));
        }
    }

    // Only one encryption layer
    if self.is_encryption() {
        if existing.iter().any(|m| m.is_encryption()) {
            return Err("重复的加密层".to_string());
        }
    }

    // Only one Sudoku
    if self.is_sudoku() {
        if existing.iter().any(|m| m.is_sudoku()) {
            return Err("重复的Sudoku".to_string());
        }
    }

    // No duplicate mask codes
    if existing.iter().any(|m| m.code() == self.code()) {
        return Err(format!("重复的{}", self.display_name()));
    }

    // mkcp-original alone is insecure
    if matches!(self, KcpMask::MkcpOriginal) && existing.is_empty() {
        return Err("mKCP Original单独使用安全性低，建议配合伪装层使用".to_string());
    }

    // Header size overflow check (reserve 2400 for sudoku's 4-6x expansion)
    let total_header: usize = existing.iter()
        .filter_map(|m| m.header_size())
        .sum::<usize>()
        + self.header_size().unwrap_or(0);
    let sudoku_reserve = if self.is_sudoku() || existing.iter().any(|m| m.is_sudoku()) {
        2400
    } else {
        0
    };
    if total_header + sudoku_reserve > 3800 {
        return Err(format!("header总大小{}字节过大，可能超出UDP包限制(4096字节)", total_header));
    }

    Ok(())
}
```

#### 1.4 Stack Validation (redesigned)

```rust
pub fn validate_stack(masks: &[KcpMask]) -> Result<(), String> {
    if masks.is_empty() {
        return Err("请至少选择1层遮罩".to_string());
    }

    // Sudoku must be last (innermost)
    if masks.iter().any(|m| m.is_sudoku()) {
        if !masks.last().map(|m| m.is_sudoku()).unwrap_or(false) {
            return Err("Sudoku必须是最后一层(最内侧)".to_string());
        }
    }

    // xicmp must be first (outermost)
    if masks.iter().any(|m| m.is_xicmp()) {
        if !masks.first().map(|m| m.is_xicmp()).unwrap_or(false) {
            return Err("XICMP必须是最外层(第一个遮罩)".to_string());
        }
    }

    // xdns must be first (outermost)
    if masks.iter().any(|m| m.is_xdns()) {
        if !masks.first().map(|m| m.is_xdns()).unwrap_or(false) {
            return Err("XDNS必须是最外层(第一个遮罩)".to_string());
        }
    }

    // xdns and xicmp cannot coexist
    if masks.iter().any(|m| m.is_xdns()) && masks.iter().any(|m| m.is_xicmp()) {
        return Err("XDNS和XICMP不能同时使用".to_string());
    }

    // Only one encryption layer
    if masks.iter().filter(|m| m.is_encryption()).count() > 1 {
        return Err("重复的加密层".to_string());
    }

    // Encryption must be after all disguise/obfuscation headers
    if let Some(enc_idx) = masks.iter().position(|m| m.is_encryption()) {
        for m in &masks[enc_idx + 1..] {
            if m.is_disguise_header() || matches!(m, KcpMask::Salamander { .. }) {
                return Err("加密层之后不能有伪装/混淆层(加密层应紧贴数据)".to_string());
            }
        }
    }

    // mkcp-original alone is insecure
    if masks.len() == 1 && matches!(masks[0], KcpMask::MkcpOriginal) {
        return Err("mKCP Original单独使用安全性低，建议配合伪装层使用".to_string());
    }

    // Header size overflow check
    let total_header: usize = masks.iter().filter_map(|m| m.header_size()).sum();
    let sudoku_reserve = if masks.iter().any(|m| m.is_sudoku()) { 2400 } else { 0 };
    if total_header + sudoku_reserve > 3800 {
        return Err(format!(
            "header总大小{}字节过大，可能超出UDP包限制(4096字节)",
            total_header
        ));
    }

    Ok(())
}
```

### Phase 2: UI Changes (main.rs)

#### 2.1 Remove Sequence Numbers from Stack Display

Replace `1️⃣ 2️⃣ 3️⃣` numbering with canonical-order display:

```
BEFORE:
📋 当前遮罩栈:
1️⃣ 🔐 mKCP AES-128-GCM
2️⃣ 🎭 SRTP 伪装
3️⃣ 🔢 Sudoku

AFTER:
📋 当前遮罩栈 (外层→内层):
🎭 SRTP 伪装 → 🔐 mKCP AES-128-GCM → 🔢 Sudoku
```

Implement by calling `KcpMask::canonical_order()` on the current mask codes when displaying.

#### 2.2 Update Callback Data Format

Keep the same callback format (comma-separated codes, order-independent). When generating the final JSON config in `u_kcp_ip` handler, call `canonical_order()` before `build_kcp_inbound()`.

```rust
// In u_kcp_ip handler:
let masks = KcpMask::parse_codes(&codes)?;
let ordered = KcpMask::canonical_order(&masks);
KcpMask::validate_stack(&ordered)?;
let inbound = ConfigManager::build_kcp_inbound(/* ... */, &ordered, ip_version);
```

#### 2.3 Update `u_kcp_mcat` Compatibility Check

Replace `is_compatible_add` with `is_compatible_with`:

```rust
// BEFORE:
match mask.is_compatible_add(&current_mask_refs) { ... }

// AFTER:
match mask.is_compatible_with(&current_masks) {
    Ok(()) => { /* show ✅ add button */ }
    Err(e) => { /* show ⛔ button with reason */ }
}
```

#### 2.4 Update `u_kcp_more` Category Buttons

When xdns or xicmp is already in the stack:
- Show `⛔ 传输替换 (已添加xdns)` or similar
- When transport replacement category is selected, disable the already-chosen option with `☑️`

#### 2.5 Update `u_kcp_add` to Use `is_compatible_with`

```rust
if let Some(m) = KcpMask::from_code(code) {
    if let Err(e) = m.is_compatible_with(&[]) {
        // show error
    }
    // ... rest of handler
}
```

#### 2.6 Update `u_kcp_push` to Use `is_compatible_with`

Same pattern: parse existing masks, then check `new_mask.is_compatible_with(&current_masks)`.

#### 2.7 Update `u_kcp_done` to Use `validate_stack` + `canonical_order`

Before proceeding to IP selection, validate and reorder:

```rust
let masks = KcpMask::parse_codes(&codes)?;
let ordered = KcpMask::canonical_order(&masks);
if let Err(e) = KcpMask::validate_stack(&ordered) {
    // show error
}
// use ordered codes for IP selection
let ordered_codes: Vec<String> = ordered.iter().map(|m| m.code().to_string()).collect();
let ordered_str = ordered_codes.join(",");
```

### Phase 3: Remove Old Code

- Remove `is_compatible_add()` method
- Remove `status(&self) -> &'static str` if it exists (was never used)

Keep: `is_encryption()`, `is_sudoku()` as convenience methods used by UI code.

### Phase 4: Tests (18 tests)

| # | Test Name | Validates |
|---|-----------|-----------|
| 1 | `test_canonical_order_transport_replacement_first` | xdns/xicmp at position 0 |
| 2 | `test_canonical_order_sudoku_last` | Sudoku at end |
| 3 | `test_canonical_order_encryption_after_disguise` | Encryption after disguise headers |
| 4 | `test_canonical_order_salamander_after_disguise_before_encryption` | Salamander between disguise and encryption |
| 5 | `test_canonical_order_noise_after_transport_before_headers` | Noise at position 1 |
| 6 | `test_compatible_with_xdns_xicmp_exclusive` | Cannot add xdns when xicmp exists |
| 7 | `test_compatible_with_duplicate_encryption` | Cannot add two encryption layers |
| 8 | `test_compatible_with_duplicate_sudoku` | Cannot add two sudoku |
| 9 | `test_compatible_with_duplicate_header` | Cannot add same header type twice |
| 10 | `test_compatible_with_mkcp_original_alone` | Warning for mkcp-original alone |
| 11 | `test_validate_stack_sudoku_not_last` | Error when sudoku not last |
| 12 | `test_validate_stack_xicmp_not_first` | Error when xicmp not first |
| 13 | `test_validate_stack_xdns_not_first` | Error when xdns not first |
| 14 | `test_validate_stack_encryption_before_disguise` | Error when encryption before disguise |
| 15 | `test_validate_stack_header_overflow` | Error when header size exceeds limit |
| 16 | `test_header_size_values` | All header_size() values correct |
| 17 | `test_canonical_order_full_stack` | Full stack reorders correctly |
| 18 | `test_canonical_order_simple_stack` | Simple stack reorders correctly |

---

## Files to Modify

| File | Changes |
|---|---|
| `rust/tgbot/src/logic/config.rs` | Add `is_transport_replacement()`, `is_xdns()`, `is_xicmp()`, `is_header_conn()`, `is_disguise_header()`, `header_size()`, `sort_priority()`, `canonical_order()`, `is_compatible_with()`. Redesign `validate_stack()`. Remove `is_compatible_add()`. |
| `rust/tgbot/src/main.rs` | Update `u_kcp_add`, `u_kcp_push`, `u_kcp_mcat`, `u_kcp_more`, `u_kcp_done`, `u_kcp_ip` to use new methods. Add canonical ordering. Update stack display format. |

## Xray-core Reference Summary

| Mask | headerConn? | Size (bytes) | Position Requirement |
|---|---|---|---|
| mkcp-original | YES | 6 | Must be last in header group |
| mkcp-aes128gcm | YES | 28 (12 nonce + 16 tag) | Must be last in header group |
| noise | NO | 0 | Any (standalone wrapper) |
| salamander | YES | 8 | After disguise headers, before encryption |
| sudoku | NO | 0 (4-6x expansion) | MUST be innermost (Xray error) |
| header-dns | YES | ~30 | After noise, before encryption |
| header-wechat | YES | 13 | After noise, before encryption |
| header-srtp | YES | 4 | After noise, before encryption |
| header-utp | YES | 4 | After noise, before encryption |
| header-dtls | YES | 13 | After noise, before encryption |
| header-wireguard | YES | 4 | After noise, before encryption |
| header-custom | YES (conditional) | variable | After noise, before encryption |
| xdns | NO | 0 | MUST be outermost (level 0) |
| xicmp | NO | 0 | MUST be outermost (level 0) |