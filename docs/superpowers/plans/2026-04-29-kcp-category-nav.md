# KCP Category Navigation UI Implementation Plan

**Goal:** Replace KCP flat categorized list with 4-button category navigation (加密层, 混淆层, 伪装层, 扩展层), each leading to a sub-menu with brief descriptions and add buttons.

**Architecture:** Pure UI layer change. Add helper methods to KcpMask, rewrite callback handlers in main.rs.

**Tech Stack:** Rust, teloxide

---

### Task 1: Add `brief()` and `category_code()` to KcpMask

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:138-165` (add after `detail()` method)

- [ ] **Step 1: Write tests for `brief()` and `category_code()`**

In `rust/tgbot/src/logic/config.rs`, after existing `test_kcp_mask_from_code_invalid` test (around line 1863), add:

```rust
#[cfg(test)]
mod kcp_mask_navigation_tests {
    use super::*;

    #[test]
    fn test_kcp_mask_brief_all_variants() {
        let variants = KcpMask::all_variants();
        assert_eq!(variants.len(), 14);
        for m in variants {
            let brief = m.brief();
            assert!(!brief.is_empty(), "brief should not be empty for {:?}", m);
        }
    }

    #[test]
    fn test_kcp_mask_category_code_all_variants() {
        let variants = KcpMask::all_variants();
        for m in variants {
            let code = m.category_code();
            assert!(
                code == "enc" || code == "obf" || code == "dis" || code == "ext",
                "category_code should be enc/obf/dis/ext for {:?}, got {}",
                m,
                code
            );
            assert_eq!(m.category_code(), KcpMask::from_code(code).map(|x| x.category_code()).unwrap_or(""));
        }
    }

    #[test]
    fn test_kcp_mask_category_code_unique() {
        let mut codes: Vec<&str> = KcpMask::all_variants().iter().map(|m| m.category_code()).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), 4, "should have exactly 4 unique category codes");
    }

    #[test]
    fn test_kcp_mask_category_code_matches_category() {
        for m in KcpMask::all_variants() {
            let cat = m.category();
            let code = m.category_code();
            match code {
                "enc" => assert!(cat.contains("加密"), "enc should map to 加密层: {}", cat),
                "obf" => assert!(cat.contains("混淆"), "obf should map to 混淆层: {}", cat),
                "dis" => assert!(cat.contains("伪装"), "dis should map to 伪装层: {}", cat),
                "ext" => assert!(cat.contains("扩展"), "ext should map to 扩展层: {}", cat),
                _ => panic!("unexpected code: {}", code),
            }
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test kcp_mask_navigation_tests --package tgbot 2>&1 | head -40`
Expected: FAIL — methods not yet defined

- [ ] **Step 3: Implement `brief()` method**

Add to `impl KcpMask` block after `detail()` method (around line 155):

```rust
pub fn brief(&self) -> &'static str {
    match self {
        KcpMask::MkcpOriginal => "轻量级XOR混淆，仅FNV1a校验",
        KcpMask::MkcpAes128Gcm { .. } => "AES-128-GCM认证加密，推荐首选",
        KcpMask::Noise => "随机噪声填充，抗流量分析",
        KcpMask::Salamander { .. } => "蝾螈混淆协议，抗深度包检测",
        KcpMask::Sudoku { .. } => "数独混淆算法，强度更高",
        KcpMask::HeaderDns { .. } => "DNS查询流量伪装",
        KcpMask::HeaderWechat => "微信视频通话流量伪装",
        KcpMask::HeaderSrtp => "SRTP音视频流媒体伪装",
        KcpMask::HeaderUtp => "BitTorrent uTP协议伪装",
        KcpMask::HeaderDtls => "DTLS 1.2加密数据包伪装",
        KcpMask::HeaderWireguard => "WireGuard VPN流量伪装",
        KcpMask::Xdns { .. } => "扩展DNS，支持自定义域名和解析器",
        KcpMask::Xicmp { .. } => "ICMP数据包伪装，极端限制网络适用",
        KcpMask::HeaderCustom => "自定义UDP头部格式",
    }
}
```

- [ ] **Step 4: Implement `category_code()` method**

Add after `brief()`:

```rust
pub fn category_code(&self) -> &'static str {
    match self {
        KcpMask::MkcpOriginal | KcpMask::MkcpAes128Gcm { .. } => "enc",
        KcpMask::Noise | KcpMask::Salamander { .. } | KcpMask::Sudoku { .. } => "obf",
        KcpMask::HeaderDns { .. }
        | KcpMask::HeaderWechat
        | KcpMask::HeaderSrtp
        | KcpMask::HeaderUtp
        | KcpMask::HeaderDtls
        | KcpMask::HeaderWireguard => "dis",
        KcpMask::Xdns { .. } | KcpMask::Xicmp { .. } | KcpMask::HeaderCustom => "ext",
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test kcp_mask_navigation_tests --package tgbot 2>&1 | tail -20`
Expected: PASS (4 tests)

- [ ] **Step 6: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "feat: add brief() and category_code() methods to KcpMask"
```

---

### Task 2: Add `variants_by_category()` and `category_from_code()` helpers

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:300-309` (add after `all_variants()`)

- [ ] **Step 1: Write tests**

Add to `kcp_mask_navigation_tests` module:

```rust
#[test]
fn test_variants_by_category_count() {
    assert_eq!(KcpMask::variants_by_category("enc").len(), 2);   // mo, ma
    assert_eq!(KcpMask::variants_by_category("obf").len(), 3);  // no, sa, su
    assert_eq!(KcpMask::variants_by_category("dis").len(), 6);  // hd, hw, hs, hu, hdt, hwg
    assert_eq!(KcpMask::variants_by_category("ext").len(), 3);   // xd, xi, hc
}

#[test]
fn test_variants_by_category_invalid() {
    assert!(KcpMask::variants_by_category("invalid").is_empty());
    assert!(KcpMask::variants_by_category("").is_empty());
}

#[test]
fn test_category_from_code() {
    assert_eq!(KcpMask::category_from_code("enc"), Some("🔐 加密层"));
    assert_eq!(KcpMask::category_from_code("obf"), Some("🌀 混淆层"));
    assert_eq!(KcpMask::category_from_code("dis"), Some("🎭 伪装层"));
    assert_eq!(KcpMask::category_from_code("ext"), Some("⚡ 扩展层"));
    assert_eq!(KcpMask::category_from_code("xx"), None);
}

#[test]
fn test_category_from_code_roundtrip() {
    for code in ["enc", "obf", "dis", "ext"] {
        let cat = KcpMask::category_from_code(code).unwrap();
        for m in KcpMask::variants_by_category(code) {
            assert_eq!(m.category(), cat);
            assert_eq!(m.category_code(), code);
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test kcp_mask_navigation_tests --package tgbot 2>&1 | head -40`
Expected: FAIL — methods not defined

- [ ] **Step 3: Implement `variants_by_category()` and `category_from_code()`**

Add after `parse_codes()` (around line 309):

```rust
pub fn variants_by_category(code: &str) -> Vec<Self> {
    Self::all_variants()
        .into_iter()
        .filter(|m| m.category_code() == code)
        .collect()
}

pub fn category_from_code(code: &str) -> Option<&'static str> {
    match code {
        "enc" => Some("🔐 加密层"),
        "obf" => Some("🌀 混淆层"),
        "dis" => Some("🎭 伪装层"),
        "ext" => Some("⚡ 扩展层"),
        _ => None,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test kcp_mask_navigation_tests --package tgbot 2>&1 | tail -20`
Expected: PASS (8 tests total)

- [ ] **Step 5: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "feat: add variants_by_category() and category_from_code() helpers"
```

---

### Task 3: Rewrite `u_kcp_init` handler with 4 category buttons

**Files:**
- Modify: `rust/tgbot/src/main.rs:2627-2666`

- [ ] **Step 1: Write the replacement handler**

Replace the entire `u_kcp_init` handler body (lines 2627-2666) with:

```rust
"u_kcp_init" => {
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    buttons.push(vec![
        InlineKeyboardButton::callback(
            "🔐 加密层 (2)",
            "u_kcp_cat:enc",
        ),
        InlineKeyboardButton::callback(
            "🌀 混淆层 (3)",
            "u_kcp_cat:obf",
        ),
    ]);
    buttons.push(vec![
        InlineKeyboardButton::callback(
            "🎭 伪装层 (6)",
            "u_kcp_cat:dis",
        ),
        InlineKeyboardButton::callback(
            "⚡ 扩展层 (3)",
            "u_kcp_cat:ext",
        ),
    ]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回",
        "m_xray_mgmt",
    )]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        "🚀 <b>KCP (mKCP+FinalMask) 配置</b>\n\n\
         ✨ <b>特点:</b>\n\
         • 基于 mKCP 协议的可靠传输\n\
         • FinalMask 多层遮罩任意叠加(1-5层)\n\
         • 支持加密、混淆、伪装、扩展四大类遮罩\n\n\
         📋 <b>步骤 1: 选择遮罩类别</b>\n\
         ⚠️ 至少选择1层，建议加密层+伪装层组合",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Build to check syntax**

Run: `cargo build --package tgbot 2>&1 | grep -E "^error|u_kcp"`
Expected: No errors related to our code

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "refactor: replace u_kcp_init flat list with 4 category buttons"
```

---

### Task 4: Add `u_kcp_cat` handler for category sub-menu

**Files:**
- Modify: `rust/tgbot/src/main.rs` — add handler between `u_kcp_init` and the next handler

- [ ] **Step 1: Write the `u_kcp_cat` handler**

Insert after `u_kcp_init` handler closing brace (after line 2666) and before `d if d.starts_with("u_kcp_sel:")`:

```rust
d if d.starts_with("u_kcp_cat:") => {
    let cat_code = d.strip_prefix("u_kcp_cat:").unwrap_or("enc");
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("未知");

    let variants = KcpMask::variants_by_category(cat_code);
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for mask in &variants {
        buttons.push(vec![InlineKeyboardButton::callback(
            format!("✅ {}", mask.display_name()),
            format!("u_kcp_add:{}", mask.code()),
        )]);
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回分类",
        "u_kcp_init",
    )]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("{}\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "<b>{}</b> — 选择要添加的遮罩\n\n{}",
            cat_name, mask_list
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Build to check syntax**

Run: `cargo build --package tgbot 2>&1 | grep -E "^error"`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "feat: add u_kcp_cat handler for category sub-menu with brief descriptions"
```

---

### Task 5: Remove `u_kcp_sel` handler

**Files:**
- Modify: `rust/tgbot/src/main.rs:2667-2692`

- [ ] **Step 1: Remove the handler**

Delete the entire `d if d.starts_with("u_kcp_sel:") => { ... }` block (lines 2667-2692).

- [ ] **Step 2: Build to check syntax**

Run: `cargo build --package tgbot 2>&1 | grep -E "^error"`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "refactor: remove u_kcp_sel handler (no longer needed with category nav)"
```

---

### Task 6: Rewrite `u_kcp_more` handler with 4 category buttons

**Files:**
- Modify: `rust/tgbot/src/main.rs:2729-2780`

- [ ] **Step 1: Write the replacement handler**

Replace the `u_kcp_more` handler with:

```rust
d if d.starts_with("u_kcp_more:") => {
    let existing = d.strip_prefix("u_kcp_more:").unwrap_or("");
    let existing_codes: Vec<&str> = existing.split(',').collect();

    let stack_display: Vec<String> = existing_codes.iter().enumerate().map(|(i, c)| {
        let m = KcpMask::from_code(c);
        format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
    }).collect();

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    let cat_counts = [
        ("enc", "🔐 加密层", KcpMask::variants_by_category("enc").len()),
        ("obf", "🌀 混淆层", KcpMask::variants_by_category("obf").len()),
        ("dis", "🎭 伪装层", KcpMask::variants_by_category("dis").len()),
        ("ext", "⚡ 扩展层", KcpMask::variants_by_category("ext").len()),
    ];

    for (code, name, total) in &cat_counts {
        let added_count = existing_codes.iter().filter(|ec| {
            KcpMask::from_code(ec).map(|m| m.category_code() == *code).unwrap_or(false)
        }).count();
        let remaining = total - added_count;
        if remaining > 0 {
            if buttons.len() == 0 || buttons.last().unwrap().len() >= 2 {
                buttons.push(Vec::new());
            }
            buttons.last_mut().unwrap().push(
                InlineKeyboardButton::callback(
                    format!("{} ({})", name, remaining),
                    format!("u_kcp_mcat:{},{}", existing, code),
                )
            );
        }
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "✅ 完成配置",
        format!("u_kcp_done:{}", existing),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "🗑️ 清空重选",
        "u_kcp_init",
    )]);

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "📋 <b>当前遮罩栈:</b>\n{}\n\n\
             ➕ <b>选择要添加的遮罩类别</b> (已达{}层，最多5层)",
            stack_display.join("\n"),
            existing_codes.len()
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Build to check syntax**

Run: `cargo build --package tgbot 2>&1 | grep -E "^error"`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "refactor: replace u_kcp_more flat list with 4 category buttons"
```

---

### Task 7: Add `u_kcp_mcat` handler for multi-select category sub-menu

**Files:**
- Modify: `rust/tgbot/src/main.rs` — add handler

- [ ] **Step 1: Write the `u_kcp_mcat` handler**

Insert after the new `u_kcp_more` handler.

```rust
d if d.starts_with("u_kcp_mcat:") => {
    let data = d.strip_prefix("u_kcp_mcat:").unwrap_or("");
    let parts: Vec<&str> = data.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Ok(());
    }
    let existing = parts[0];
    let cat_code = parts[1];
    let existing_codes: Vec<&str> = existing.split(',').collect();
    let cat_name = KcpMask::category_from_code(cat_code).unwrap_or("未知");

    let variants = KcpMask::variants_by_category(cat_code);

    let stack_display: Vec<String> = existing_codes.iter().enumerate().map(|(i, c)| {
        let m = KcpMask::from_code(c);
        format!("{}️⃣ {}", i + 1, m.map(|m| m.display_name()).unwrap_or("???"))
    }).collect();

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for mask in &variants {
        let code = mask.code();
        if existing_codes.contains(&code) {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("☑️ {}", mask.display_name()),
                "noop",
            )]);
        } else {
            buttons.push(vec![InlineKeyboardButton::callback(
                format!("✅ {}", mask.display_name()),
                format!("u_kcp_push:{}:{}", existing, code),
            )]);
        }
    }

    buttons.push(vec![
        InlineKeyboardButton::callback(
            "⬅️ 返回分类",
            format!("u_kcp_more:{}", existing),
        ),
    ]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "✅ 完成配置",
        format!("u_kcp_done:{}", existing),
    )]);
    buttons.push(vec![InlineKeyboardButton::callback(
        "🗑️ 清空重选",
        "u_kcp_init",
    )]);

    let mask_list: String = variants
        .iter()
        .map(|m| format!("{}\n{}", m.display_name(), m.brief()))
        .collect::<Vec<_>>()
        .join("\n\n");

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "📋 <b>当前遮罩栈:</b>\n{}\n\n\
             <b>{}</b> — 选择要添加的遮罩\n\n{}",
            stack_display.join("\n"),
            cat_name,
            mask_list
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Build to check syntax**

Run: `cargo build --package tgbot 2>&1 | grep -E "^error"`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add rust/tgbot/src/main.rs
git commit -m "feat: add u_kcp_mcat handler for multi-select category sub-menu"
```

---

### Task 8: Verify all tests pass and add smoke test

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs`

- [ ] **Step 1: Run full test suite**

Run: `cargo test --package tgbot 2>&1 | tail -30`
Expected: All tests pass

- [ ] **Step 2: Add a smoke test for category navigation**

Add to `kcp_mask_navigation_tests`:

```rust
#[test]
fn test_category_buttons_count_matches_variant_counts() {
    let enc = KcpMask::variants_by_category("enc").len();
    let obf = KcpMask::variants_by_category("obf").len();
    let dis = KcpMask::variants_by_category("dis").len();
    let ext = KcpMask::variants_by_category("ext").len();
    assert_eq!(enc + obf + dis + ext, 14, "total variants should be 14");
}
```

- [ ] **Step 3: Run tests again**

Run: `cargo test --package tgbot 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add rust/tgbot/src/logic/config.rs
git commit -m "test: add smoke test for category navigation completeness"
```

---

### Task 9: Final verification

**Files:**
- None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test --package tgbot 2>&1 | grep -E "test result|error|FAILED"`
Expected: All tests pass, no errors

- [ ] **Step 2: Build release binary**

Run: `cargo build --release --package tgbot 2>&1 | tail -5`
Expected: Successful build

- [ ] **Step 3: Final summary**

Check git log: `git log --oneline -10`
Should show 7 commits for this feature

---

**Plan complete and saved to `docs/superpowers/plans/2026-04-29-kcp-category-nav.md`.**

Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?