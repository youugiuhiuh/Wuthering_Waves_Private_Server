# SPEC: XDRIVE Cloudflare R2 极速性能调优与中俄伊全通增强规范

## 1. 目标与背景
Cloudflare R2 是目前跨越**中国（GFW）、伊朗（网络限断）、俄罗斯（RKN 审查）**三地审查环境的「最大公约数」：
1. **网络可达性与抗封锁**：S3 API 接入端点 `*.r2.cloudflarestorage.com` 部署在 Cloudflare 全球 Anycast 边缘网络，TLS 1.3 握手直连畅通，且免出站流量费（Zero Egress Fees）；
2. **当前性能瓶颈**：原配置中未显式设置并发度与快速轮询参数（导致走 Xray-core 默认的 8 并发、500ms 最大轮询、20ms 写入间隔），首字节响应慢（TTFB 较高），且国内 DNS 解析可能绕路美西；
3. **本规范目标**：
   - **参数级极限提速**：全面激进化传输参数（256KB 分片、64 并发、10ms 轮询、120ms 上限轮询、10ms 刷新），大幅削减首字节与打字交互延迟；
   - **优选 IP 与自定义域名支持**：解耦 TCP 连接目标（Address）与 TLS 握手 SNI，支持用户填入就近低延迟 Cloudflare Anycast 节点（如香港、东京、新加坡等），避开 DNS 劣化；
   - **双端协同落地**：在服务端 `rust/aegis` 升级构造器与引导，在客户端 `aegis_v2rayN` 注入优化预设与参数。

---

## 2. 核心参数调优设计

### 2.1 参数对比矩阵
| 调优维度 | 旧版默认 / 未配置 | 新版 R2 激进优化配置 | 调优效果与理论依据 |
| :--- | :--- | :--- | :--- |
| **`segment_bytes` (分片大小)** | 1MB (`1048576`) | **256KB (`262144`)** | 降低首包缓冲等待时间；对于 SSH 交互或小页面，首字节返回快 75% |
| **`concurrency` (并发读写槽)** | 未设置 (内核默认 8) | **64** | R2 具备极高边缘弹性，64 并发有效占满带宽，消灭传输阻塞 |
| **`poll_interval_ms` (最小轮询)** | 25ms | **10ms** | 减少收到新数据前的等待时延 |
| **`max_poll_interval_ms` (最大轮询)**| 800ms | **120ms** | 防止空闲后长轮询进入深度休眠，唤醒延迟从近 1 秒压缩至百毫秒内 |
| **`flush_interval_ms` (刷新间隔)** | 未设置 (内核默认 20ms) | **10ms** | 写入缓冲区更快刷出到云端，减少发送端排队延时 |
| **`eager_window_ms` (积极轮询窗口)**| 未设置 (内核默认 2000ms)| **1000ms** | 有流量时保持 1 秒高频轮询，兼顾性能与请求数平衡 |

### 2.2 构造器签名变更 (`rust/aegis/src/core/xray/xdrive.rs`)
```rust
impl XdriveConfig {
    /// 升级版 Cloudflare R2 极速构造器（默认注入 256KB 分片与激进流控参数）
    pub fn new_cloudflare_r2(
        account_id: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        folder: &str,
    ) -> Self {
        Self::new_cloudflare_r2_with_endpoint(
            &format!("https://{account_id}.r2.cloudflarestorage.com"),
            bucket,
            access_key_id,
            secret_access_key,
            folder,
        )
    }

    /// 支持自定义域名或预设 Endpoint 的 R2 构造器
    pub fn new_cloudflare_r2_with_endpoint(
        endpoint: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        folder: &str,
    ) -> Self {
        Self {
            service: XdriveService::S3,
            remote_folder: folder.to_string(),
            secrets: vec![access_key_id.to_string(), secret_access_key.to_string()],
            s3: Some(S3Config {
                endpoint: endpoint.trim_end_matches('/').to_string(),
                region: "auto".to_string(),
                bucket: bucket.to_string(),
                path_style: true,
            }),
            segment_bytes: Some(256 * 1024), // 256KB 激进极速首包
            poll_interval_ms: Some(10),      // 10ms 极低轮询
            max_poll_interval_ms: Some(120), // 120ms 最大轮询上限
            flush_interval_ms: Some(10),     // 10ms 快速刷出
            eager_window_ms: Some(1000),     // 1s 积极轮询窗口
            concurrency: Some(64),           // 64 满血边缘并发
        }
    }
}
```

---

## 3. 优选 Anycast IP 与自建域名兼容性

### 3.1 客户端 Outbound 生成规则 (`aegis_v2rayN`)
1. **地址解耦**：
   - `Address`: 允许用户填入 Cloudflare 优质直连 IP（例如 `104.16.x.x` 或香港/东京/首尔 Anycast 节点）；若未填则默认为 S3 Endpoint 的域名；
   - `Port`: 固定为 `443`；
   - `StreamSecurity`: `tls`；
   - `Sni`: 保持与 R2 域名或自定义域名完全一致；
   - `extra`: 自动注入激进参数 JSON：
     ```json
     {
       "concurrency": 64,
       "flushIntervalMs": 10,
       "maxPollIntervalMs": 120,
       "segmentBytes": 262144,
       "eagerWindowMs": 1000
     }
     ```

### 3.2 自定义域名支持 (Custom Domain)
当用户在 Cloudflare 后台为 R2 Bucket 绑定自建域名（如 `r2.mycompany.com`）时：
- 向导与解析器直接识别该域名作为 Endpoint；
- 避免直接暴露 `*.r2.cloudflarestorage.com`，强化抗审查特征隐匿性。

---

## 4. 交互引导与 i18n 优化 (`rust/aegis`)

### 4.1 提示文案更新 (`zh.yml`, `en.yml`, `ja.yml`)
在向导第一步增加提示：
- **中文 (`zh.yml`)**：
  `"💡 提示：输入 Cloudflare Account ID（32 位十六进制），或直接粘贴 R2 自定义域名（如 r2.example.com）。也可直接粘贴控制台整页凭据完成一键识别。"`
- **英文 (`en.yml`)**：
  `"💡 Tip: Enter your Cloudflare Account ID (32 hex chars), or paste your R2 custom domain (e.g. r2.example.com), or paste full credentials page for auto-detection."`
- **日文 (`ja.yml`)**：
  `"💡 ヒント: Cloudflare Account ID (32桁) または R2 カスタムドメイン (例: r2.example.com) を入力してください。認証情報ページの全体貼り付けによる自動認識にも対応しています。"`

---

## 5. 客户端 `aegis_v2rayN` 联动

1. **`AddServerViewModel.cs`**：
   - 切换到 `Cloudflare R2` 预设时，自动填充：
     - `XdriveService = "s3"`
     - `XdriveS3Region = "auto"`
     - `XdriveS3PathStyle = true`
     - `XdriveExtra` 注入 `{"concurrency":64,"flushIntervalMs":10,"maxPollIntervalMs":120,"segmentBytes":262144,"eagerWindowMs":1000}`。
2. **`V2rayOutboundService.cs`**：
   - 确保生成的 `xdriveSettings` 包含上述优化参数；
   - 确保 `mux: { enabled: true, concurrency: 128 }` 继续强制生效。

---

## 6. 验证与测试策略

1. **Rust 服务端测试**：
   - `test_cloudflare_r2_optimized_parameters`：断言 `new_cloudflare_r2` 输出的 `segment_bytes == 262144`，`concurrency == Some(64)`，`poll_interval_ms == Some(10)`，`max_poll_interval_ms == Some(120)`，`flush_interval_ms == Some(10)`；
   - `test_cloudflare_r2_custom_endpoint`：测试使用自定义域名生成配置；
   - `cargo nextest run` 全量通过；
   - `cargo clippy` 与 `cargo fmt` 零警告。
2. **C# 客户端测试**：
   - `AddServerViewModelTests.cs`：断言选择 `Cloudflare R2` 后自动注入 64 并发与 256KB 分片等优化 Extra；
   - `dotnet test` 全量通过。
