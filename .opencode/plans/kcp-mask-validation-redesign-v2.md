# KCP Mask Validation Redesign v2 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite KCP mask stacking validation to match Xray-core source code constraints, add canonical ordering, fix header_size values, add is_header_conn, update UI.

**Architecture:** Two-phase: (1) config.rs data model + validation methods, (2) main.rs UI handlers. All values verified against Xray-core source. canonical_order applied at u_kcp_done finalization.

**Tech Stack:** Rust, Teloxide, serde_json

---

## File Structure

| File | Responsibility |
|---|---|
| `rust/tgbot/src/logic/config.rs` | KcpMask enum, classification methods, header_size, canonical_order, is_compatible_with, validate_stack, dns_header_size |
| `rust/tgbot/src/main.rs` | UI callback handlers: u_kcp_add, u_kcp_more, u_kcp_mcat, u_kcp_push, u_kcp_done |

---

### Task 1: Add `is_header_conn()` method

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:391` (after `is_disguise_header`)

- [ ] **Step 1: Write the failing test**

Add in the test module (after the existing tests, before the closing `}` of the mod):

```rust
#[test]
fn test_is_header_conn_classification() {
    // headerConn masks (from Xray-core)
    assert!(KcpMask::MkcpOriginal.is_header_conn());
    assert!(KcpMask::MkcpAes128Gcm { password: "test".to_string() }.is_header_conn());
    assert!(KcpMask::Salamander { password: "test".to_string() }.is_header_conn());
    assert!(KcpMask::HeaderDns { domain: "example.com".to_string() }.is_header_conn());
    assert!(KcpMask::HeaderWechat.is_header_conn());
    assert!(KcpMask::HeaderSrtp.is_header_conn());
    assert!(KcpMask::HeaderUtp.is_header_conn());
    assert!(KcpMask::HeaderDtls.is_header_conn());
    assert!(KcpMask::HeaderWireguard.is_header_conn());
    assert!(KcpMask::HeaderCustom.is_header_conn());

    // NOT headerConn (standalone wrappers)
    assert!(!KcpMask::Noise.is_header_conn());
    assert!(!KcpMask::Sudoku { password: "test".to_string() }.is_header_conn());
    assert!(!KcpMask::Xdns { domains: vec![], resolvers: vec![] }.is_header_conn());
    assert!(!KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }.is_header_conn());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust/tgbot && cargo test test_is_header_conn_classification -- --nocapture 2>&1`
Expected: FAIL — `method is_header_conn not found`

- [ ] **Step 3: Add `is_header_conn()` method**

Insert after `is_disguise_header()` (line 391):

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust/tgbot && cargo test test_is_header_conn_classification -- --nocapture 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "feat: add is_header_conn method matching Xray-core headerConn interface"
```

---

### Task 2: Fix `is_disguise_header()` to include `HeaderCustom`

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:381-391`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_is_disguise_header_includes_custom() {
    assert!(KcpMask::HeaderCustom.is_disguise_header());
    assert!(KcpMask::HeaderDns { domain: "example.com".to_string() }.is_disguise_header());
    assert!(KcpMask::HeaderWechat.is_disguise_header());
    assert!(KcpMask::HeaderSrtp.is_disguise_header());
    assert!(KcpMask::HeaderUtp.is_disguise_header());
    assert!(KcpMask::HeaderDtls.is_disguise_header());
    assert!(KcpMask::HeaderWireguard.is_disguise_header());

    // Non-disguise headers
    assert!(!KcpMask::MkcpOriginal.is_disguise_header());
    assert!(!KcpMask::Salamander { password: "test".to_string() }.is_disguise_header());
    assert!(!KcpMask::Noise.is_disguise_header());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust/tgbot && cargo test test_is_disguise_header_includes_custom -- --nocapture 2>&1`
Expected: FAIL — `assertion failed: KcpMask::HeaderCustom.is_disguise_header()`

- [ ] **Step 3: Fix `is_disguise_header()`**

Replace the current `is_disguise_header` method (lines 381-391) with:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd rust/tgbot && cargo test test_is_disguise_header_includes_custom -- --nocapture 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "fix: add HeaderCustom to is_disguise_header matching Xray-core headerConn classification"
```

---

### Task 3: Fix `header_size()` values and add `dns_header_size()`

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:393-410`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn test_header_size_values() {
    // headerConn masks — values from Xray-core source
    assert_eq!(KcpMask::MkcpOriginal.header_size(), Some(6));
    assert_eq!(KcpMask::MkcpAes128Gcm { password: "test".to_string() }.header_size(), Some(28));
    assert_eq!(KcpMask::Salamander { password: "test".to_string() }.header_size(), Some(8));
    assert_eq!(KcpMask::HeaderDns { domain: "example.com".to_string() }.header_size(), Some(29));
    assert_eq!(KcpMask::HeaderWechat.header_size(), Some(13));
    assert_eq!(KcpMask::HeaderSrtp.header_size(), Some(4));
    assert_eq!(KcpMask::HeaderUtp.header_size(), Some(4));
    assert_eq!(KcpMask::HeaderDtls.header_size(), Some(13));
    assert_eq!(KcpMask::HeaderWireguard.header_size(), Some(4));
    assert_eq!(KcpMask::HeaderCustom.header_size(), Some(4));

    // Standalone wrappers — None (not headerConn)
    assert_eq!(KcpMask::Noise.header_size(), None);
    assert_eq!(KcpMask::Sudoku { password: "test".to_string() }.header_size(), None);
    assert_eq!(KcpMask::Xdns { domains: vec![], resolvers: vec![] }.header_size(), None);
    assert_eq!(KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }.header_size(), None);
}

#[test]
fn test_dns_header_size_dynamic() {
    // dns_header_size: 12 (header) + labels + 1 (null) + 4 (type/class)
    assert_eq!(KcpMask::HeaderDns { domain: "a.io".to_string() }.header_size(), Some(22));
    assert_eq!(KcpMask::HeaderDns { domain: "example.com".to_string() }.header_size(), Some(29));
    assert_eq!(KcpMask::HeaderDns { domain: "sub.domain.example.com".to_string() }.header_size(), Some(42));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust/tgbot && cargo test test_header_size_values -- --nocapture 2>&1`
Expected: FAIL — multiple assertion failures (wrong values)

- [ ] **Step 3: Replace `header_size()` and add `dns_header_size()`**

Replace the current `header_size` method (lines 393-410) with:

```rust
    pub fn header_size(&self) -> Option<usize> {
        match self {
            KcpMask::MkcpOriginal => Some(6),
            KcpMask::MkcpAes128Gcm { .. } => Some(28),
            KcpMask::Salamander { .. } => Some(8),
            KcpMask::HeaderDns { domain } => Some(dns_header_size(domain)),
            KcpMask::HeaderWechat => Some(13),
            KcpMask::HeaderSrtp => Some(4),
            KcpMask::HeaderUtp => Some(4),
            KcpMask::HeaderDtls => Some(13),
            KcpMask::HeaderWireguard => Some(4),
            KcpMask::HeaderCustom => Some(4),
            KcpMask::Noise => None,
            KcpMask::Sudoku { .. } => None,
            KcpMask::Xdns { .. } => None,
            KcpMask::Xicmp { .. } => None,
        }
    }

    fn dns_header_size(domain: &str) -> usize {
        let mut size = 12;
        for label in domain.split('.') {
            size += 1 + label.len();
        }
        size += 1;
        size += 4;
        size
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust/tgbot && cargo test test_header_size_values test_dns_header_size_dynamic -- --nocapture 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "fix: correct header_size values to match Xray-core (6 for original, 28 for aes128gcm, dynamic for dns)"
```

---

### Task 4: Add `sort_priority()` and `canonical_order()` methods

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs` (after `dns_header_size`)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn test_canonical_order_transport_replacement_first() {
    let masks = vec![
        KcpMask::HeaderSrtp,
        KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
    ];
    let ordered = KcpMask::canonical_order(&masks);
    assert!(ordered[0].is_xicmp());
}

#[test]
fn test_canonical_order_sudoku_last() {
    let masks = vec![
        KcpMask::Sudoku { password: "test".to_string() },
        KcpMask::HeaderSrtp,
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
    ];
    let ordered = KcpMask::canonical_order(&masks);
    assert!(ordered.last().unwrap().is_sudoku());
}

#[test]
fn test_canonical_order_encryption_after_disguise() {
    let masks = vec![
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::HeaderSrtp,
    ];
    let ordered = KcpMask::canonical_order(&masks);
    let enc_pos = ordered.iter().position(|m| m.is_encryption()).unwrap();
    let dis_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
    assert!(dis_pos < enc_pos, "disguise header should come before encryption");
}

#[test]
fn test_canonical_order_salamander_after_disguise_before_encryption() {
    let masks = vec![
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::Salamander { password: "test".to_string() },
        KcpMask::HeaderDns { domain: "example.com".to_string() },
    ];
    let ordered = KcpMask::canonical_order(&masks);
    let dis_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
    let sal_pos = ordered.iter().position(|m| matches!(m, KcpMask::Salamander { .. })).unwrap();
    let enc_pos = ordered.iter().position(|m| m.is_encryption()).unwrap();
    assert!(dis_pos < sal_pos, "disguise should be before salamander");
    assert!(sal_pos < enc_pos, "salamander should be before encryption");
}

#[test]
fn test_canonical_order_noise_after_transport_before_headers() {
    let masks = vec![
        KcpMask::HeaderSrtp,
        KcpMask::Noise,
        KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
    ];
    let ordered = KcpMask::canonical_order(&masks);
    let xicmp_pos = ordered.iter().position(|m| m.is_xicmp()).unwrap();
    let noise_pos = ordered.iter().position(|m| matches!(m, KcpMask::Noise)).unwrap();
    let header_pos = ordered.iter().position(|m| m.is_disguise_header()).unwrap();
    assert!(xicmp_pos < noise_pos, "xicmp should be before noise");
    assert!(noise_pos < header_pos, "noise should be before disguise header");
}

#[test]
fn test_canonical_order_full_stack() {
    let masks = vec![
        KcpMask::Sudoku { password: "test".to_string() },
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::HeaderDns { domain: "example.com".to_string() },
        KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
        KcpMask::Noise,
        KcpMask::Salamander { password: "test".to_string() },
    ];
    let ordered = KcpMask::canonical_order(&masks);
    assert!(ordered[0].is_xicmp());
    assert!(matches!(ordered[1], KcpMask::Noise));
    assert!(ordered[2].is_disguise_header());
    assert!(matches!(ordered[3], KcpMask::Salamander { .. }));
    assert!(ordered[4].is_encryption());
    assert!(ordered[5].is_sudoku());
}

#[test]
fn test_canonical_order_simple_stack() {
    let masks = vec![
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::HeaderSrtp,
    ];
    let ordered = KcpMask::canonical_order(&masks);
    assert!(ordered[0].is_disguise_header());
    assert!(ordered[1].is_encryption());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust/tgbot && cargo test test_canonical_order -- --nocapture 2>&1`
Expected: FAIL — `method canonical_order not found`

- [ ] **Step 3: Add `sort_priority()` and `canonical_order()` methods**

Insert after `dns_header_size`:

```rust
    fn sort_priority(&self) -> u8 {
        match self {
            KcpMask::Xicmp { .. } => 0,
            KcpMask::Xdns { .. } => 1,
            KcpMask::Noise => 10,
            KcpMask::HeaderDns { .. }
            | KcpMask::HeaderWechat
            | KcpMask::HeaderSrtp
            | KcpMask::HeaderUtp
            | KcpMask::HeaderDtls
            | KcpMask::HeaderWireguard
            | KcpMask::HeaderCustom => 20,
            KcpMask::Salamander { .. } => 30,
            KcpMask::MkcpOriginal
            | KcpMask::MkcpAes128Gcm { .. } => 40,
            KcpMask::Sudoku { .. } => 50,
        }
    }

    pub fn canonical_order(masks: &[KcpMask]) -> Vec<KcpMask> {
        let mut ordered: Vec<KcpMask> = masks.to_vec();
        ordered.sort_by_key(|m| m.sort_priority());
        ordered
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust/tgbot && cargo test test_canonical_order -- --nocapture 2>&1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "feat: add sort_priority and canonical_order for Xray-core layer ordering"
```

---

### Task 5: Fix `is_compatible_with()` — add xicmp position check

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:412-455`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_compatible_with_xicmp_not_first() {
    let existing = vec![KcpMask::HeaderSrtp];
    let xicmp = KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 };
    assert!(xicmp.is_compatible_with(&existing).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd rust/tgbot && cargo test test_compatible_with_xicmp_not_first -- --nocapture 2>&1`
Expected: FAIL — assertion fails (xicmp is currently allowed with non-empty existing)

- [ ] **Step 3: Add xicmp position check to `is_compatible_with()`**

At the beginning of `is_compatible_with()` body (line 413, before the transport_replacement check), add:

```rust
        if self.is_xicmp() && !existing.is_empty() {
            return Err("XICMP必须是最外层(第一个添加的遮罩)".to_string());
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust/tgbot && cargo test test_compatible_with -- --nocapture 2>&1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "fix: add xicmp position check to is_compatible_with (must be first)"
```

---

### Task 6: Fix `validate_stack()` — remove xdns constraint, remove 5-layer limit, add sudoku duplicate check

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:457-510`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn test_validate_stack_xdns_not_enforced_first() {
    // xdns has no position constraint (commented out in Xray-core)
    let masks = vec![
        KcpMask::HeaderSrtp,
        KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] },
    ];
    assert!(KcpMask::validate_stack(&masks).is_ok());
}

#[test]
fn test_validate_stack_sudoku_duplicate() {
    // duplicate sudoku should be rejected
    let masks = vec![
        KcpMask::Sudoku { password: "test1".to_string() },
        KcpMask::Sudoku { password: "test2".to_string() },
    ];
    assert!(KcpMask::validate_stack(&masks).is_err());
}

#[test]
fn test_validate_stack_no_layer_limit() {
    // 7 layers should be allowed (no 5-layer hard limit)
    let masks = vec![
        KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] },
        KcpMask::Noise,
        KcpMask::HeaderDns { domain: "a.com".to_string() },
        KcpMask::HeaderSrtp,
        KcpMask::Salamander { password: "test".to_string() },
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::Sudoku { password: "test".to_string() },
    ];
    assert!(KcpMask::validate_stack(&masks).is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust/tgbot && cargo test test_validate_stack_xdns_not_enforced test_validate_stack_sudoku_duplicate test_validate_stack_no_layer_limit -- --nocapture 2>&1`
Expected: `test_validate_stack_xdns_not_enforced` FAILS, `test_validate_stack_sudoku_duplicate` may pass or fail, `test_validate_stack_no_layer_limit` may pass

- [ ] **Step 3: Rewrite `validate_stack()`**

Replace the current `validate_stack` method (lines 457-510) with:

```rust
    pub fn validate_stack(masks: &[KcpMask]) -> Result<(), String> {
        if masks.is_empty() {
            return Err("请至少选择1层遮罩".to_string());
        }
        if masks.iter().any(|m| m.is_xicmp()) {
            if !masks.first().map(|m| m.is_xicmp()).unwrap_or(false) {
                return Err("XICMP必须是最外层(第一个遮罩)".to_string());
            }
        }
        if masks.iter().any(|m| m.is_xdns()) && masks.iter().any(|m| m.is_xicmp()) {
            return Err("XDNS和XICMP不能同时使用".to_string());
        }
        if masks.iter().filter(|m| m.is_encryption()).count() > 1 {
            return Err("重复的加密层".to_string());
        }
        if masks.iter().filter(|m| m.is_sudoku()).count() > 1 {
            return Err("重复的Sudoku".to_string());
        }
        if masks.iter().any(|m| m.is_sudoku()) {
            if !masks.last().map(|m| m.is_sudoku()).unwrap_or(false) {
                return Err("Sudoku必须是最后一层(最内侧)".to_string());
            }
        }
        if let Some(enc_idx) = masks.iter().position(|m| m.is_encryption()) {
            for m in &masks[enc_idx + 1..] {
                if m.is_disguise_header() || matches!(m, KcpMask::Salamander { .. }) {
                    return Err("加密层之后不能有伪装/混淆层(加密层应紧贴数据)".to_string());
                }
            }
        }
        if masks.len() == 1 && matches!(masks[0], KcpMask::MkcpOriginal) {
            return Err("mKCP Original单独使用安全性低，建议配合伪装层使用".to_string());
        }
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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust/tgbot && cargo test test_validate_stack -- --nocapture 2>&1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "fix: remove xdns position constraint, remove layer limit, add sudoku duplicate check in validate_stack"
```

---

### Task 7: Add remaining 5 compatibility/validation tests

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs` (test module)

- [ ] **Step 1: Add all remaining tests**

Add these tests to the test module:

```rust
#[test]
fn test_compatible_with_xdns_xicmp_exclusive() {
    let existing = vec![KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 }];
    let xdns = KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] };
    assert!(xdns.is_compatible_with(&existing).is_err());

    let existing2 = vec![KcpMask::Xdns { domains: vec!["example.com".to_string()], resolvers: vec![] }];
    let xicmp = KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 };
    assert!(xicmp.is_compatible_with(&existing2).is_err());
}

#[test]
fn test_compatible_with_duplicate_encryption() {
    let existing = vec![KcpMask::MkcpAes128Gcm { password: "test".to_string() }];
    let dup = KcpMask::MkcpOriginal;
    assert!(dup.is_compatible_with(&existing).is_err());
}

#[test]
fn test_compatible_with_duplicate_sudoku() {
    let existing = vec![KcpMask::Sudoku { password: "test1".to_string() }];
    let dup = KcpMask::Sudoku { password: "test2".to_string() };
    assert!(dup.is_compatible_with(&existing).is_err());
}

#[test]
fn test_compatible_with_duplicate_header() {
    let existing = vec![KcpMask::HeaderSrtp];
    let dup = KcpMask::HeaderSrtp;
    assert!(dup.is_compatible_with(&existing).is_err());
}

#[test]
fn test_compatible_with_mkcp_original_alone() {
    let alone = KcpMask::MkcpOriginal;
    assert!(alone.is_compatible_with(&[]).is_err());

    let with_header = KcpMask::MkcpOriginal;
    let existing = vec![KcpMask::HeaderSrtp];
    assert!(with_header.is_compatible_with(&existing).is_ok());
}

#[test]
fn test_validate_stack_xicmp_not_first() {
    let masks = vec![
        KcpMask::HeaderSrtp,
        KcpMask::Xicmp { listen_ip: "0.0.0.0".to_string(), id: 0 },
    ];
    assert!(KcpMask::validate_stack(&masks).is_err());
}

#[test]
fn test_validate_stack_sudoku_not_last() {
    let masks = vec![
        KcpMask::Sudoku { password: "test".to_string() },
        KcpMask::HeaderSrtp,
    ];
    assert!(KcpMask::validate_stack(&masks).is_err());
}

#[test]
fn test_validate_stack_encryption_before_disguise() {
    let masks = vec![
        KcpMask::MkcpAes128Gcm { password: "test".to_string() },
        KcpMask::HeaderSrtp,
    ];
    // After canonical_order this becomes [HeaderSrtp, MkcpAes128Gcm] which is valid.
    // But in this order (encryption before disguise), it's invalid.
    assert!(KcpMask::validate_stack(&masks).is_err());
}

#[test]
fn test_validate_stack_header_overflow() {
    // Build a stack with many large headers to exceed 3800 bytes
    let masks: Vec<KcpMask> = (0..200)
        .map(|_| KcpMask::HeaderDns { domain: "sub.domain.example.com".to_string() })
        .collect();
    assert!(KcpMask::validate_stack(&masks).is_err());
}
```

- [ ] **Step 2: Run all new tests**

Run: `cd rust/tgbot && cargo test test_compatible_with test_validate_stack -- --nocapture 2>&1`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "test: add compatibility and validation tests matching Xray-core constraints"
```

---

### Task 8: Remove 5-layer hard limit in `u_kcp_more` and `u_kcp_push`

**Files:**
- Modify: `rust/tgbot/src/main.rs:2759-2835` (u_kcp_more handler)
- Modify: `rust/tgbot/src/main.rs:2923-3001` (u_kcp_push handler)

- [ ] **Step 1: Remove 5-layer limit from `u_kcp_more`**

In the `u_kcp_more` handler (around line 2762-2763), remove:

```rust
    let stack_len = current_masks.len();
    let stack_full = stack_len >= 5;
```

And remove `has_transport_replacement` since it's no longer used for disabling "dis" category.

Change the `disabled_reason` match arm (line 2781) from:
```rust
            "dis" if has_transport_replacement => Some("传输替换已存在"),
```
Remove this line entirely.

Change the `if stack_full` block (around line 2785-2789) — remove the entire `stack_full` branch:
```rust
        if stack_full {
            // REMOVE THIS ENTIRE BLOCK
        } else if let Some(reason) = disabled_reason {
```
to:
```rust
        if let Some(reason) = disabled_reason {
```

Change the display text (around line 2827) from:
```
➕ <b>选择要添加的遮罩类别</b> (已达{}层，最多5层)
```
to:
```
➕ <b>选择要添加的遮罩类别</b>
```
Remove `existing_codes.len()` from the format arguments.

Also remove the `has_transport_replacement` variable since it's no longer used:
```rust
    let has_transport_replacement = current_masks.iter().any(|m| m.is_transport_replacement());
```
Remove this line.

- [ ] **Step 2: Remove 5-layer limit from `u_kcp_push`**

In the `u_kcp_push` handler, remove the `if codes.len() < 5` condition from the "continue adding" button (around line 2975):

Change from:
```rust
    if codes.len() < 5 {
        buttons.push(vec![InlineKeyboardButton::callback(
            "➕ 继续添加遮罩层",
            format!("u_kcp_more:{}", new_stack),
        )]);
    }
```
to:
```rust
    buttons.push(vec![InlineKeyboardButton::callback(
        "➕ 继续添加遮罩层",
        format!("u_kcp_more:{}", new_stack),
    )]);
```

Change the display text (around line 2995) from:
```rust
    if codes.len() < 5 { "➕ 可以继续添加，或完成配置" } else { "✅ 已达最大层数(5层)" }
```
to:
```rust
    "➕ 可以继续添加，或完成配置"
```

Remove the `u_kcp_add` "最多5层" text (around line 2732-2733):

Change from:
```
➕ 可以继续添加(最多5层)，或完成配置
```
to:
```
➕ 可以继续添加，或完成配置
```

- [ ] **Step 3: Build to verify compilation**

Run: `cd rust/tgbot && cargo build 2>&1`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "fix: remove 5-layer hard limit (Xray-core has no such limit) and fix dis category logic"
```

---

### Task 9: Add `canonical_order()` call in `u_kcp_done` and change display format

**Files:**
- Modify: `rust/tgbot/src/main.rs:3002-3073` (u_kcp_done handler)

- [ ] **Step 1: Modify `u_kcp_done` to use canonical_order**

Replace the `u_kcp_done` handler body (lines 3002-3073) with:

```rust
d if d.starts_with("u_kcp_done:") => {
    let mask_codes_str = d.strip_prefix("u_kcp_done:").unwrap_or("");
    let codes: Vec<&str> = mask_codes_str.split(',').collect();

    if codes.is_empty() {
        bot.answer_callback_query(q.id.clone())
            .text("❌ 请至少选择1层遮罩")
            .await?;
        return Ok(());
    }

    let masks: Vec<KcpMask> = codes
        .iter()
        .filter_map(|c| KcpMask::from_code(c))
        .collect();

    let ordered = KcpMask::canonical_order(&masks);

    if let Err(e) = KcpMask::validate_stack(&ordered) {
        bot.answer_callback_query(q.id.clone())
            .text(format!("❌ {}", e))
            .await?;
        return Ok(());
    }

    let stack_display: Vec<String> = ordered.iter().map(|m| {
        format!("{}", m.display_name())
    }).collect();

    let ordered_codes: Vec<String> = ordered.iter().map(|m| m.code().to_string()).collect();
    let ordered_str = ordered_codes.join(",");

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
    let mut buttons = vec![vec![
        InlineKeyboardButton::callback(
            "🌐 IPv4 (0.0.0.0)",
            format!("u_kcp_ip:{}:4", ordered_str),
        ),
    ]];
    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("u_kcp_ip:{}:6", ordered_str),
        ));
    }
    buttons.push(vec![
        InlineKeyboardButton::callback(
            "🔄 双栈 IPv4优先",
            format!("u_kcp_ip:{}:s4", ordered_str),
        ),
    ]);
    buttons.push(vec![
        InlineKeyboardButton::callback(
            "🔄 双栈 IPv6优先",
            format!("u_kcp_ip:{}:s6", ordered_str),
        ),
    ]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回",
        format!("u_kcp_more:{}", mask_codes_str),
    )]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>KCP 配置</b>\n\n\
             📋 <b>遮罩栈 (外层→内层):</b>\n{}\n\n\
             ⬇️ <b>请选择网络协议版本:</b>",
            stack_display.join(" → ")
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Build to verify compilation**

Run: `cd rust/tgbot && cargo build 2>&1`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "feat: add canonical_order to u_kcp_done, display stack as outer→inner format"
```

---

### Task 10: Show error reasons in `u_kcp_mcat`

**Files:**
- Modify: `rust/tgbot/src/main.rs:2870-2883` (u_kcp_mcat error handling)

- [ ] **Step 1: Change the Err branch to show error reason**

In the `u_kcp_mcat` handler, change the `Err(_)` match arm (around line 2877-2881) from:

```rust
                Err(_) => {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("⛔ {}", mask.display_name()),
                        format!("noop:⛔:{}", code),
                    )]);
                }
```

to:

```rust
                Err(e) => {
                    buttons.push(vec![InlineKeyboardButton::callback(
                        format!("⛔ {} ({})", mask.display_name(), e),
                        format!("noop:⛔:{}", code),
                    )]);
                }
```

- [ ] **Step 2: Build to verify compilation**

Run: `cd rust/tgbot && cargo build 2>&1`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "feat: show compatibility error reasons in u_kcp_mcat mask list"
```

---

### Task 11: Run full test suite and verify build

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

Run: `cd rust/tgbot && cargo test 2>&1`
Expected: ALL tests pass

- [ ] **Step 2: Build release**

Run: `cd rust/tgbot && cargo build --release 2>&1`
Expected: Compiles successfully

- [ ] **Step 3: Final commit (if any fixes needed)**

Only commit if fixes were applied during verification.

---

## Self-Review

### 1. Spec Coverage

| Spec Section | Task |
|---|---|
| 1.1 Classification Methods (`is_header_conn`) | Task 1 |
| 1.1 `is_disguise_header` fix (add HeaderCustom) | Task 2 |
| 1.1 `header_size` fix + `dns_header_size` | Task 3 |
| 1.2 `sort_priority` + `canonical_order` | Task 4 |
| 1.3 `is_compatible_with` xicmp check | Task 5 |
| 1.4 `validate_stack` rewrite | Task 6 |
| Tests (18 tests) | Tasks 1-7 (test_ methods alongside implementation tasks) |
| 2.1 Remove 5-layer limit | Task 8 |
| 2.2 canonical_order in u_kcp_done + → display | Task 9 |
| 2.3 Fix "dis" category disable logic | Task 8 |
| 2.4 Show error reasons in mcat | Task 10 |
| 3 Remove old code | Not needed (is_compatible_add already removed) |
| Full build + test verification | Task 11 |

### 2. Placeholder Scan

No TBD, TODO, or placeholder patterns found.

### 3. Type Consistency

- `is_header_conn()` returns `bool` — consistent with other classification methods
- `header_size()` returns `Option<usize>` — `None` for standalone, `Some(usize)` for headerConn — consistent
- `dns_header_size()` takes `&str` and returns `usize` — consistent with header_size returning `Some(dns_header_size(domain))`
- `canonical_order()` takes `&[KcpMask]` and returns `Vec<KcpMask>` — used in u_kcp_done correctly
- `sort_priority()` returns `u8` — used as key in `sort_by_key`
- `is_compatible_with()` takes `&[KcpMask]` and returns `Result<(), String>` — consistent with current API
- `validate_stack()` takes `&[KcpMask]` and returns `Result<(), String>` — consistent