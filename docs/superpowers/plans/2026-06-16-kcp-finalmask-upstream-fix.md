# KCP FinalMask 上游对齐实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) for tracking.

**Goal:** 修正 KCP FinalMask JSON 输出以匹配上游 XTLS/Xray-core — 将 `finalmask.udp` 改为 `udpmasks`，扁平化 settings，使用完整 protobuf 类型路径。

**Architecture:** 修改 `kcp_mask.rs` 的 `as_json()` 以移除 `settings` 包装器并改用完整类型字符串；修改 `kcp.rs` 的 `build_kcp_inbound()` 和 `generate_kcp_client_link()` 以使用更正后的 JSON 路径。5 个测试需要更新预期值。

**Tech Stack:** Rust, serde_json, Xray-core proto

---

### Task 1: 扁平化 as_json() 并更新类型路径

**Files:**
- Modify: `rust/aegis/src/core/xray/kcp_mask.rs:249-317`
- Test: `rust/aegis/src/core/xray/kcp_mask.rs:589-693`

- [ ] **Step 1: 运行现有测试确认基线**

Run: `cargo test test_mkcp_legacy_as_json test_salamander_with_packet_size test_xdns_new_format test_xicmp_new_format test_realm_as_json -- --nocapture 2>&1 | tail -15`
Expected: 5 test cases, all PASS

- [ ] **Step 2: 修改 as_json() — 扁平化 MkcpLegacy 变体**

将以下代码（行 251-266）：
```rust
KcpMask::MkcpLegacy { header, value } => {
    if header.is_none() && value.is_none() {
        json!({"type": "mkcp-legacy"})
    } else {
        let mut settings = serde_json::Map::new();
        if let Some(h) = header {
            settings.insert("header".to_string(), serde_json::Value::String(h.clone()));
        }
        if let Some(v) = value {
            settings.insert("value".to_string(), serde_json::Value::String(v.clone()));
        }
        json!({
            "type": "mkcp-legacy",
            "settings": serde_json::Value::Object(settings)
        })
    }
}
```
替换为：
```rust
KcpMask::MkcpLegacy { header, value } => {
    if header.is_none() && value.is_none() {
        json!({"type": "xray.transport.internet.finalmask.mkcp.Header"})
    } else {
        let mut map = serde_json::Map::new();
        map.insert(
            "type".to_string(),
            serde_json::Value::String(
                "xray.transport.internet.finalmask.mkcp.Header".to_string(),
            ),
        );
        if let Some(h) = header {
            map.insert("header".to_string(), serde_json::Value::String(h.clone()));
        }
        if let Some(v) = value {
            map.insert("value".to_string(), serde_json::Value::String(v.clone()));
        }
        serde_json::Value::Object(map)
    }
}
```

- [ ] **Step 3: 修改 as_json() — 扁平化 Noise 变体**

将第 268 行：
```rust
KcpMask::Noise => json!({"type": "noise"}),
```
替换为：
```rust
KcpMask::Noise => json!({"type": "xray.transport.internet.finalmask.noise.Config"}),
```

- [ ] **Step 4: 修改 as_json() — 扁平化 Salamander 变体**

将第 269-287 行：
```rust
KcpMask::Salamander {
    password,
    packet_size,
} => {
    let mut settings = serde_json::Map::new();
    settings.insert(
        "password".to_string(),
        serde_json::Value::String(password.clone()),
    );
    if let Some(ps) = packet_size {
        settings.insert(
            "packetSize".to_string(),
            serde_json::Value::String(ps.clone()),
        );
    }
    json!({
        "type": "salamander",
        "settings": serde_json::Value::Object(settings)
    })
}
```
替换为：
```rust
KcpMask::Salamander {
    password,
    packet_size,
} => {
    let mut map = serde_json::Map::new();
    map.insert(
        "type".to_string(),
        serde_json::Value::String(
            "xray.transport.internet.finalmask.salamander.Config".to_string(),
        ),
    );
    map.insert(
        "password".to_string(),
        serde_json::Value::String(password.clone()),
    );
    if let Some(ps) = packet_size {
        map.insert(
            "packetSize".to_string(),
            serde_json::Value::String(ps.clone()),
        );
    }
    serde_json::Value::Object(map)
}
```

- [ ] **Step 5: 修改 as_json() — 扁平化 Sudoku 变体**

将第 289-292 行：
```rust
KcpMask::Sudoku { password } => json!({
    "type": "sudoku",
    "settings": { "password": password }
}),
```
替换为：
```rust
KcpMask::Sudoku { password } => json!({
    "type": "xray.transport.internet.finalmask.sudoku.Config",
    "password": password
}),
```

- [ ] **Step 6: 修改 as_json() — 扁平化 Xdns 变体**

将第 293-296 行：
```rust
KcpMask::Xdns { domains, resolvers } => json!({
    "type": "xdns",
    "settings": { "domains": domains, "resolvers": resolvers }
}),
```
替换为：
```rust
KcpMask::Xdns { domains, resolvers } => json!({
    "type": "xray.transport.internet.finalmask.xdns.Config",
    "domains": domains,
    "resolvers": resolvers
}),
```

- [ ] **Step 7: 修改 as_json() — 扁平化 Xicmp 变体**

将第 297-308 行：
```rust
KcpMask::Xicmp { dgram, ips } => {
    if *dgram || !ips.is_empty() {
        json!({
            "type": "xicmp",
            "settings": {
                "dgram": dgram,
                "ips": ips
            }
        })
    } else {
        json!({"type": "xicmp"})
    }
}
```
替换为：
```rust
KcpMask::Xicmp { dgram, ips } => {
    let mut map = serde_json::Map::new();
    map.insert(
        "type".to_string(),
        serde_json::Value::String(
            "xray.transport.internet.finalmask.xicmp.Config".to_string(),
        ),
    );
    if *dgram || !ips.is_empty() {
        map.insert(
            "dgram".to_string(),
            serde_json::Value::Bool(*dgram),
        );
        map.insert(
            "ips".to_string(),
            serde_json::Value::Array(ips.iter().map(|s| serde_json::Value::String(s.clone())).collect()),
        );
    }
    serde_json::Value::Object(map)
}
```

- [ ] **Step 8: 修改 as_json() — 扁平化 Realm 变体**

将第 310-316 行：
```rust
KcpMask::Realm { url, stun_servers } => json!({
    "type": "realm",
    "settings": {
        "url": url,
        "stunServers": stun_servers
    }
}),
```
替换为：
```rust
KcpMask::Realm { url, stun_servers } => json!({
    "type": "xray.transport.internet.finalmask.realm.Config",
    "url": url,
    "stunServers": stun_servers
}),
```

- [ ] **Step 9: 更新测试 `test_mkcp_legacy_as_json`**

将第 589-623 行替换为：
```rust
#[test]
fn test_mkcp_legacy_as_json() {
    let json = KcpMask::MkcpLegacy {
        header: None,
        value: None,
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.mkcp.Header");
    assert!(json.get("header").is_none());
    assert!(json.get("value").is_none());

    let json = KcpMask::MkcpLegacy {
        header: None,
        value: Some("pwd".into()),
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.mkcp.Header");
    assert!(json.get("header").is_none());
    assert_eq!(json["value"], "pwd");

    let json = KcpMask::MkcpLegacy {
        header: Some("dns".into()),
        value: Some("example.com".into()),
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.mkcp.Header");
    assert_eq!(json["header"], "dns");
    assert_eq!(json["value"], "example.com");

    let json = KcpMask::MkcpLegacy {
        header: Some("wechat".into()),
        value: None,
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.mkcp.Header");
    assert_eq!(json["header"], "wechat");
    assert!(json.get("value").is_none());
}
```

- [ ] **Step 10: 更新测试 `test_salamander_with_packet_size`**

将第 626-642 行替换为：
```rust
#[test]
fn test_salamander_with_packet_size() {
    let json = KcpMask::Salamander {
        password: "obfs".into(),
        packet_size: Some("512-1200".into()),
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.salamander.Config");
    assert_eq!(json["password"], "obfs");
    assert_eq!(json["packetSize"], "512-1200");

    let json_no_ps = KcpMask::Salamander {
        password: "obfs".into(),
        packet_size: None,
    }
    .as_json();
    assert!(json_no_ps.get("packetSize").is_none());
}
```

- [ ] **Step 11: 更新测试 `test_xdns_new_format`**

将第 645-658 行替换为：
```rust
#[test]
fn test_xdns_new_format() {
    let json = KcpMask::Xdns {
        domains: vec!["example.com:aaaa".into()],
        resolvers: vec!["example.com:aaaa+udp://1.1.1.1:53".into()],
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.xdns.Config");
    assert_eq!(json["domains"][0], "example.com:aaaa");
    assert_eq!(
        json["resolvers"][0],
        "example.com:aaaa+udp://1.1.1.1:53"
    );
}
```

- [ ] **Step 12: 更新测试 `test_xicmp_new_format`**

将第 660-678 行替换为：
```rust
#[test]
fn test_xicmp_new_format() {
    let json = KcpMask::Xicmp {
        dgram: false,
        ips: vec![],
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.xicmp.Config");
    assert!(json.get("dgram").is_none());
    assert!(json.get("ips").is_none());

    let json = KcpMask::Xicmp {
        dgram: true,
        ips: vec!["1.2.3.4".into(), "5.6.7.8".into()],
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.xicmp.Config");
    assert_eq!(json["dgram"], true);
    assert_eq!(json["ips"][0], "1.2.3.4");
}
```

- [ ] **Step 13: 更新测试 `test_realm_as_json`**

将第 680-693 行替换为：
```rust
#[test]
fn test_realm_as_json() {
    let json = KcpMask::Realm {
        url: "realm://example.com:1234".into(),
        stun_servers: vec!["stun:stun.l.google.com:19302".into()],
    }
    .as_json();
    assert_eq!(json["type"], "xray.transport.internet.finalmask.realm.Config");
    assert_eq!(json["url"], "realm://example.com:1234");
    assert_eq!(
        json["stunServers"][0],
        "stun:stun.l.google.com:19302"
    );
}
```

- [ ] **Step 14: 运行修改后的测试**

Run: `cargo test test_mkcp_legacy_as_json test_salamander_with_packet_size test_xdns_new_format test_xicmp_new_format test_realm_as_json -- --nocapture 2>&1 | tail -15`
Expected: 5 test cases, all PASS

- [ ] **Step 15: 提交 Task 1**

```bash
git add rust/aegis/src/core/xray/kcp_mask.rs
git commit -m "fix(kcp): flatten as_json() output, use full protobuf type paths"
```

---

### Task 2: 修正 kcp.rs — build_kcp_inbound 和 generate_kcp_client_link

**Files:**
- Modify: `rust/aegis/src/core/xray/kcp.rs:34-63, 74-78`

- [ ] **Step 1: 修改 build_kcp_inbound() — finalmask.udp → udpmasks**

将第 46-48 行：
```rust
"finalmask": {
    "udp": udp_array
},
```
替换为：
```rust
"udpmasks": udp_array,
```

- [ ] **Step 2: 修改 generate_kcp_client_link() — 移除 {"udp": []} 包装**

将第 75-78 行：
```rust
let finalmask_json = json!({
    "udp": udp_array
});
let fm_str = serde_json::to_string(&finalmask_json).unwrap();
```
替换为：
```rust
let fm_str = serde_json::to_string(&udp_array).unwrap();
```

- [ ] **Step 3: 运行完整测试套件**

Run: `cargo test 2>&1 | tail -5`
Expected: 422 tests, all PASS

- [ ] **Step 4: 运行 clippy 和 fmt**

Run: `cargo clippy 2>&1 | tail -3 && cargo fmt --check 2>&1`
Expected: 0 clippy warnings, fmt clean

- [ ] **Step 5: 提交 Task 2**

```bash
git add rust/aegis/src/core/xray/kcp.rs
git commit -m "fix(kcp): use udpmasks directly in streamSettings, remove udp wrapper"
```

---

### Task 3: 最终验证

**Files:** (无代码修改)

- [ ] **Step 1: 完整最终验证**

Run: `cargo test 2>&1 | tail -5 && cargo clippy 2>&1 | tail -3 && cargo fmt --check 2>&1`
Expected: all pass, 0 warnings, clean fmt

- [ ] **Step 2: 提交任何最终调整**

```bash
git add -A
git commit -m "chore: final cleanup after KCP upstream alignment"
```
