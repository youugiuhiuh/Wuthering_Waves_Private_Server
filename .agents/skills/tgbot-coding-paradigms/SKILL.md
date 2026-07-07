# tgbot Coding Paradigms Skill

> 指导 rust/tgbot 项目中三种编程范式的正确使用  
> Guide for correct usage of three programming paradigms in rust/tgbot project

## Overview / 概述

This skill defines when and how to use **Functional**, **Imperative**, and **Declarative** programming paradigms in the rust/tgbot codebase.

本技能定义了 **rust/tgbot** 代码库中 **函数式**、**指令式** 和 **声明式** 三种编程范式的使用场景和最佳实践。

### Three Paradigms / 三种范式

| Paradigm / 范式 | Chinese / 中文 | English / English | Primary Use / 主要用途 |
|-----------------|-----------------|-------------------|----------------------|
| Functional / 函数式 | 数据转换、集合处理 | Data transformation, collection processing | `filter()`, `map()`, `collect()` |
| Imperative / 指令式 | I/O 操作、状态变更 | I/O operations, state mutation | `async/await`, sequential steps |
| Declarative / 声明式 | 配置生成、UI 构建 | Configuration generation, UI construction | `json!` macros, pattern matching |

---

## 1. Functional Paradigm / 函数式范式

### When to Use / 适用场景

| Scenario / 场景 | Example / 示例 | File Location / 文件位置 |
|----------------|----------------|-------------------------|
| Collection processing / 集合处理 | `get_locked_ranges()` | `port_allocator.rs:141` |
| Data mapping / 数据映射 | `wipe_targets()` | `maintenance.rs:419` |
| Filter/Search / 过滤/查找 | Port range search / 端口范围搜索 | `port_allocator.rs:110` |
| Aggregation / 聚合 | Config counting / 配置计数 | `port_allocator.rs:55` |

### When NOT to Use / 避免使用

- I/O operations with side effects / 有副作用的 I/O 操作
- State mutation / 状态变更
- Sequential steps requiring order / 需要顺序的步骤

### Correct Examples / 正确示例

```rust
// ✅ Functional: 数据转换，使用 Iterator 链
pub async fn get_locked_ranges() -> Vec<(u16, u16)> {
    let data = load_port_alloc().await.unwrap_or_default();
    data.locked_ranges
        .iter()
        .map(|r| (r.start, r.end))
        .collect()
}

// ✅ Functional: 端口范围搜索 - 表达"寻找什么"而非"如何寻找"
fn find_consecutive_range(occupied: &HashSet<u16>, size: u16) -> Result<u16> {
    (XRAY_PORT_MIN..=(XRAY_PORT_MAX.saturating_sub(size)))
        .find(|&start| {
            (start..(start + size))
                .all(|port| !occupied.contains(&port))
        })
        .ok_or_else(|| anyhow::anyhow!("在 {} 范围内找不到连续的 {} 个空闲端口", XRAY_PORT_MIN, size))
}

// ✅ Functional: 过滤并转换
pub fn wipe_targets<'a>(targets: &'a [&'a str]) -> Vec<(&'a str, Result<()>)> {
    targets
        .iter()
        .map(|&target| {
            let path = std::path::Path::new(target);
            let result = if path.exists() {
                crate::logic::security::secure_wipe_path(path)
            } else {
                Ok(())
            };
            (target, result)
        })
        .collect()
}
```

### Anti-Patterns / 反模式

```rust
// ❌ Imperative: 不必要的可变状态
pub async fn get_locked_ranges_bad() -> Vec<(u16, u16)> {
    let data = load_port_alloc().await.unwrap_or_default();
    let mut ranges = Vec::new();  // 不必要的 mut
    for range in &data.locked_ranges {
        ranges.push((range.start, range.end));
    }
    ranges
}

// ❌ Imperative: 手动循环实现 Iterator 已经提供的功能
fn find_consecutive_range_bad(occupied: &HashSet<u16>, size: u16) -> Result<u16> {
    for main_port in XRAY_PORT_MIN..=(XRAY_PORT_MAX.saturating_sub(size)) {
        let mut found = true;
        for port in main_port..(main_port + size) {
            if occupied.contains(&port) {
                found = false;
                break;
            }
        }
        if found {
            return Ok(main_port);
        }
    }
    anyhow::bail!("...")
}
```

---

## 2. Imperative Paradigm / 指令式范式

### When to Use / 适用场景

| Scenario / 场景 | Example / 示例 | File Location / 文件位置 |
|----------------|----------------|-------------------------|
| Sequential I/O / 顺序 I/O | BBR3 installation / BBR3 安装 | `maintenance.rs:112` |
| State mutation / 状态变更 | Progress callback / 进度回调 | `maintenance.rs:112` |
| Early returns / 早期返回 | Error handling / 错误处理 | Various / 各处 |
| Resource management / 资源管理 | File operations / 文件操作 | `singbox/config.rs` |

### When NOT to Use / 避免使用

- Pure data transformation without side effects / 纯数据转换（无副作用）
- Building configuration objects / 构建配置对象
- Simple collection mappings / 简单集合映射

### Correct Examples / 正确示例

```rust
// ✅ Imperative: 顺序 I/O 操作，带状态变更
pub async fn install_bbr3_with_progress<F>(&self, callback: F) -> Result<()>
where
    F: Fn(usize, &str),
{
    callback(1, "🔧 修复主机名解析...");
    fix_local_hostname_resolution().await?;

    callback(2, "📦 检查并安装依赖...");
    ensure_bbr3_dependencies().await?;

    callback(3, "⚙️ 配置内核参数...");
    Self::apply_sysctl_params().await?;

    callback(4, "🔄 加载 BBR 模块...");
    Self::load_bbr_module().await?;

    callback(5, "✅ 验证安装...");
    Self::verify_bbr3_installation().await?;

    Ok(())
}

// ✅ Imperative: 证书解析 - 状态机更适合指令式
async fn ensure_tls_certificates() -> Result<()> {
    // 检查证书是否存在
    if tokio::fs::try_exists(singbox::TLS_CERT).await.unwrap_or(false)
        && tokio::fs::try_exists(singbox::TLS_KEY).await.unwrap_or(false)
    {
        return Ok(());
    }

    // 生成证书
    let output = tokio::process::Command::new(singbox::BIN)
        .args(["generate", "tls-keypair", "tls", "-m", "456"])
        .output()
        .await
        .context("生成 TLS 证书失败")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "生成证书失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // 解析输出 - 手动状态机
    let output_str = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = output_str.lines().collect();

    let mut key_content = String::new();
    let mut cert_content = String::new();
    let mut in_key = false;
    let mut in_cert = false;

    for line in lines {
        if line.contains("BEGIN PRIVATE KEY") {
            in_key = true;
            key_content.push_str(line);
            key_content.push('\n');
            continue;
        }
        if line.contains("END PRIVATE KEY") {
            key_content.push_str(line);
            key_content.push('\n');
            in_key = false;
            continue;
        }
        if line.contains("BEGIN CERTIFICATE") {
            in_cert = true;
            cert_content.push_str(line);
            cert_content.push('\n');
            continue;
        }
        if line.contains("END CERTIFICATE") {
            cert_content.push_str(line);
            cert_content.push('\n');
            in_cert = false;
            continue;
        }

        if in_key {
            key_content.push_str(line);
            key_content.push('\n');
        }
        if in_cert {
            cert_content.push_str(line);
            cert_content.push('\n');
        }
    }

    tokio::fs::write(singbox::TLS_KEY, key_content).await?;
    tokio::fs::write(singbox::TLS_CERT, cert_content).await?;

    Ok(())
}

// ✅ Imperative: 顺序步骤，清晰表达执行流程
pub async fn batch_create_hysteria2(
    count: usize,
    ip_version: IpVersion,
    enable_obfs: bool,
) -> Result<BatchCreationResult> {
    // 检查限制
    if !PortAllocator::check_hysteria2_limit().await? {
        return Err(anyhow::anyhow!("已达到最大 Hysteria2 配置数量限制（50个）"));
    }

    // 获取 IP
    let host = match ip_version {
        IpVersion::IPv4 | IpVersion::SplitStackV4Primary => {
            SystemMonitor::get_public_ip().await?
        }
        IpVersion::IPv6 | IpVersion::SplitStackV6Primary => {
            SystemMonitor::get_public_ipv6().await?
        }
    };

    let geoip = crate::logic::geoip::GeoIPService::new();
    let country_code = geoip.get_country_code().await;

    let mut selector = SNISelector::get_for_country(&country_code);
    let mut links = Vec::new();
    let mut configs = Vec::new();

    // 循环创建配置 - 混合函数式与指令式
    for i in 0..count {
        let sni = selector.next();
        let (main_port, hop_range) = PortAllocator::allocate_hysteria2().await?;
        let port = main_port;

        let password = Hysteria2Config::generate_password();
        let tag = format!("HYSTERIA2-{}-{}", i + 1, &password[..8]);

        let config = if enable_obfs {
            let obfs_password = Hysteria2Config::generate_obfs_password();
            Hysteria2Config::with_obfs(
                port,
                password.clone(),
                sni.clone(),
                "salamander".to_string(),
                obfs_password,
            )
        } else {
            Hysteria2Config::new(port, password.clone(), sni.clone())
        };

        let link = if enable_obfs {
            config.to_client_link_with_hopping_and_obfs(&host, &tag, hop_range)
        } else {
            config.to_client_link_with_hopping(&host, &tag, hop_range)
        };

        links.push(link);
        configs.push(config.to_inbound_json(&tag));

        // 副作用：防火墙规则
        let _ = MaintenanceManager::allow_port(main_port).await;
        let _ = MaintenanceManager::allow_port_range(hop_range.0, hop_range.1).await;

        let has_ipv6 = SystemMonitor::get_public_ipv6().await.is_ok();
        if has_ipv6 {
            let _ = MaintenanceManager::allow_port_range_v6(hop_range.0, hop_range.1).await;
        }

        Self::add_port_hopping_firewall_rules_v4(main_port, hop_range).await?;
        if has_ipv6 {
            Self::add_port_hopping_firewall_rules_v6(main_port, hop_range).await?;
        }
    }

    // 保存配置并重载服务
    let (filename, _path) = Self::save_standalone_config(configs, "hysteria2").await?;
    Self::ensure_tls_certificates().await?;
    Self::reload_service().await?;

    Ok(BatchCreationResult {
        links,
        config_file: Some(filename),
        backup_file: None,
        created_count: count,
    })
}
```

### Anti-Patterns / 反模式

```rust
// ❌ Declarative: 不要把指令式 I/O 改成函数式链
async fn setup_bad() -> Result<()> {
    // 这不是函数式的正确用法
    [step_one, step_two, step_three]
        .iter()
        .try_for_each(|f| f())  // 掩盖了顺序依赖关系
        .await?;
    Ok(())
}

// ❌ Mixed: 不要在纯数据转换中使用 async
async fn transform_data_bad(data: &Data) -> TransformedData {
    // 这是错误的使用场景
    data.iter()
        .map(|x| x.process())  // 无需 async
        .collect()
}
```

---

## 3. Declarative Paradigm / 声明式范式

### When to Use / 适用场景

| Scenario / 场景 | Example / 示例 | File Location / 文件位置 |
|----------------|----------------|-------------------------|
| JSON config generation / JSON 配置生成 | `json!` macros | `singbox/config.rs:545` |
| UI keyboard construction / UI 键盘构建 | `InlineKeyboardMarkup` | `main.rs:635` |
| Route matching / 路由匹配 | Callback handling | `main.rs:821` |
| Static configuration / 静态配置 | Network optimization constants | `maintenance.rs:21` |

### When NOT to Use / 避免使用

- Complex business logic with many conditions / 复杂业务逻辑
- I/O operations / I/O 操作
- State-dependent transformations / 依赖状态的转换

### Correct Examples / 正确示例

```rust
// ✅ Declarative: JSON 配置生成 - 配置即代码
let full_config = json!({
    "log": {
        "level": "warning",
        "output": "/var/log/sing-box.log"
    },
    "dns": {
        "servers": [
            {"tag": "dns", "type": "udp", "server": "8.8.8.8", "domain_resolver": "local"},
            {"tag": "local", "type": "local"}
        ]
    },
    "route": {
        "default_domain_resolver": "dns"
    },
    "inbounds": configs,
    "outbounds": [
        {"type": "direct", "tag": "direct"},
        {"type": "block", "tag": "block"}
    ]
});

// ✅ Declarative: UI 键盘构建 - 声明式 API
fn build_custom_day_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("周一", "s_custom_set:day:Mon"),
            InlineKeyboardButton::callback("周二", "s_custom_set:day:Tue"),
            InlineKeyboardButton::callback("周三", "s_custom_set:day:Wed"),
            InlineKeyboardButton::callback("周四", "s_custom_set:day:Thu"),
            InlineKeyboardButton::callback("周五", "s_custom_set:day:Fri"),
        ],
        vec![
            InlineKeyboardButton::callback("周六", "s_custom_set:day:Sat"),
            InlineKeyboardButton::callback("周日", "s_custom_set:day:Sun"),
        ],
        vec![
            InlineKeyboardButton::callback("🔙 返回", "s_custom"),
        ],
    ])
}

// ✅ Declarative: 路由匹配 - 模式匹配
async fn handle_callback_query(bot: &Bot, callback: CallbackQuery) -> Result<()> {
    let data = callback.data.unwrap_or_default();
    match data.as_str() {
        "m_main" => show_main_menu(bot, &callback).await?,
        "m_ops_center" => show_ops_center(bot, &callback).await?,
        "m_install" => show_install_menu(bot, &callback).await?,
        "m_uninstall" => show_uninstall_menu(bot, &callback).await?,
        "m_upgrade" => show_upgrade_menu(bot, &callback).await?,
        "m_security" => show_security_menu(bot, &callback).await?,
        "m_backup" => show_backup_menu(bot, &callback).await?,
        "m_restore" => show_restore_menu(bot, &callback).await?,
        "m_self_destruct" => handle_self_destruct(bot, &callback).await?,
        d if d.starts_with("sb_h2_exec:") => handle_singbox_hy2(bot, &callback, d).await?,
        d if d.starts_with("sb_h2_del:") => handle_singbox_hy2_delete(bot, &callback, d).await?,
        d if d.starts_with("sb_tuic_exec:") => handle_singbox_tuic(bot, &callback, d).await?,
        _ => {}
    }
    Ok(())
}

// ✅ Declarative: 静态配置 - 数据即代码
const COMBINED_NETWORK_OPTIMIZE_CONF: &str = r#"fs.file-max = 1000000
fs.inotify.max_user_instances = 8192
fs.inotify.max_user_watches = 524288
vm.swappiness = 60
vm.dirty_ratio = 15
vm.dirty_background_ratio = 5
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.core.rmem_default = 16777216
net.core.wmem_default = 16777216
net.core.optmem_max = 25165824
net.core.netdev_max_backlog = 16384
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216
net.ipv4.tcp_fastopen = 3
net.ipv4.tcp_max_tw_buckets = 2000000
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 10
net.ipv4.tcp_slow_start_after_idle = 0
net.ipv4.tcp_keepalive_time = 300
net.ipv4.tcp_keepalive_intvl = 30
net.ipv4.tcp_keepalive_probes = 5
net.ipv4.tcp_max_syn_backlog = 8192
net.ipv4.tcp_syncookies = 1
net.ipv4.tcp_synack_retries = 2
net.ipv4.tcp_syn_retries = 5
net.ipv4.ip_local_port_range = 1000 65535
net.ipv4.conf.all.rp_filter = 1
net.ipv4.conf.default.rp_filter = 1
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.default.send_redirects = 0
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.default.accept_source_route = 0
net.ipv6.conf.all.accept_redirects = 0
net.ipv6.conf.default.accept_redirects = 0
net.ipv6.conf.all.accept_source_route = 0
net.ipv6.conf.default.accept_source_route = 0
"#;
```

### Anti-Patterns / 反模式

```rust
// ❌ Imperative: 不要用 if-else 链替代模式匹配
async fn handle_callback_bad(bot: &Bot, callback: CallbackQuery) -> Result<()> {
    let data = callback.data.unwrap_or_default();

    if data == "m_main" {
        show_main_menu(bot, &callback).await?;
    } else if data == "m_ops_center" {
        show_ops_center(bot, &callback).await?;
    } else if data == "m_install" {
        show_install_menu(bot, &callback).await?;
    } else if data.starts_with("sb_h2_exec:") {
        handle_singbox_hy2(bot, &callback, &data).await?;
    }
    // ... 更多 if-else

    Ok(())
}

// ❌ Imperative: 不要手动构建复杂 JSON
fn build_config_bad() -> Value {
    let mut map = serde_json::Map::new();
    map.insert("log".to_string(), serde_json::json!({"level": "warning"}));
    // 手动构建所有字段...
    Value::Object(map)
}
```

---

## 4. Core Rules / 核心规则

### Rule 1: Separate Pure Functions from Side Effects / 分离纯函数与副作用

```
Split / 分离:
    generate_configs()    → 纯函数，函数式
    apply_configs()      → 副作用，指令式
```

**Before:**
```rust
async fn create_and_apply() -> Result<()> {
    let config = generate();    // 纯转换
    write_file(config).await?; // I/O
    reload_service().await?;   // I/O
}
```

**After:**
```rust
fn generate_config() -> Config { /* 纯函数式 */ }

async fn apply_config(config: Config) -> Result<()> {
    write_file(config).await?;
    reload_service().await?;
}
```

### Rule 2: Use Iterator Over Manual Loops / 使用 Iterator 替代手动循环

```rust
// ❌ Bad: 手动循环
let mut result = Vec::new();
for item in collection {
    if item.is_valid() {
        result.push(item.transform());
    }
}

// ✅ Good: Iterator 链
let result: Vec<_> = collection
    .iter()
    .filter(|item| item.is_valid())
    .map(|item| item.transform())
    .collect();
```

### Rule 3: Keep Sequential I/O Imperative / I/O 操作保持指令式

```rust
// ✅ Correct: 顺序步骤清晰表达
async fn setup() -> Result<()> {
    step_one().await?;
    step_two().await?;
    step_three().await?;
    Ok(())
}
```

### Rule 4: Use Declarative Macros for Configuration / 配置使用声明式宏

```rust
// ✅ Correct: json! 宏
let config = json!({
    "setting": value,
    "nested": {"key": "value"}
});
```

### Rule 5: Prefer ? Operator for Error Handling / 错误处理优先使用 ? 操作符

```rust
// ✅ Good: 清晰的错误传播
async fn do_something() -> Result<()> {
    let data = load_data().await?;
    process_data(&data)?;
    save_result().await?;
    Ok(())
}
```

### Rule 6: Keep Complex State Machines Imperative / 复杂状态机保持指令式

```rust
// ✅ Correct: 状态机使用指令式
let mut state = ParseState::Looking;
for line in lines {
    match (&state, line.contains("BEGIN")) {
        (ParseState::Looking, true) => state = ParseState::InKey,
        (ParseState::InKey, true) => state = ParseState::Looking,
        // ...
    }
}
```

### Rule 7: Build UI Declaratively / UI 构建使用声明式

```rust
// ✅ Correct: 声明式键盘构建
fn build_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        rows...
    ])
}
```

### Rule 8: Use Declarative Routing / 路由匹配使用声明式

```rust
// ✅ Correct: 模式匹配路由
match callback_data {
    "menu_main" => handle_main(),
    "menu_settings" => handle_settings(),
    d if d.starts_with("cmd:") => handle_command(d),
    _ => handle_unknown(),
}
```

---

## 5. Module Guidelines / 模块指导

| Module / 模块 | Current State / 当前状态 | Recommended Paradigm / 推荐范式 | Priority / 优先级 |
|--------------|-------------------------|-------------------------------|------------------|
| `singbox/config.rs` | Mixed (70% Imperative, 25% Declarative, 5% Functional) | Separate pure/impure, increase Functional | P0 |
| `port_allocator.rs` | Good (60% Imperative, 40% Functional) | Keep, minor improvements | P2 |
| `maintenance.rs` | Correct (85% Imperative, 10% Declarative, 5% Functional) | Keep as-is | P2 |
| `config.rs` | Good (45% Imperative, 50% Declarative, 5% Functional) | Increase Functional for batch ops | P1 |
| `main.rs` | Mixed (60% Imperative, 40% Declarative) | Add declarative routing table | P1 |

---

## 6. Decision Tree / 决策树

```
Is this a data transformation? / 这是数据转换吗？
    │
    ├─ Yes → Use Functional / → 使用函数式
    │         filter() / map() / collect()
    │
    └─ No → Is this an I/O operation or state mutation?
            │                   这是 I/O 操作或状态变更吗？
            │
            ├─ Yes → Use Imperative / → 使用指令式
            │         async/await, sequential steps
            │
            └─ No → Is this configuration or UI construction?
                    │        这是配置或 UI 构建吗？
                    │
                    ├─ Yes → Use Declarative / → 使用声明式
                    │        json! macros, pattern matching
                    │
                    └─ No → Consider refactoring / → 考虑重构
```

---

## 7. References / 参考

- [Rust Iterator Documentation](https://doc.rust-lang.org/std/iter/trait.Iterator.html)
- [tokio.rs - Async in Rust](https://tokio.rs)
- [serde_json - json! macro](https://docs.rs/serde_json/latest/serde_json/macro.json.html)

---

## 8. Examples Directory / 示例目录

See `examples/` directory for additional code samples:

- `examples/functional_data_transform.rs` - 函数式数据转换示例
- `examples/imperative_io_operations.rs` - 指令式 I/O 操作示例
- `examples/declarative_configuration.rs` - 声明式配置生成示例
- `examples/checklist.md` - 重构检查清单

---

*Last updated: 2026-04-25*
*Version: 1.0*
