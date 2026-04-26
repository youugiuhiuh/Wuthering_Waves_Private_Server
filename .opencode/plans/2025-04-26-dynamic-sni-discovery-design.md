# 动态SNI发现引擎设计文档

**版本**: 1.0  
**日期**: 2025-04-26  
**作者**: OpenCode  
**项目**: rust/tgbot  
**功能**: 动态邻居发现与SNI智能评分系统

---

## 1. 功能概述

为现有 Telegram Bot 新增动态SNI发现能力，通过主动扫描邻居IP的TLS证书，发现并评分优质SNI域名，形成独立的高质量SNI池。当动态池耗尽时，自动回退到静态PB文件。

**核心特性**:
- 手动触发扫描（通过Bot命令）
- C段(/24)和B段(/16)邻居发现
- 多维度评分（稳定性/权威性/地理）
- 正则表达式安全过滤
- 独立优质SNI池持久化
- 用户可选择数据源（静态/动态）
- 动态池耗尽自动回退静态文件

---

## 2. 使用场景

### 场景1：日本VPS首次配置
1. 用户选择"使用动态发现"
2. 启动扫描：扫描同C段256个IP
3. 发现47个活跃IP，提取120个域名
4. 过滤后剩余12个，评分后取TOP-10
5. 创建Reality配置时优先使用这10个SNI
6. 第11个配置开始，自动使用JP.pb文件

### 场景2：静态→动态切换
1. 用户当前使用US.pb
2. 手动触发扫描
3. 发现优质SNI后，自动切换到动态模式
4. 后续配置优先使用扫描结果

---

## 3. 架构设计

### 3.1 模块结构

```
src/logic/sni_discovery/
├── mod.rs              # 公共API导出
├── asn.rs              # ASN查询（IP→AS号）
├── neighbor.rs         # 邻居IP生成（C段/B段）
├── scanner.rs          # 异步443端口扫描
├── cert_fetcher.rs     # TLS握手+证书提取
├── analyzer.rs         # 域名分析（提取SAN）
├── scorer.rs           # 多维度评分计算
├── filter.rs           # 正则表达式过滤
└── pool.rs             # 优质SNI池管理
```

### 3.2 依赖关系

```
┌─────────────────────────────────────────┐
│         config.rs (批量创建)             │
│              │                          │
│              ▼                          │
│    ┌─────────────────┐                  │
│    │  acquire_sni()  │                  │
│    └────────┬────────┘                  │
│             │                           │
│     ┌───────┴───────┐                   │
│     ▼               ▼                   │
│ DynamicPool    SNISelector              │
│ (优先使用)     (回退使用)               │
│     │               │                   │
│     └───────┬───────┘                   │
│             ▼                           │
│     返回SNI给配置生成                    │
└─────────────────────────────────────────┘
```

---

## 4. 数据结构设计

### 4.1 核心结构

```rust
// mod.rs
pub struct DiscoveryConfig {
    pub enabled: bool,
    pub scan_c_segment: bool,      // 扫描C段 /24
    pub scan_b_segment: bool,      // 扫描B段 /16（采样）
    pub concurrency: usize,        // 并发数（默认50）
    pub b_segment_sample_size: usize, // B段采样数（默认1000）
    pub pool_size: usize,          // 保留TOP-N（默认50）
    pub weight_stability: u8,      // 稳定性权重 %
    pub weight_authority: u8,      // 权威性权重 %
    pub weight_geography: u8,      // 地理权重 %
    pub blacklist_patterns: Vec<String>, // 正则黑名单
}

pub struct DiscoveredDomain {
    pub domain: String,
    pub ip: IpAddr,
    pub port: u16,
    pub san_count: usize,               // SAN中域名总数
    pub cert_issuer: String,            // 颁发机构
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub scores: DomainScores,
    pub total_score: f64,               // 0.0-1.0
    pub discovered_at: DateTime<Utc>,
}

pub struct DomainScores {
    pub stability: f64,   // 历史稳定性
    pub authority: f64,   // 证书权威性
    pub geography: f64,   // 地理相关性
}

pub enum DiscoveryError {
    PoolExhausted { country_code: String },
    NoCandidatesFound,
    ScanTimeout,
    // ...
}
```

### 4.2 池状态持久化

```rust
// pool.rs
#[derive(Serialize, Deserialize)]
pub struct DynamicPoolState {
    pub domains: Vec<DiscoveredDomain>,
    pub shuffled_indices: Vec<usize>,
    pub used_count: usize,
    pub created_at: DateTime<Utc>,
    pub source_ip: String,          // 扫描时的公网IP
    pub source_country: String,     // 扫描时的国家码
}
```

使用AES-256-GCM加密存储到 `/etc/wwps/tgbot/sni_discovery/pool.enc`

---

## 5. 核心算法

### 5.1 评分算法（均衡发展）

```rust
// scorer.rs
impl DomainScorer {
    pub fn calculate_total(&self, domain: &DiscoveredDomain, target_country: &str) -> f64 {
        let stability = self.score_stability(domain);
        let authority = self.score_authority(&domain.cert_issuer);
        let geography = self.score_geography(&domain.domain, target_country);
        
        // 权重：稳定性33%，权威性33%，地理34%
        stability * 0.33 + authority * 0.33 + geography * 0.34
    }
    
    fn score_stability(&self, domain: &DiscoveredDomain) -> f64 {
        let validity_days = (domain.not_after - domain.not_before).num_days();
        let age_days = (Utc::now() - domain.not_before).num_days();
        
        // 有效期60-365天得满分
        let validity_score = match validity_days {
            60..=365 => 1.0,
            d if d < 60 => d as f64 / 60.0,
            _ => 365.0 / validity_days as f64,
        };
        
        // 已存在>30天得满分
        let age_score = (age_days as f64 / 30.0).min(1.0);
        
        validity_score * 0.6 + age_score * 0.4
    }
    
    fn score_authority(&self, issuer: &str) -> f64 {
        match issuer {
            i if i.contains("DigiCert") => 1.0,
            i if i.contains("Sectigo") => 0.9,
            i if i.contains("Entrust") => 0.9,
            i if i.contains("GlobalSign") => 0.85,
            i if i.contains("Google Trust Services") => 0.85,
            i if i.contains("Let's Encrypt") => 0.6,
            i if i.contains("self-signed") || i.contains("Self") => 0.0,
            _ => 0.5,
        }
    }
    
    fn score_geography(&self, domain: &str, target_country: &str) -> f64 {
        let tld = extract_tld(domain);
        
        // TLD与国家码匹配
        if tld.eq_ignore_ascii_case(target_country) {
            return 1.0;
        }
        
        // 同一大洲（简化映射）
        if is_same_region(&tld, target_country) {
            return 0.7;
        }
        
        // 通用域名
        match tld.as_str() {
            "com" | "net" | "org" | "io" => 0.5,
            _ => 0.3,
        }
    }
}
```

### 5.2 安全过滤（正则）

```rust
// filter.rs
pub struct DomainFilter {
    patterns: Vec<Regex>,
}

impl DomainFilter {
    pub fn with_defaults() -> Self {
        let default_patterns = vec![
            r"(?i)vpn",           // 不区分大小写匹配vpn
            r"(?i)proxy",
            r"(?i)node",
            r"(?i)v2ray",
            r"(?i)shadowsock",
            r"(?i)trojan",
            r"(?i)github",
            r"(?i)cloudflare",
            r"(?i)azure",
            r"(?i)aws\.amazon",
        ];
        
        Self::new(default_patterns).expect("Default patterns are valid")
    }
    
    pub fn is_allowed(&self, domain: &str) -> bool {
        !self.patterns.iter().any(|re| re.is_match(domain))
    }
}
```

### 5.3 回退机制

```rust
// pool.rs
impl DynamicPool {
    /// 获取下一个SNI，池空时返回错误携带国家码
    pub async fn get_next_sni(&mut self, geoip: &GeoIPService) 
        -> Result<String, DiscoveryError> 
    {
        // 尝试从动态池获取
        if let Some(idx) = self.state.shuffled_indices.pop() {
            self.state.used_count += 1;
            self.save().await?;
            return Ok(self.state.domains[idx].domain.clone());
        }
        
        // 池已空，获取当前国家码用于回退
        let country_code = geoip.get_country_code().await;
        log::info!(
            "Dynamic SNI pool exhausted (used {} domains), "
            "falling back to static PB: {}.pb",
            self.state.used_count,
            country_code
        );
        
        Err(DiscoveryError::PoolExhausted { country_code })
    }
}

// config.rs 中的使用
async fn acquire_sni(
    state: &AppState, 
    geoip: &GeoIPService
) -> Result<String> {
    let config = state.sni_source_config.lock().await;
    
    match config.source_type {
        SNISourceType::DynamicDiscovery => {
            // 优先尝试动态池
            let mut pool = DynamicPool::load().await?;
            match pool.get_next_sni(geoip).await {
                Ok(sni) => Ok(sni),
                Err(DiscoveryError::PoolExhausted { country_code }) => {
                    // 自动回退到对应国家的PB
                    drop(config); // 释放锁
                    let selector = SNISelector::get_for_country(&country_code);
                    Ok(selector.next())
                }
                Err(e) => Err(e.into()),
            }
        }
        SNISourceType::StaticFile => {
            // 直接使用静态PB
            let country_code = geoip.get_country_code().await;
            drop(config);
            let selector = SNISelector::get_for_country(&country_code);
            Ok(selector.next())
        }
    }
}
```

---

## 6. Bot界面设计

### 6.1 SNI数据源设置菜单

```
┌─────────────────────────────┐
│  🌐 SNI 数据源设置            │
├─────────────────────────────┤
│  当前: 动态发现 (10个可用)     │
│  国家: JP                    │
├─────────────────────────────┤
│  📁 使用静态文件 (JP.pb)     │
│  🔍 使用动态发现              │
│  ⚙️ 动态发现配置              │
│  ▶️ 开始扫描                  │
└─────────────────────────────┘
```

### 6.2 动态发现配置菜单

```
┌─────────────────────────────┐
│  🔍 动态发现配置              │
├─────────────────────────────┤
│  扫描范围:                   │
│  • C段 (/24): ✅            │
│  • B段 (/16): ❌            │
│                             │
│  扫描设置:                   │
│  • 并发数: 50               │
│  • 保留TOP: 50              │
│                             │
│  评分权重:                   │
│  • 稳定性: 33%              │
│  • 权威性: 33%              │
│  • 地理: 34%                │
├─────────────────────────────┤
│  📝 编辑正则黑名单           │
└─────────────────────────────┘
```

### 6.3 扫描进度界面

```
🔍 正在发现优质SNI...

━━━━━━━━━━━━━━━━━━ 45%

已扫描: 128/256 IP (50.0%)
发现证书: 47个
提取域名: 120个
通过过滤: 12个
优质候选: 3个

当前最佳: example.co.jp (评分: 0.89)

⏳ 预计剩余: 12秒
[取消扫描]
```

### 6.4 扫描完成界面

```
✅ 扫描完成！

发现结果:
• 扫描IP: 256个
• 活跃主机: 47个
• 提取域名: 120个
• 通过过滤: 12个
• 优质SNI: 10个 (已保存)

TOP-3 优质SNI:
1. example.co.jp (0.89)
2. shop.jp.example (0.85)
3. api.example.jp (0.82)

💾 已自动切换到动态发现模式
后续配置将优先使用扫描结果
```

---

## 7. 与现有系统集成

### 7.1 AppState扩展

```rust
// src/app/state.rs
pub struct AppState {
    // ... 现有字段 ...
    
    /// SNI数据源配置
    pub sni_source_config: Arc<Mutex<SNISourceConfig>>,
}

pub struct SNISourceConfig {
    pub source_type: SNISourceType,
    pub discovery_config: DiscoveryConfig,
}

pub enum SNISourceType {
    StaticFile,
    DynamicDiscovery,
}
```

### 7.2 BotSettings持久化

```rust
// src/bootstrap.rs
#[derive(Serialize, Deserialize, Clone)]
pub struct BotSettings {
    pub session_timeout_secs: u64,
    pub sni_source: SNISourceSetting,  // 新增
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SNISourceSetting {
    pub mode: String,  // "static" | "dynamic"
    pub discovery_enabled: bool,
}
```

### 7.3 回调路由

```rust
// src/main.rs handle_callback
match callback.data() {
    // ... 现有路由 ...
    
    // SNI设置
    "m_sni_settings" => handle_sni_settings(bot, chat_id, state).await,
    "a_sni_static" => switch_to_static(bot, chat_id, state).await,
    "a_sni_dynamic" => switch_to_dynamic(bot, chat_id, state).await,
    "m_discovery_config" => handle_discovery_config(bot, chat_id, state).await,
    
    // 扫描控制
    "a_discovery_run" => start_discovery_scan(bot, chat_id, state).await,
    "a_discovery_cancel" => cancel_discovery_scan(bot, chat_id, state).await,
    
    // 配置开关
    "a_discovery_toggle_c" => toggle_c_segment(bot, chat_id, state).await,
    "a_discovery_toggle_b" => toggle_b_segment(bot, chat_id, state).await,
    
    // 编辑配置
    "a_discovery_weights" => edit_weights_dialog(bot, chat_id, state).await,
    "a_discovery_blacklist" => edit_blacklist_dialog(bot, chat_id, state).await,
    
    // ...
}
```

---

## 8. 性能与安全考虑

### 8.1 性能优化

- **异步扫描**: 使用 `tokio::spawn` 并发扫描，默认50并发
- **连接超时**: 2秒TCP连接超时，避免长时间等待
- **TLS快速中断**: 收到Server Certificate后立即RST，不完成握手
- **B段采样**: /16段太大，随机采样1000个IP而非全扫
- **进度节流**: Telegram消息更新间隔500ms，避免API限流

### 8.2 安全防护

- **反蜜罐**: 正则过滤敏感关键词
- **证书验证**: 拒绝自签名证书
- **频率控制**: 单次扫描间隔限制（建议>1小时）
- **数据加密**: 动态池使用AES-256-GCM加密存储
- **权限控制**: 文件权限0o600，仅所有者可读写

---

## 9. 测试策略

### 9.1 单元测试

- `scorer.rs`: 评分算法边界条件
- `filter.rs`: 正则匹配测试
- `neighbor.rs`: IP段生成正确性

### 9.2 集成测试

- 完整扫描流程（使用测试IP段）
- 池持久化与加载
- 回退机制验证

### 9.3 模拟测试

- 使用mock TLS服务器测试证书提取
- 模拟GeoIP响应测试地理评分

---

## 10. 实现计划

### Phase 1: 基础模块（4-5天）
1. 创建 `sni_discovery/` 目录结构
2. 实现 `asn.rs` - ASN查询
3. 实现 `neighbor.rs` - 邻居IP生成
4. 实现 `scanner.rs` - 异步端口扫描

### Phase 2: 核心功能（4-5天）
5. 实现 `cert_fetcher.rs` - TLS证书抓取
6. 实现 `analyzer.rs` - 域名分析
7. 实现 `scorer.rs` - 评分算法
8. 实现 `filter.rs` - 正则过滤

### Phase 3: 池管理（3-4天）
9. 实现 `pool.rs` - 池管理与持久化
10. 实现回退机制
11. 集成测试

### Phase 4: Bot界面（3-4天）
12. 扩展 `AppState` 和 `BotSettings`
13. 实现回调路由和菜单
14. 扫描进度实时显示

### Phase 5: 集成与优化（2-3天）
15. 修改 `config.rs` 使用新SNI源
16. 端到端测试
17. 性能优化

**总计**: 16-21天

---

## 11. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 扫描被VPS商阻止 | 高 | 提供配置关闭扫描，仅使用静态PB |
| 扫描时间过长 | 中 | 可中断扫描，进度持久化 |
| 无合格SNI | 中 | 自动回退静态文件，用户体验无感 |
| 证书解析失败 | 低 | 错误处理，跳过该IP继续扫描 |

---

## 12. 附录

### 12.1 ASN查询API备选

1. **ip-api.com**: `http://ip-api.com/json/{ip}` (免费，非商业)
2. **ipinfo.io**: `https://ipinfo.io/{ip}/json` (有免费额度)
3. **Team Cymru**: DNS查询 `AS{ip}.asn.cymru.com`
4. **whois**: 备用方案，直接whois查询

### 12.2 文件路径

- 动态池加密文件: `/etc/wwps/tgbot/sni_discovery/pool.enc`
- 配置文件: `/etc/wwps/tgbot/sni_discovery/config.json`
- 密钥文件: `/etc/wwps/tgbot/sni_discovery/.key`

### 12.3 相关模块文档

- 现有SNI系统: `src/logic/sni_selector.rs`
- 状态持久化: `src/logic/sni_state.rs`
- TLS探测: `src/logic/tls_probe.rs`

---

**设计状态**: 待审核  
**下一步**: 用户审核通过后编写详细实现计划
