# KCP FinalMask Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove XDNS mKCP+Reality feature entirely, add official Xray-core KCP transport with FinalMask support (security=none only).

**Architecture:** Rename `RealityProto` → `Proto`, remove `XdnsMkcp` variant, add `Kcp` variant. Add `KcpFinalMask` enum with all official types. New KCP functions generate configs with official kcpSettings format and finalmask UDP array. KCP UI flow uses multi-step callback: finalmask group → finalmask type → IP version → count → execute.

**Tech Stack:** Rust, teloxide (Telegram bot), serde_json for JSON generation, percent_encoding for URL encoding.

---

## Task 1: Remove XdnsMkcp from Proto enum and update all match branches in config.rs

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs:74-79` (RealityProto enum)
- Modify: `rust/tgbot/src/logic/config.rs:244-253` (generate_secure_batch_filename)
- Modify: `rust/tgbot/src/logic/config.rs:740-755` (generate_enhanced_config match)
- Modify: `rust/tgbot/src/logic/config.rs:799-864` (generate_client_link match)

- [ ] **Step 1: Rename RealityProto to Proto and update variants**

Replace at `config.rs:74-79`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RealityProto {
    Vision,
    XHTTP,
    XdnsMkcp,
}
```

With:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Vision,
    XHTTP,
    Kcp,
}
```

- [ ] **Step 2: Update generate_secure_batch_filename match**

At `config.rs:248-250`, replace:

```rust
let prefix = match proto {
    RealityProto::Vision => "batch_reality",
    RealityProto::XHTTP => "batch_xhttp",
    RealityProto::XdnsMkcp => "batch_xdns",
};
```

With:

```rust
let prefix = match proto {
    Proto::Vision => "batch_reality",
    Proto::XHTTP => "batch_xhttp",
    Proto::Kcp => "batch_kcp",
};
```

- [ ] **Step 3: Update generate_enhanced_config match**

At `config.rs:740-752`, replace:

```rust
let suffix = match proto {
    RealityProto::Vision => "vless_reality_vision",
    RealityProto::XHTTP => "vless_xhttp_reality",
    RealityProto::XdnsMkcp => "vless_xdns_mkcp",
};
let email = format!("{}-{}", uuid_short, suffix);
let tag = format!(
    "{}-{}-{}",
    match proto {
        RealityProto::Vision => "VLESS",
        RealityProto::XHTTP => "XHTTP",
        RealityProto::XdnsMkcp => "XDNS",
    },
    uuid_short,
    index
);
```

With:

```rust
let suffix = match proto {
    Proto::Vision => "vless_reality_vision",
    Proto::XHTTP => "vless_xhttp_reality",
    Proto::Kcp => "vless_kcp",
};
let email = format!("{}-{}", uuid_short, suffix);
let tag = format!(
    "{}-{}-{}",
    match proto {
        Proto::Vision => "VLESS",
        Proto::XHTTP => "XHTTP",
        Proto::Kcp => "KCP",
    },
    uuid_short,
    index
);
```

Also at `config.rs:757`, the `path` condition — XdnsMkcp didn't have a path either, so Kcp also won't:

```rust
let path = if proto == Proto::XHTTP {
    Some(Self::generate_random_path())
} else {
    None
};
```

- [ ] **Step 4: Update generate_client_link signature and match**

At `config.rs:776-786`, update the function signature from `RealityProto` to `Proto`:

```rust
fn generate_client_link(
    uuid: &str,
    host: &str,
    port: i32,
    sni: &str,
    pub_key: &str,
    short_id: &str,
    email: &str,
    ip_version: IpVersion,
    proto: Proto,
    path: Option<&str>,
    host_secondary: Option<&str>,
    enable_pq: bool,
) -> String {
```

At `config.rs:799`, update the match:

```rust
match proto {
    Proto::Vision => {
        // ... existing Vision code unchanged ...
    }
    Proto::XHTTP => {
        // ... existing XHTTP code unchanged ...
    }
    Proto::Kcp => {
        unreachable!("Kcp should use generate_kcp_client_link instead")
    }
}
```

- [ ] **Step 5: Update create_standalone_config signature**

At `config.rs:870`, change `RealityProto` to `Proto`:

```rust
async fn create_standalone_config(
    configs: Vec<Value>,
    links: Vec<String>,
    proto: Proto,
) -> Result<BatchCreationResult> {
```

- [ ] **Step 6: Run cargo check to verify title renames compile**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -50`
Expected: Compilation errors about `RealityProto`/`XdnsMkcp` references in main.rs (will fix in Task 3)

---

## Task 2: Remove XDNS functions and tests from config.rs

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs` (delete functions and tests)

- [ ] **Step 1: Delete batch_create_xdns_mkcp function**

Remove lines 512-572 (`pub async fn batch_create_xdns_mkcp` entire function body).

- [ ] **Step 2: Delete build_xdns_mkcp_inbound function**

Remove lines 574-639 (`pub(crate) fn build_xdns_mkcp_inbound` entire function body).

- [ ] **Step 3: Delete generate_xdns_client_link function**

Remove lines 641-676 (`pub(crate) fn generate_xdns_client_link` entire function body).

- [ ] **Step 4: Delete XDNS-related tests**

Remove tests at lines 1482-1615:
- `test_build_xdns_mkcp_inbound_structure`
- `test_build_xdns_mkcp_ipv6_listen`
- `test_generate_xdns_client_link_format`
- `test_generate_xdns_client_link_ipv6`
- `test_xdns_mkcp_config_mtu_130`
- `test_xdns_finalmask_domain`

- [ ] **Step 5: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -50`
Expected: Still compilation errors in main.rs (will fix next task)

---

## Task 3: Update main.rs — remove XDNS UI, remove XdnsMkcp references, add Kcp

**Files:**
- Modify: `rust/tgbot/src/main.rs`

- [ ] **Step 1: Update import line**

At line 33, change:

```rust
use tgbot::logic::config::{ConfigManager, RealityProto, WarpMode};
```

To:

```rust
use tgbot::logic::config::{ConfigManager, KcpFinalMask, Proto, WarpMode};
```

- [ ] **Step 2: Update show_reality_batch_prompt**

At line 124, change `proto: RealityProto` to `proto: Proto`.

At lines 126-129, change:

```rust
let (ip_prefix, title) = match proto {
    RealityProto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
    RealityProto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
    RealityProto::XdnsMkcp => ("u_xdns_ip:", "XDNS Finalmask (mKCP+DNS)"),
};
```

To:

```rust
let (ip_prefix, title) = match proto {
    Proto::Vision => ("u_batch_ip_init:", "Reality (Vision)"),
    Proto::XHTTP => ("u_xhttp_batch_ip_init:", "Reality (XHTTP)"),
    Proto::Kcp => unreachable!("KCP uses separate UI flow"),
};
```

At line 148, change `RealityProto::XHTTP` to `Proto::XHTTP`.

- [ ] **Step 3: Update show_reality_qty_prompt**

At line 183, change `proto: RealityProto` to `proto: Proto`.

At lines 198-201, change:

```rust
let (exec_prefix, title) = match proto {
    RealityProto::Vision => ("u_batch_exec:", "Reality"),
    RealityProto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
    RealityProto::XdnsMkcp => ("u_xdns_exec:", "XDNS"),
};
```

To:

```rust
let (exec_prefix, title) = match proto {
    Proto::Vision => ("u_batch_exec:", "Reality"),
    Proto::XHTTP => ("u_xhttp_batch_exec:", "XHTTP"),
    Proto::Kcp => unreachable!("KCP uses separate UI flow"),
};
```

- [ ] **Step 4: Remove XDNS menu button**

At line 1062, replace:

```rust
InlineKeyboardButton::callback("🚀 XDNS (mKCP+DNS)", "u_xdns_init"),
```

With:

```rust
InlineKeyboardButton::callback("🚀 KCP (mKCP+FinalMask)", "u_kcp_init"),
```

- [ ] **Step 5: Delete entire XDNS callback handlers**

Remove lines 2427-2629 (the entire `"u_xdns_init"` match arm and all `"u_xdns_ip:"` and `"u_xdns_exec:"` handler blocks).

- [ ] **Step 6: Update batch execution handler Proto references**

At lines 2685-2718, update the `proto_str` match and `res` match:

```rust
let proto_str = match proto {
    Proto::Vision => "Reality",
    Proto::XHTTP => "XHTTP",
    Proto::Kcp => "KCP",
};

let res = match proto {
    Proto::Vision => {
        ConfigManager::batch_create_reality_vision_enhanced(
            n, standalone_mode, ip_version,
        )
        .await
    }
    Proto::XHTTP => {
        ConfigManager::batch_create_xhttp_reality_enhanced(
            n, standalone_mode, ip_version,
        )
        .await
    }
    Proto::Kcp => {
        unreachable!("KCP uses separate batch handler")
    }
};
```

- [ ] **Step 7: Update RealityProto→Proto in trigger_reality_auto_init**

At lines 243, 247, 2393, 2411, and any other `RealityProto::Vision` or `RealityProto::XHTTP` references, change to `Proto::Vision` and `Proto::XHTTP`.

- [ ] **Step 8: Update line 2635 and 2637**

At `main.rs:2635` and `main.rs:2637`:

```rust
("u_batch_ip_init:", RealityProto::Vision)
...
("u_xhttp_batch_ip_init:", RealityProto::XHTTP)
```

Change to:

```rust
("u_batch_ip_init:", Proto::Vision)
...
("u_xhttp_batch_ip_init:", Proto::XHTTP)
```

- [ ] **Step 9: Update line 2651 and 2653**

```rust
("u_batch_exec:", RealityProto::Vision)
...
("u_xhttp_batch_exec:", RealityProto::XHTTP)
```

Change to:

```rust
("u_batch_exec:", Proto::Vision)
...
("u_xhttp_batch_exec:", Proto::XHTTP)
```

- [ ] **Step 10: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -50`
Expected: Now only errors about missing `KcpFinalMask` import and KCP functions. XDNS removal should be clean.

---

## Task 4: Add KcpFinalMask enum to config.rs

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs` (add after Proto enum, before ConfigManager)

- [ ] **Step 1: Add KcpFinalMask enum after Proto definition**

Insert after the `Proto` enum (around line 82), before `pub struct ConfigManager;`:

```rust
#[derive(Debug, Clone)]
pub enum KcpFinalMask {
    MkcpOriginal,
    MkcpAes128Gcm { password: String },
    HeaderDns { domain: String },
    HeaderWechat,
    HeaderSrtp,
    HeaderUtp,
    HeaderDtls,
    HeaderWireguard,
}

impl KcpFinalMask {
    pub fn type_str(&self) -> &'static str {
        match self {
            KcpFinalMask::MkcpOriginal => "mkcp-original",
            KcpFinalMask::MkcpAes128Gcm { .. } => "mkcp-aes128gcm",
            KcpFinalMask::HeaderDns { .. } => "header-dns",
            KcpFinalMask::HeaderWechat => "header-wechat",
            KcpFinalMask::HeaderSrtp => "header-srtp",
            KcpFinalMask::HeaderUtp => "header-utp",
            KcpFinalMask::HeaderDtls => "header-dtls",
            KcpFinalMask::HeaderWireguard => "header-wireguard",
        }
    }

    pub fn group_name(&self) -> &'static str {
        match self {
            KcpFinalMask::MkcpOriginal | KcpFinalMask::MkcpAes128Gcm { .. } => "enc",
            KcpFinalMask::HeaderDns { .. }
            | KcpFinalMask::HeaderWechat
            | KcpFinalMask::HeaderSrtp
            | KcpFinalMask::HeaderUtp
            | KcpFinalMask::HeaderDtls
            | KcpFinalMask::HeaderWireguard => "dsg",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            KcpFinalMask::MkcpOriginal => "mKCP Original (XOR)",
            KcpFinalMask::MkcpAes128Gcm { .. } => "mKCP AES-128-GCM",
            KcpFinalMask::HeaderDns { .. } => "DNS 查询伪装",
            KcpFinalMask::HeaderWechat => "微信视频通话伪装",
            KcpFinalMask::HeaderSrtp => "SRTP 伪装",
            KcpFinalMask::HeaderUtp => "uTP (BitTorrent) 伪装",
            KcpFinalMask::HeaderDtls => "DTLS 1.2 伪装",
            KcpFinalMask::HeaderWireguard => "WireGuard 伪装",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            KcpFinalMask::MkcpOriginal => "mo",
            KcpFinalMask::MkcpAes128Gcm { .. } => "ma",
            KcpFinalMask::HeaderDns { .. } => "hd",
            KcpFinalMask::HeaderWechat => "hw",
            KcpFinalMask::HeaderSrtp => "hs",
            KcpFinalMask::HeaderUtp => "hu",
            KcpFinalMask::HeaderDtls => "hdt",
            KcpFinalMask::HeaderWireguard => "hwg",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "mo" => Some(KcpFinalMask::MkcpOriginal),
            "ma" => Some(KcpFinalMask::MkcpAes128Gcm {
                password: ConfigManager::generate_aes_password(),
            }),
            "hd" => Some(KcpFinalMask::HeaderDns {
                domain: "www.baidu.com".to_string(),
            }),
            "hw" => Some(KcpFinalMask::HeaderWechat),
            "hs" => Some(KcpFinalMask::HeaderSrtp),
            "hu" => Some(KcpFinalMask::HeaderUtp),
            "hdt" => Some(KcpFinalMask::HeaderDtls),
            "hwg" => Some(KcpFinalMask::HeaderWireguard),
            _ => None,
        }
    }

    pub fn as_json(&self) -> Value {
        match self {
            KcpFinalMask::MkcpOriginal => json!({
                "type": "mkcp-original"
            }),
            KcpFinalMask::MkcpAes128Gcm { password } => json!({
                "type": "mkcp-aes128gcm",
                "settings": { "password": password }
            }),
            KcpFinalMask::HeaderDns { domain } => json!({
                "type": "header-dns",
                "settings": { "domain": domain }
            }),
            KcpFinalMask::HeaderWechat => json!({
                "type": "header-wechat"
            }),
            KcpFinalMask::HeaderSrtp => json!({
                "type": "header-srtp"
            }),
            KcpFinalMask::HeaderUtp => json!({
                "type": "header-utp"
            }),
            KcpFinalMask::HeaderDtls => json!({
                "type": "header-dtls"
            }),
            KcpFinalMask::HeaderWireguard => json!({
                "type": "header-wireguard"
            }),
        }
    }

    pub fn encryption_variants() -> Vec<Self> {
        vec![
            KcpFinalMask::MkcpOriginal,
            KcpFinalMask::MkcpAes128Gcm {
                password: String::new(),
            },
        ]
    }

    pub fn disguise_variants() -> Vec<Self> {
        vec![
            KcpFinalMask::HeaderDns {
                domain: String::new(),
            },
            KcpFinalMask::HeaderWechat,
            KcpFinalMask::HeaderSrtp,
            KcpFinalMask::HeaderUtp,
            KcpFinalMask::HeaderDtls,
            KcpFinalMask::HeaderWireguard,
        ]
    }
}
```

- [ ] **Step 2: Add generate_aes_password helper to ConfigManager**

Add inside `impl ConfigManager`:

```rust
fn generate_aes_password() -> String {
    use rand::Rng;
    let rng_len = rand::thread_rng().gen_range(16..32);
    let password: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(rng_len)
        .map(char::from)
        .collect();
    password
}
```

- [ ] **Step 3: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -30`
Expected: No errors related to KcpFinalMask. Only errors from missing KCP config functions still.

---

## Task 5: Add build_kcp_inbound, generate_kcp_client_link, batch_create_kcp to config.rs

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs`

- [ ] **Step 1: Add build_kcp_inbound function**

Add inside `impl ConfigManager`, after where the old XDNS functions were removed:

```rust
pub(crate) fn build_kcp_inbound(
    tag: &str,
    port: i32,
    uuid: &str,
    email: &str,
    ip_version: IpVersion,
    finalmask: &KcpFinalMask,
) -> Value {
    let listen_ip = match ip_version {
        IpVersion::IPv4 | IpVersion::SplitStackV4Primary => "0.0.0.0",
        IpVersion::IPv6 | IpVersion::SplitStackV6Primary => "::",
    };

    let client = json!({
        "id": uuid,
        "email": email
    });

    let finalmask = finalmask.as_json();
    let udp_array = match finalmask.get("type") {
        Some(_) => json!([finalmask]),
        None => json!([finalmask]),
    };

    json!({
        "listen": listen_ip,
        "port": port,
        "protocol": "vless",
        "tag": tag,
        "settings": {
            "clients": [client],
            "decryption": "none"
        },
        "streamSettings": {
            "network": "kcp",
            "kcpSettings": {
                "mtu": 1350,
                "tti": 50,
                "uplinkCapacity": 5,
                "downlinkCapacity": 20,
                "cwndMultiplier": 1,
                "maxSendingWindow": 2097152
            },
            "security": "none",
            "finalmask": {
                "udp": udp_array
            }
        },
        "sniffing": {
            "enabled": true,
            "destOverride": ["http", "tls", "quic"],
            "metadataOnly": false
        }
    })
}
```

- [ ] **Step 2: Add generate_kcp_client_link function**

```rust
pub(crate) fn generate_kcp_client_link(
    uuid: &str,
    host: &str,
    port: i32,
    email: &str,
    ip_version: IpVersion,
    finalmask: &KcpFinalMask,
) -> String {
    let finalmask_json = json!({
        "udp": [finalmask.as_json()]
    });
    let fm_str = serde_json::to_string(&finalmask_json).unwrap();
    let fm_encoded = utf8_percent_encode(&fm_str, NON_ALPHANUMERIC).to_string();

    let fmt_host = match ip_version {
        IpVersion::IPv6 | IpVersion::SplitStackV6Primary => format!("[{}]", host),
        IpVersion::IPv4 | IpVersion::SplitStackV4Primary => host.to_string(),
    };
    let encoded_email = utf8_percent_encode(email, NON_ALPHANUMERIC).to_string();

    format!(
        "vless://{}@{}:{}?encryption=none&type=kcp&security=none&fm={}#{}",
        uuid, fmt_host, port, fm_encoded, encoded_email
    )
}
```

- [ ] **Step 3: Add batch_create_kcp function**

```rust
pub async fn batch_create_kcp(
    count: usize,
    standalone: bool,
    ip_version: IpVersion,
    finalmask_code: &str,
) -> Result<BatchCreationResult> {
    let finalmask = KcpFinalMask::from_code(finalmask_code)
        .ok_or_else(|| anyhow!("Invalid finalmask code: {}", finalmask_code))?;

    let (host, _) = Self::resolve_public_hosts(
        ip_version,
        crate::logic::system::SystemMonitor::get_public_ip().await,
        crate::logic::system::SystemMonitor::get_public_ipv6().await,
    )?;

    let mut rng = StdRng::from_entropy();

    let mut links = Vec::new();
    let mut batch_configs = Vec::new();

    for i in 0..count {
        let port = loop {
            let p = rng.gen_range(10000..60000);
            if crate::logic::port_allocator::PortAllocator::is_port_in_locked_range(p).await {
                continue;
            }
            if crate::logic::maintenance::MaintenanceManager::is_port_available(p).await {
                break p as i32;
            }
        };

        let uuid = Self::generate_wwps_uuid().await?;
        let uuid_short = Self::uuid_short_prefix(&uuid);

        let email = format!("{}-vless-kcp-{}", uuid_short, finalmask.type_str());
        let tag = format!("KCP-{}-{}", i + 1, uuid_short);

        let config = Self::build_kcp_inbound(
            &tag, port, &uuid, &email, ip_version, &finalmask,
        );
        batch_configs.push(config);

        let link = Self::generate_kcp_client_link(
            &uuid, &host, port, &email, ip_version, &finalmask,
        );
        links.push(link);

        let _ = crate::logic::maintenance::MaintenanceManager::allow_port(port as u16).await;
    }

    if standalone {
        Self::create_standalone_config(batch_configs, links, Proto::Kcp).await
    } else {
        Self::update_existing_config(batch_configs, links).await
    }
}
```

- [ ] **Step 4: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -30`
Expected: Only errors in main.rs about missing KCP UI handlers.

---

## Task 6: Add KCP Telegram bot UI flow in main.rs

**Files:**
- Modify: `rust/tgbot/src/main.rs`

- [ ] **Step 1: Add KCP finalmask group selection callback handler**

After the XDNS handlers have been removed, add the `u_kcp_init` handler:

```rust
"u_kcp_init" => {
    let buttons = vec![
        vec![
            InlineKeyboardButton::callback("🔐 加密", "u_kcp_grp:enc"),
            InlineKeyboardButton::callback("🎭 伪装", "u_kcp_grp:dsg"),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", "m_xray_mgmt")],
    ];

    bot.edit_message_text(
        chat_id,
        msg_id,
        "🚀 <b>KCP (mKCP+FinalMask) 配置</b>\n\n✨ <b>特点:</b>\n• 基于 mKCP 协议的可靠传输\n• FinalMask 伪装/加密支持\n• 官方 Xray-core 标准 kcpSettings\n\n⬇️ <b>请选择 FinalMask 类别:</b>",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 2: Add finalmask type selection handler**

```rust
d if d.starts_with("u_kcp_grp:") => {
    let group = d.strip_prefix("u_kcp_grp:").unwrap_or("enc");
    let finalmasks = if group == "enc" {
        KcpFinalMask::encryption_variants()
    } else {
        KcpFinalMask::disguise_variants()
    };

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for fm in &finalmasks {
        let code = fm.code();
        let name = fm.display_name();
        buttons.push(vec![InlineKeyboardButton::callback(
            name,
            format!("u_kcp_fm:{}", code),
        )]);
    }
    buttons.push(vec![InlineKeyboardButton::callback("⬅️ 返回", "u_kcp_init")]);

    let group_name = if group == "enc" { "加密" } else { "伪装" };

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>KCP FinalMask - {}</b>\n\n⬇️ <b>请选择 FinalMask 类型:</b>",
            group_name
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 3: Add finalmask → IP version selection handler**

```rust
d if d.starts_with("u_kcp_fm:") => {
    let fm_code = d.strip_prefix("u_kcp_fm:").unwrap_or("mo");

    let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        "🌐 IPv4 (0.0.0.0)",
        format!("u_kcp_ip:{}:4", fm_code),
    )]];

    if has_ipv6 {
        buttons[0].push(InlineKeyboardButton::callback(
            "🌐 IPv6 (::)",
            format!("u_kcp_ip:{}:6", fm_code),
        ));
    }

    buttons.push(vec![InlineKeyboardButton::callback(
        "⬅️ 返回",
        "u_kcp_init",
    )]);

    let fm = KcpFinalMask::from_code(fm_code);
    let fm_name = fm.as_ref().map(|f| f.display_name()).unwrap_or("Unknown");

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>KCP FinalMask 配置</b>\n\n🎭 FinalMask: <b>{}</b>\n\n⬇️ <b>请选择网络协议版本:</b>",
            fm_name
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 4: Add IP version → count selection handler**

```rust
d if d.starts_with("u_kcp_ip:") => {
    let parts: Vec<&str> = d.strip_prefix("u_kcp_ip:").unwrap_or("").split(':').collect();
    if parts.len() != 2 {
        return Ok(());
    }
    let fm_code = parts[0];
    let ip_ver_code = parts[1];
    let ip_version: IpVersion = match ip_ver_code {
        "6" => IpVersion::IPv6,
        _ => IpVersion::IPv4,
    };
    let ip_display = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        _ => "IPv4",
    };

    let fm = KcpFinalMask::from_code(fm_code);
    let fm_name = fm.as_ref().map(|f| f.display_name()).unwrap_or("Unknown");

    let buttons = vec![
        vec![
            InlineKeyboardButton::callback("1", format!("u_kcp_ex:{}:{}:1", fm_code, ip_ver_code)),
            InlineKeyboardButton::callback("3", format!("u_kcp_ex:{}:{}:3", fm_code, ip_ver_code)),
            InlineKeyboardButton::callback("5", format!("u_kcp_ex:{}:{}:5", fm_code, ip_ver_code)),
        ],
        vec![
            InlineKeyboardButton::callback("10", format!("u_kcp_ex:{}:{}:10", fm_code, ip_ver_code)),
            InlineKeyboardButton::callback("20", format!("u_kcp_ex:{}:{}:20", fm_code, ip_ver_code)),
            InlineKeyboardButton::callback("50", format!("u_kcp_ex:{}:{}:50", fm_code, ip_ver_code)),
        ],
        vec![InlineKeyboardButton::callback("⬅️ 返回", format!("u_kcp_fm:{}", fm_code))],
    ];

    bot.edit_message_text(
        chat_id,
        msg_id,
        format!(
            "🚀 <b>KCP Finalmask 批量配置</b>\n\n🎭 FinalMask: <b>{}</b>\n🌐 网络协议: <b>{}</b>\n\n⬇️ <b>请选择生成数量:</b>",
            fm_name, ip_display
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(InlineKeyboardMarkup::new(buttons))
    .await?;
}
```

- [ ] **Step 5: Add batch execution handler**

```rust
d if d.starts_with("u_kcp_ex:") => {
    let parts: Vec<&str> = d.strip_prefix("u_kcp_ex:").unwrap_or("").split(':').collect();
    if parts.len() != 3 {
        return Ok(());
    }
    let fm_code = parts[0];
    let ip_ver_code = parts[1];
    let n: usize = parts[2].parse().unwrap_or(0);

    let ip_version = match ip_ver_code {
        "6" => IpVersion::IPv6,
        _ => IpVersion::IPv4,
    };
    let ip_str = match ip_version {
        IpVersion::IPv4 => "IPv4",
        IpVersion::IPv6 => "IPv6",
        _ => "IPv4",
    };

    let fm = KcpFinalMask::from_code(fm_code);
    let fm_name = fm.as_ref().map(|f| f.display_name()).unwrap_or("Unknown");

    bot.answer_callback_query(q.id.clone())
        .text(format!("⏳ 正在生成 {} 个 KCP {} 配置...", n, fm_name))
        .await?;

    let res = ConfigManager::batch_create_kcp(n, true, ip_version, fm_code).await;

    match res {
        Ok(result) => {
            let mut message_ids: Vec<MessageId> = Vec::new();

            let mut combined_links = String::new();
            for (i, link) in result.links.iter().enumerate() {
                combined_links.push_str(&format!("<code>{}</code>\n\n", link));
                if (i + 1) % 2 == 0 {
                    if let Ok(msg) = bot
                        .send_message(chat_id, combined_links.clone())
                        .parse_mode(ParseMode::Html)
                        .await
                    {
                        message_ids.push(msg.id);
                    }
                    combined_links.clear();
                }
            }
            if !combined_links.is_empty() {
                if let Ok(msg) = bot
                    .send_message(chat_id, combined_links)
                    .parse_mode(ParseMode::Html)
                    .await
                {
                    message_ids.push(msg.id);
                }
            }

            let links_text = result.links.join("\n");
            let timestamp = chrono::Utc::now().timestamp();
            let temp_file_path = format!("/tmp/wwps_kcp_links_{}.txt", timestamp);

            if let Err(e) = tokio::fs::write(&temp_file_path, &links_text).await {
                log::warn!("写入临时文件失败: {}", e);
            } else {
                let document_sent = bot
                    .send_document(chat_id, InputFile::file(&temp_file_path))
                    .caption(format!("KCP {} 完整链接列表", fm_name))
                    .await;

                if let Err(e) = tokio::fs::remove_file(&temp_file_path).await {
                    log::warn!("删除临时文件失败: {}", e);
                }

                if let Ok(msg) = document_sent {
                    message_ids.push(msg.id);
                }
            }

            let mut result_msg = format!(
                "✅ KCP {} 批量生成完成！\n\n📊 生成数量: {}\n🌐 网络协议: {}\n⚡ 特点: {} Finalmask + mKCP传输",
                fm_name, result.created_count, ip_str, fm_name
            );

            if let Some(filename) = result.config_file {
                result_msg.push_str(&format!("\n\n📁 配置文件: {}", filename));
            }

            let summary_msg = bot.send_message(chat_id, result_msg).await?;
            message_ids.push(summary_msg.id);

            let bot_clone = bot.clone();
            let chat_id_clone = chat_id;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                for msg_id in message_ids {
                    let _ = bot_clone.delete_message(chat_id_clone, msg_id).await;
                }
            });
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ 生成失败: {}", e))
                .parse_mode(ParseMode::Html)
                .await?;
        }
    }
}
```

- [ ] **Step 6: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1 | head -30`
Expected: Clean compilation or minor fixable errors only.

---

## Task 7: Add KCP unit tests to config.rs

**Files:**
- Modify: `rust/tgbot/src/logic/config.rs` (append to tests module)

- [ ] **Step 1: Add tests for build_kcp_inbound**

Add in the `#[cfg(test)] mod tests` section:

```rust
#[test]
fn test_build_kcp_inbound_mkcp_original() {
    let config = ConfigManager::build_kcp_inbound(
        "KCP-TEST",
        34456,
        "test-uuid",
        "test-email",
        IpVersion::IPv4,
        &KcpFinalMask::MkcpOriginal,
    );

    assert_eq!(config["listen"], "0.0.0.0");
    assert_eq!(config["port"], 34456);
    assert_eq!(config["protocol"], "vless");
    assert_eq!(config["tag"], "KCP-TEST");

    let ss = &config["streamSettings"];
    assert_eq!(ss["network"], "kcp");
    assert_eq!(ss["kcpSettings"]["mtu"], 1350);
    assert_eq!(ss["kcpSettings"]["tti"], 50);
    assert_eq!(ss["kcpSettings"]["uplinkCapacity"], 5);
    assert_eq!(ss["kcpSettings"]["downlinkCapacity"], 20);
    assert_eq!(ss["kcpSettings"]["cwndMultiplier"], 1);
    assert_eq!(ss["kcpSettings"]["maxSendingWindow"], 2097152);
    assert_eq!(ss["security"], "none");

    let fm = &ss["finalmask"]["udp"][0];
    assert_eq!(fm["type"], "mkcp-original");
    assert!(fm.get("settings").is_none());
}

#[test]
fn test_build_kcp_inbound_header_dns() {
    let config = ConfigManager::build_kcp_inbound(
        "KCP-TEST",
        34456,
        "test-uuid",
        "test-email",
        IpVersion::IPv4,
        &KcpFinalMask::HeaderDns {
            domain: "www.google.com".to_string(),
        },
    );

    let fm = &config["streamSettings"]["finalmask"]["udp"][0];
    assert_eq!(fm["type"], "header-dns");
    assert_eq!(fm["settings"]["domain"], "www.google.com");
}

#[test]
fn test_build_kcp_inbound_mkcp_aes128gcm() {
    let config = ConfigManager::build_kcp_inbound(
        "KCP-TEST",
        34456,
        "test-uuid",
        "test-email",
        IpVersion::IPv6,
        &KcpFinalMask::MkcpAes128Gcm {
            password: "testpass".to_string(),
        },
    );

    assert_eq!(config["listen"], "::");
    let fm = &config["streamSettings"]["finalmask"]["udp"][0];
    assert_eq!(fm["type"], "mkcp-aes128gcm");
    assert_eq!(fm["settings"]["password"], "testpass");
}

#[test]
fn test_kcp_no_reality_settings() {
    let config = ConfigManager::build_kcp_inbound(
        "KCP-TEST",
        34456,
        "test-uuid",
        "test-email",
        IpVersion::IPv4,
        &KcpFinalMask::MkcpOriginal,
    );

    assert!(config["streamSettings"].get("realitySettings").is_none());
    assert!(config["streamSettings"].get("tlsSettings").is_none());
}

#[test]
fn test_kcp_no_old_fields() {
    let config = ConfigManager::build_kcp_inbound(
        "KCP-TEST",
        34456,
        "test-uuid",
        "test-email",
        IpVersion::IPv4,
        &KcpFinalMask::MkcpOriginal,
    );

    let kcp = &config["streamSettings"]["kcpSettings"];
    assert!(kcp.get("congestion").is_none(), "congestion should be removed");
    assert!(kcp.get("readBufferSize").is_none(), "readBufferSize should be removed");
    assert!(kcp.get("writeBufferSize").is_none(), "writeBufferSize should be removed");
}
```

- [ ] **Step 2: Add tests for KcpFinalMask enum methods**

```rust
#[test]
fn test_kcp_finalmask_type_str() {
    assert_eq!(KcpFinalMask::MkcpOriginal.type_str(), "mkcp-original");
    assert_eq!(KcpFinalMask::MkcpAes128Gcm { password: "x".into() }.type_str(), "mkcp-aes128gcm");
    assert_eq!(KcpFinalMask::HeaderDns { domain: "x".into() }.type_str(), "header-dns");
    assert_eq!(KcpFinalMask::HeaderWechat.type_str(), "header-wechat");
    assert_eq!(KcpFinalMask::HeaderSrtp.type_str(), "header-srtp");
    assert_eq!(KcpFinalMask::HeaderUtp.type_str(), "header-utp");
    assert_eq!(KcpFinalMask::HeaderDtls.type_str(), "header-dtls");
    assert_eq!(KcpFinalMask::HeaderWireguard.type_str(), "header-wireguard");
}

#[test]
fn test_kcp_finalmask_group_name() {
    assert_eq!(KcpFinalMask::MkcpOriginal.group_name(), "enc");
    assert_eq!(KcpFinalMask::MkcpAes128Gcm { password: "x".into() }.group_name(), "enc");
    assert_eq!(KcpFinalMask::HeaderDns { domain: "x".into() }.group_name(), "dsg");
    assert_eq!(KcpFinalMask::HeaderWireguard.group_name(), "dsg");
}

#[test]
fn test_kcp_finalmask_code_roundtrip() {
    let codes = ["mo", "ma", "hd", "hw", "hs", "hu", "hdt", "hwg"];
    for code in codes {
        let fm = KcpFinalMask::from_code(code);
        assert!(fm.is_some(), "Failed to parse code: {}", code);
        assert_eq!(fm.unwrap().code(), code);
    }
}

#[test]
fn test_kcp_finalmask_from_code_invalid() {
    assert!(KcpFinalMask::from_code("invalid").is_none());
    assert!(KcpFinalMask::from_code("").is_none());
}

#[test]
fn test_generate_kcp_client_link_mkcp_original() {
    let link = ConfigManager::generate_kcp_client_link(
        "test-uuid",
        "192.168.1.1",
        34456,
        "test-user",
        IpVersion::IPv4,
        &KcpFinalMask::MkcpOriginal,
    );

    assert!(link.starts_with("vless://test-uuid@192.168.1.1:34456"));
    assert!(link.contains("type=kcp"));
    assert!(link.contains("security=none"));
    assert!(link.contains("fm="));
    assert!(link.contains("#test-user"));
    assert!(!link.contains("sni="));
    assert!(!link.contains("pbk="));
}
```

- [ ] **Step 3: Run all tests**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo test 2>&1 | tail -30`
Expected: All new tests pass, no old tests broken.

---

## Task 8: Final verification — compile check and full test run

**Files:**
- All modified files

- [ ] **Step 1: Run cargo check**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo check 2>&1`
Expected: Clean compilation, zero errors.

- [ ] **Step 2: Run full test suite**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 3: Run cargo clippy**

Run: `cd /home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot && cargo clippy 2>&1 | head -40`
Expected: No new warnings.

- [ ] **Step 4: Commit all changes**

```bash
git add rust/tgbot/src/logic/config.rs rust/tgbot/src/main.rs
git commit -m "feat: remove XDNS mKCP+Reality, add KCP+FinalMask support

- Remove RealityProto::XdnsMkcp and all XDNS-related functions
- Remove XDNS callback handlers from Telegram bot UI
- Rename RealityProto to Proto, add Kcp variant
- Add KcpFinalMask enum with all official Xray-core types
- Add build_kcp_inbound() with official kcpSettings format
- Add generate_kcp_client_link() for VLESS link generation
- Add batch_create_kcp() for batch creation
- Add KCP Telegram bot UI flow (group → type → IP → count → execute)
- Follow official Xray-core: removed congestion/readBufferSize/writeBufferSize
- KCP security=none only (no TLS, no Reality)
- Add comprehensive unit tests for KCP functionality"
```