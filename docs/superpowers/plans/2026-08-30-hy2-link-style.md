# hy2 端口跳跃分享链接按客户端格式区分 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 hy2 端口跳跃分享链接按目标客户端生成两种格式(官方 multi-port / v2rayN mport),在 bot 确认页三按钮选择。

**Architecture:** 在 `hysteria2.rs` 引入 `Hy2LinkStyle { Official, V2rayN }` 枚举,两个跳跃链接方法增加 style 参数并按 style 输出不同 URI;`batch_create_hysteria2` 增加 `link_style` 参数逐层透传;`singbox.rs` 确认页由两个按钮改为三个按钮,exec 回调扩展为 5 段解析;i18n 增加按钮文案。非跳跃链接、inbound 配置、一键部署行为不变。

**Tech Stack:** Rust (edition 2024), tokio, rust_i18n, telegram/discord bot adapter。

**Spec:** `docs/superpowers/specs/2026-08-30-hy2-link-style-design.md`

## Global Constraints

- 工作目录:worktree `.worktrees/feat-hy2-link-style` 下的 `rust/aegis`(所有命令在该目录执行)
- 非跳跃链接格式不变;两种跳跃风格都保留 `pinSHA256`、都不带 `insecure`
- 每个任务结束必须 `cargo test` 全绿(基线:678 passed, 0 failed),并且 git 提交
- Rust 质量门禁(完成时执行):`cargo fmt`、`cargo clippy -- -D warnings`、`cargo test`

---

### Task 1: `Hy2LinkStyle` 枚举 + 跳跃链接方法支持风格(TDD)

**Files:**
- Modify: `src/core/singbox/hysteria2.rs`(枚举 + `to_client_link_with_hopping`:168-194 + `to_client_link_with_hopping_and_obfs`:220-254 + 测试模块)
- Modify: `src/core/singbox/hy2_batch.rs:81-89`(调用点硬编码 `Hy2LinkStyle::Official`,保持编译绿)

**Interfaces:**
- Produces: `pub enum Hy2LinkStyle { Official, V2rayN }`(derive `Debug, Clone, Copy, PartialEq, Eq`);`to_client_link_with_hopping(&self, host: &str, name: &str, hop_range: (u16, u16), style: Hy2LinkStyle) -> String`;`to_client_link_with_hopping_and_obfs(&self, host: &str, name: &str, hop_range: (u16, u16), style: Hy2LinkStyle) -> String`(签名均增加第四个参数 `style`)

- [ ] **Step 1: 写失败测试**(在 `hysteria2.rs` 测试模块追加)

在现有 `mod tests` 内新增,断言 V2rayN 风格格式。注意现有测试 `test_hysteria2_to_client_link_with_hopping`(395 行附近)、`test_hysteria2_to_client_link_with_hopping_and_obfs`(464 行附近)、`test_hysteria2_to_client_link_with_gecko_hopping`(447 行附近)的调用需补 `Hy2LinkStyle::Official` 参数(本步一并更新,保证它们仍测官方格式)。

```rust
#[test]
fn test_hysteria2_to_client_link_hopping_v2rayn_style() {
    let config = Hysteria2Config::new(8443, "test_password".into(), "sni.example.com".into());
    let link = config.to_client_link_with_hopping("1.2.3.4", "MyNode", (8444, 8543), Hy2LinkStyle::V2rayN);
    assert!(link.starts_with("hysteria2://"));
    assert!(link.contains("@1.2.3.4:8443?"));
    assert!(!link.contains(":8443,8444"));
    assert!(link.contains("mport=8444-8543"));
    assert!(!link.contains("hop_interval"));
    assert!(link.contains("sni=sni.example.com"));
    assert!(link.contains("#MyNode"));
}

#[test]
fn test_hysteria2_to_client_link_hopping_obfs_v2rayn_style() {
    let config = Hysteria2Config::with_obfs(
        8443, "test_password".into(), "sni.example.com".into(),
        Hysteria2ObfsType::Salamander, "obfs_secret".into(),
    );
    let link = config.to_client_link_with_hopping_and_obfs("1.2.3.4", "MyNode", (8444, 8543), Hy2LinkStyle::V2rayN);
    assert!(link.contains("@1.2.3.4:8443?"));
    assert!(link.contains("mport=8444-8543"));
    assert!(!link.contains("hop_interval"));
    assert!(link.contains("obfs=salamander"));
    assert!(link.contains("obfs-password=obfs_secret"));
}

#[test]
fn test_hysteria2_to_client_link_hopping_v2rayn_keeps_pin() {
    let config = Hysteria2Config::new(8443, "pw".into(), "s.example.com".into())
        .with_pin_sha256("AA:BB:CC".into());
    let link = config.to_client_link_with_hopping("1.2.3.4", "N", (8444, 8543), Hy2LinkStyle::V2rayN);
    assert!(link.contains("pinSHA256=AA:BB:CC"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test test_hysteria2_to_client_link_hopping_v2rayn_style test_hysteria2_to_client_link_hopping_obfs_v2rayn_style test_hysteria2_to_client_link_hopping_v2rayn_keeps_pin`
Expected: 编译失败(`Hy2LinkStyle` 未定义、`to_client_link_with_hopping` 参数数量不匹配)

- [ ] **Step 3: 最小实现**

在 `hysteria2.rs` 结构体定义前新增枚举:

```rust
/// 端口跳跃分享链接的目标客户端格式。
/// - `Official`: 官方 URI Scheme 的端口位置 multi-port + `hop_interval`
/// - `V2rayN`: v2rayN 系客户端的 `mport` 查询参数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hy2LinkStyle {
    Official,
    V2rayN,
}
```

`to_client_link_with_hopping` 改为:

```rust
pub fn to_client_link_with_hopping(
    &self,
    host: &str,
    name: &str,
    hop_range: (u16, u16),
    style: Hy2LinkStyle,
) -> String {
    let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
    let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
    let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
    let pin_param = self
        .pin_sha256
        .as_ref()
        .map(|p| format!("&pinSHA256={}", p))
        .unwrap_or_default();
    match style {
        Hy2LinkStyle::Official => format!(
            "hysteria2://{}@{}:{},{}-{}?sni={}&alpn=h3{}&hop_interval=30s#{}",
            encoded_password,
            host,
            self.port,
            hop_range.0,
            hop_range.1,
            encoded_sni,
            pin_param,
            encoded_name
        ),
        Hy2LinkStyle::V2rayN => format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&mport={}-{}#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            pin_param,
            hop_range.0,
            hop_range.1,
            encoded_name
        ),
    }
}
```

`to_client_link_with_hopping_and_obfs` 改为(签名加 `style: Hy2LinkStyle`,官方分支保持现状字符串,新增 V2rayN 分支——V2rayN 分支同样去掉 `hop_interval`、端口位置只放 `self.port`、范围进 `mport={}-{}`):

```rust
pub fn to_client_link_with_hopping_and_obfs(
    &self,
    host: &str,
    name: &str,
    hop_range: (u16, u16),
    style: Hy2LinkStyle,
) -> String {
    let encoded_password = utf8_percent_encode(&self.password, NON_ALPHANUMERIC).to_string();
    let encoded_sni = utf8_percent_encode(&self.sni, NON_ALPHANUMERIC).to_string();
    let encoded_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
    let encoded_obfs_password = utf8_percent_encode(
        self.obfs_password.as_deref().unwrap_or(""),
        NON_ALPHANUMERIC,
    )
    .to_string();
    let obfs_value = self.obfs_type.map(|t| t.as_str()).unwrap_or("salamander");
    let pin_param = self
        .pin_sha256
        .as_ref()
        .map(|p| format!("&pinSHA256={}", p))
        .unwrap_or_default();
    match style {
        Hy2LinkStyle::Official => format!(
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
        ),
        Hy2LinkStyle::V2rayN => format!(
            "hysteria2://{}@{}:{}?sni={}&alpn=h3{}&mport={}-{}&obfs={}&obfs-password={}#{}",
            encoded_password,
            host,
            self.port,
            encoded_sni,
            pin_param,
            hop_range.0,
            hop_range.1,
            obfs_value,
            encoded_obfs_password,
            encoded_name
        ),
    }
}
```

同步更新 `hy2_batch.rs:81-89` 两处跳跃调用,补 `Hy2LinkStyle::Official` 参数(保证编译绿,行为=现状):

```rust
let link = if obfs_type.is_some() && enable_hopping {
    config.to_client_link_with_hopping_and_obfs(&host, &tag, hop_range, Hy2LinkStyle::Official)
} else if obfs_type.is_some() {
    config.to_client_link_with_obfs(&host, &tag)
} else if enable_hopping {
    config.to_client_link_with_hopping(&host, &tag, hop_range, Hy2LinkStyle::Official)
} else {
    config.to_client_link(&host, &tag)
};
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test hysteria2`
Expected: 全部通过(新增 3 个 + 更新后的现有跳跃测试)

- [ ] **Step 5: Commit**

```bash
git add rust/aegis/src/core/singbox/hysteria2.rs rust/aegis/src/core/singbox/hy2_batch.rs
git commit -m "feat(singbox): Hy2LinkStyle 枚举及跳跃链接 V2rayN 格式支持"
```

---

### Task 2: `batch_create_hysteria2` 增加 `link_style` 参数并透传

**Files:**
- Modify: `src/core/singbox/hy2_batch.rs:15-120`(签名 + 透传)
- Modify: `src/shared/handlers/singbox.rs:636`(调用点占位传 `Hy2LinkStyle::Official`)
- Modify: `src/shared/handlers/ops.rs:772`(调用点占位传 `Hy2LinkStyle::Official`)

**Interfaces:**
- Consumes: `Hy2LinkStyle`(Task 1)
- Produces: `batch_create_hysteria2(count: usize, ip_version: IpVersion, obfs_type: Option<Hysteria2ObfsType>, enable_hopping: bool, link_style: Hy2LinkStyle) -> Result<BatchCreationResult>`

**说明:** hy2_batch.rs 无测试基建(依赖 PortAllocator/SystemMonitor/网络),此任务以编译 + 现有测试全绿验证。

- [ ] **Step 1: 修改签名与透传**

`hy2_batch.rs:15` 签名增加 `link_style: Hy2LinkStyle`,并将 Task 1 硬编码的两处 `Hy2LinkStyle::Official` 替换为参数:

```rust
pub async fn batch_create_hysteria2(
    count: usize,
    ip_version: IpVersion,
    obfs_type: Option<Hysteria2ObfsType>,
    enable_hopping: bool,
    link_style: Hy2LinkStyle,
) -> Result<BatchCreationResult> {
```

两处调用改为 `..., link_style)`(替换 `Hy2LinkStyle::Official`)。

- [ ] **Step 2: 更新两个调用点(占位)**

`singbox.rs:636`:
```rust
match SingBoxConfigManager::batch_create_hysteria2(
    count,
    ip_version,
    obfs_type,
    hopping_enabled,
    Hy2LinkStyle::Official,
)
```

`ops.rs:772`:
```rust
match SingBoxConfigManager::batch_create_hysteria2(3, ip_version, None, false, Hy2LinkStyle::Official).await {
```

- [ ] **Step 3: 编译 + 测试**

Run: `cargo build` 然后 `cargo test`
Expected: 编译通过,678 passed, 0 failed(行为未变:交互层仍只走官方格式)

- [ ] **Step 4: Commit**

```bash
git add rust/aegis/src/core/singbox/hy2_batch.rs rust/aegis/src/shared/handlers/singbox.rs rust/aegis/src/shared/handlers/ops.rs
git commit -m "feat(singbox): batch_create_hysteria2 透传 Hy2LinkStyle"
```

---

### Task 3: 确认页三按钮 + exec 回调 5 段解析 + i18n 文案

**Files:**
- Modify: `src/shared/handlers/singbox.rs`(确认页按钮 511-523 + exec 回调 600-640)
- Modify: `src/resources/i18n/zh.yml:80-82`(hop_title 文案 + 新按钮键)
- Modify: `src/resources/i18n/en.yml:70-72`(同上)

**Interfaces:**
- Consumes: `batch_create_hysteria2(..., link_style: Hy2LinkStyle)`(Task 2)
- 新回调数据格式:`sb_h2_exec:{ip}:{count}:{obfs}:{hopping}:{style}`,其中 `style ∈ {"official","v2rayn",""}`(无跳跃时为空)

**说明:** singbox.rs 无测试基建(handler 依赖 adapter mock),以编译 + 现有测试全绿验证。

- [ ] **Step 1: i18n 新键(zh.yml:80-82 区域)**

将 `singbox_h2_hop_title` 文案改为说明三种选择,并新增两个按钮键:

```yaml
  singbox_h2_hop_title: "🔀 <b>端口跳跃</b>\n\n端口跳跃允许客户端在多个 UDP 端口之间切换，以绕过 QoS/限速。\n\n<b>跳跃链接格式因客户端而异：</b>\n• sing-box 系：官方 multi-port 格式\n• v2rayN 系：mport 参数\n\n请选择："
  singbox_h2_hop_disable: "🔴 不启用端口跳跃"
  singbox_h2_hop_enable_singbox: "🔀 跳跃 · sing-box（官方格式）"
  singbox_h2_hop_enable_v2rayn: "🔀 跳跃 · v2rayN（mport）"
```

- [ ] **Step 2: i18n 新键(en.yml:70-72 区域)**

```yaml
  singbox_h2_hop_title: "🔀 <b>Port Hopping</b>\n\nPort hopping lets the client switch between multiple UDP ports to bypass QoS/throttling.\n\n<b>Hopping link format differs by client:</b>\n• sing-box family: official multi-port format\n• v2rayN family: mport parameter\n\nChoose:"
  singbox_h2_hop_disable: "🔴 Disable port hopping"
  singbox_h2_hop_enable_singbox: "🔀 Hopping · sing-box (official format)"
  singbox_h2_hop_enable_v2rayn: "🔀 Hopping · v2rayN (mport)"
```

- [ ] **Step 3: 确认页三按钮(singbox.rs:508-523)**

将 `rows` 从两个执行按钮改为三个:

```rust
let rows = vec![
    vec![InlineButton {
        text: t!("menu.singbox_h2_hop_disable").into(),
        data: format!("sb_h2_exec:{}:{}:{}:0:", ip_ver, count, obfs_enabled),
    }],
    vec![InlineButton {
        text: t!("menu.singbox_h2_hop_enable_singbox").into(),
        data: format!("sb_h2_exec:{}:{}:{}:1:official", ip_ver, count, obfs_enabled),
    }],
    vec![InlineButton {
        text: t!("menu.singbox_h2_hop_enable_v2rayn").into(),
        data: format!("sb_h2_exec:{}:{}:{}:1:v2rayn", ip_ver, count, obfs_enabled),
    }],
    vec![InlineButton {
        text: t!("menu.back_user").into(),
        data: format!("sb_h2_obfs:{}:{}", ip_ver, count),
    }],
];
```

- [ ] **Step 4: exec 回调 5 段解析(singbox.rs:600-640)**

`parts.len()` 校验从 4 改为 5,新增 style 解析,并传给 batch 调用:

```rust
if parts.len() != 5 {
    // ... 现有参数错误处理不变
}
let ip_ver = parts[0];
let count: usize = parts[1].parse().unwrap_or(1);
let obfs_type = match parts[2] {
    "1" => Some(Hysteria2ObfsType::Salamander),
    "2" => Some(Hysteria2ObfsType::Gecko),
    _ => None,
};
let hopping_enabled: bool = parts[3] == "1";
let link_style = match parts[4] {
    "v2rayn" => Hy2LinkStyle::V2rayN,
    _ => Hy2LinkStyle::Official,
};
```

并将 `batch_create_hysteria2(count, ip_version, obfs_type, hopping_enabled)` 改为 `batch_create_hysteria2(count, ip_version, obfs_type, hopping_enabled, link_style)`。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build` 然后 `cargo test`
Expected: 编译通过,678 passed, 0 failed

- [ ] **Step 6: Commit**

```bash
git add rust/aegis/src/shared/handlers/singbox.rs rust/aegis/src/resources/i18n/zh.yml rust/aegis/src/resources/i18n/en.yml
git commit -m "feat(singbox): hy2 确认页三按钮选择跳跃链接客户端格式"
```

---

### Task 4: 全量质量门禁 + 设计核对

**Files:** 无(验证任务)

- [ ] **Step 1: 质量门禁**

Run(worktree 的 `rust/aegis` 下):
```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```
Expected: fmt 无 diff(若有,说明步骤遗漏)、clippy 零警告、678+ 测试全绿

- [ ] **Step 2: 与设计文档核对**

逐条核对 `docs/superpowers/specs/2026-08-30-hy2-link-style-design.md`:
- §3 两种格式已实现(Official 现状 / V2rayN `mport`、无 `hop_interval`)
- §4 三按钮交互已实现
- D4 pinSHA256 保留、无 insecure(检查 `to_client_link_with_hopping*` 两个分支均含 `pin_param`)
- D5 一键部署(ops.rs:772)仍无跳跃、inbound 未动

- [ ] **Step 3: 汇报**

输出:实现摘要、测试结果、与设计差异(如有)、下一步(请求 code review)
