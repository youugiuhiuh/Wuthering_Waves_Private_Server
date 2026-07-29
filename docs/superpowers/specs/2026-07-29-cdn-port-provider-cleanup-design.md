# CDN 端口白名单 + DNS 提供商清理

**Date:** 2026-07-29
**Status:** Design (approved)

---

## 背景

1. **DNS 提供商安全审计**：当前 4 个提供商中包含 Aliyun、Dnspod（CCP 背景，数据风险），需移除。
2. **CDN 端口限制**：TLS xhttp 分享链接通过 Cloudflare CDN 代理时，非白名单端口被拒绝转发，导致客户端连接超时。此前 `batch_create_xhttp_tls_enhanced` 使用随机 10000-60000 端口不可达。

## 变更概要

1. `DnsProvider` 剔除 Aliyun/Dnspod，仅保留 Cloudflare、Route53
2. 新增 `cdn_ports()` 方法，返回 CDN 代理兼容端口白名单
3. `batch_create_xhttp_tls_enhanced` 根据已配置提供商选择端口池

---

## 变更1: DnsProvider 精简

### 涉及文件

| 文件 | 变更 |
|---|---|
| `rust/aegis/src/core/types.rs` | 删除 `Aliyun`、`Dnspod` 变体；新增 `cdn_ports()` 方法 |
| `rust/aegis/src/core/security/acme.rs` | `configured_provider_from` 匹配列表删除 Aliyun/Dnspod |
| `rust/aegis/src/shared/handlers/message.rs` | 删除 Aliyun/Dnspod 的按钮、解析、引导文案分支 |
| `rust/aegis/src/shared/handlers/xray.rs` | `provider_credential_guidance` 调用方（旧 match 臂不可达） |

### types.rs 变更

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsProvider {
    Cloudflare,
    Route53,
}

impl DnsProvider {
    pub const fn acme_flag(self) -> &'static str {
        match self {
            Self::Cloudflare => "dns_cf",
            Self::Route53 => "dns_aws",
        }
    }

    pub const fn credential_names(self) -> (&'static str, &'static str) {
        match self {
            Self::Cloudflare => ("CF_Token", "CF_Zone_ID"),
            Self::Route53 => ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
        }
    }

    /// CDN 代理模式可用端口。无 CDN 的提供商返回空切片。
    pub const fn cdn_ports(self) -> &'static [u16] {
        match self {
            Self::Cloudflare => &[443, 8443, 2053, 2083, 2087, 2096],
            Self::Route53 => &[],
        }
    }
}
```

### acme.rs 变更

`configured_provider_from` 删除 Aliyun/Dnspod：
```rust
fn configured_provider_from(config: &str) -> Option<DnsProvider> {
    [DnsProvider::Cloudflare, DnsProvider::Route53]
        .into_iter()
        .find(|provider| {
            let (first, second) = provider.credential_names();
            has_non_empty_assignment(config, first) && has_non_empty_assignment(config, second)
        })
}
```

### message.rs 变更

| 函数 | 改动 |
|---|---|
| `provider_buttons()` | 删除 Aliyun/Dnspod 按钮 |
| `parse_provider_selection()` | 删除 aliyun/ali/dns_ali, dnspod/dp/dns_dp 分支 |
| `provider_credential_guidance()` | 删除 Aliyun/Dnspod match 臂 |
| `show_provider_selection()` | 已删除的项不可达 |

### 测试变更

- `dns_provider_maps_to_acme_contract`：删除 Aliyun/Dnspod 断言
- `provider_guidance_runtime_never_returns_raw_keys`：迭代集合删除 Aliyun/Dnspod
- 新增 `cdn_ports_for_cloudflare_are_cf_proxy_ports`
- 新增 `route53_has_no_cdn_port_restriction`

---

## 变更2: CDN 端口限制

### 涉及文件

| 文件 | 变更 |
|---|---|
| `rust/aegis/src/core/xray/xhttp.rs` | `batch_create_xhttp_tls_enhanced` 端口选择逻辑 |

### xhttp.rs 变更

**当前逻辑：**
```
node[0]: 443（如空闲）else 随机 10000-60000
node[1..]: 随机 10000-60000
```

**新逻辑：**
```
若有 CDN 白名单：
    从 CDN 端口池顺序选取可用端口（含端口分配器锁范围检查），
    不足时放回已占用端口。
无 CDN 白名单：
    原随机策略不变。
```

```rust
let cdn_ports = AcmeManager::configured_provider()
    .map(|p| p.cdn_ports())
    .unwrap_or(&[]);

for i in 0..20 {
    let port: i32 = if !cdn_ports.is_empty() {
        // 从 CDN 端口池顺序选取可用端口
        select_cdn_port(cdn_ports, &port_allocator).await?
    } else if i == 0 && port_443_available {
        443
    } else {
        // 原随机逻辑
        loop {
            let p = rng.gen_range(10000..60000);
            // ... existing checks
        }
    };
    // ... rest unchanged
}
```

新增辅助函数：
```rust
async fn select_cdn_port(
    cdn_ports: &[u16],
    port_allocator: &PortAllocator,
) -> Result<i32> {
    for &port in cdn_ports {
        if port_allocator.is_port_in_locked_range(port).await { continue; }
        if MaintenanceManager::is_port_available(port).await {
            return Ok(port as i32);
        }
    }
    bail!("all CDN ports occupied or locked");
}
```

### 测试

- `tls_batch_uses_cf_ports_when_cloudflare_configured`（需 mock）
- 现有 `build_tls_node_returns_matching_config_and_link` 保持不变

---

## 不涉及的文件（边界）

| 排除 | 原因 |
|---|---|
| `deploy.yml` / CI | 无配置变更 |
| `Cargo.toml` | 无依赖变更 |
| 翻译文件 (i18n) | 删除的 provider 文案键虽成死键，主动删除不在本次范围内 |
| `dispatch.rs` | handler 路由不变，`DomainReady` 枚举不变 |
| `ops.rs` | `batch_create_xhttp_tls_enhanced` 调用方不感知端口选择逻辑 |
| `config.rs` | 入站/分享链接生成不变 |

---

## 规格自查

- [x] 无 TBD/TODO 残留
- [x] 变更范围一致（仅 provider 清理 + 端口限制）
- [x] 范围聚焦于单一目标
- [x] 无歧义需求
