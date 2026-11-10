# Spec: XDRIVE 微软云存储（OneDrive / SharePoint / Microsoft Graph）深度技术研究与架构设计

## 1. 目标与背景 (Objective & Background)

### 1.1 背景
XDRIVE 是基于底层流式传输协议（类似 Xray-core transport: xdrive）的去中心化中转通道。目前系统已成功落地 **Cloudflare R2 (S3)** 与 **Google Drive (OAuth2 网盘)**。为了进一步丰富抗封锁通道矩阵，需要引入在全球范围内拥有极高网络权重、广泛免费/教育配额（如 Office 365 E5 开发者订阅 5TB、个人版 1TB、SharePoint 团队站点大容量存储）以及顶级 CDN 边缘加速的**微软云存储体系（Microsoft Graph API）**。

### 1.2 目标
- 第一期全量实装**微软国际版 (Global) OneDrive / Microsoft 365**，世纪互联中国特供版标记为二期暂不实现。
- 制定稳定高效的 OAuth2 授权、Token 刷新与多租户适配模型。
- 设计针对 XDRIVE 协议的流式分片上传/下载与长轮询调度算法。
- 给出 Aegis（Rust 后端与 C# 客户端）中数据模型定义、客户端连接串生成规范及性能预设参数。

---

## 2. 微软云存储生态全景与端点差异 (Ecosystem & Endpoints)

| 存储类型 | 目标用户与场景 | 根 API 路径 / 资源标识 | 国际版端点 (Global) | 世纪互联端点 (China 21Vianet) |
| :--- | :--- | :--- | :--- | :--- |
| **OneDrive 个人版** | 免费 5GB / 订阅 1TB / 家庭组 | `/me/drive/root` | `graph.microsoft.com` | 不适用（个人版无世纪互联） |
| **OneDrive 商业版 / E5** | E5 开发者 5TB / 企业员工 | `/me/drive/root` 或 `/users/{id\|upn}/drive/root` | `graph.microsoft.com` | `microsoftgraph.chinacloudapi.cn` |
| **SharePoint 文档库** | 团队协作站点 / 机构超大容量 | `/sites/{site-id}/drives/{drive-id}/root` | `graph.microsoft.com` | `microsoftgraph.chinacloudapi.cn` |

### 核心差异解析：
1. **租户端点隔离 (Tenant Resolution):**
   - 国际版 OAuth 授权端点：`https://login.microsoftonline.com/{tenant}/oauth2/v2.0/`
     - 个人版常用 `{tenant} = consumers`
     - 企业/E5/多租户应用常用 `{tenant} = common` 或 `{tenant} = organizations`
   - 世纪互联授权端点：`https://login.chinacloudapi.cn/{tenant}/oauth2/v2.0/`
     - **注：世纪互联不支持个人 MSA 账户**，`{tenant}` 严禁填 `consumers`，需为 `organizations` 或具体的租户 ID。
2. **Drive ID 与 Item 定位机制:**
   - 个人版与商业版个人盘均可简写使用 `/me/drive/root:/{remote_folder}:` 进行路径索引。
   - SharePoint 需先通过站点域名获取 `site-id`（`GET /sites/{host}:/sites/{sitename}`），再枚举 `drives` 获得 `drive-id`。

---

## 3. OAuth2 鉴权与生命周期管理 (Authentication & Lifecycle)

### 3.1 权限作用域 (Scopes)
微软采用更细粒度的 Graph 权限体系：
- 个人 / 商业 OneDrive 读写权限：`Files.ReadWrite`（委托权限，用于操作当前登录用户的整个驱动器）。
- 离线长久访问：`offline_access`（必需，用于换取 `refresh_token`，默认有效期 90 天，每次刷新滑动续期）。
- SharePoint 扩展：若需访问非当前用户的共享文档库，需追加 `Sites.ReadWrite.All`。
- 最终授权 Scope 组合：`offline_access Files.ReadWrite`（若含 SharePoint：`offline_access Files.ReadWrite Sites.ReadWrite.All`）。

### 3.2 授权流设计（面向 Aegis 交互式向导）
与 Google Drive 类似，Aegis 采用 **OAuth 2.0 Authorization Code with PKCE** 或带 Client Secret 的授权码流：
1. **应用注册平台类型（微软官方安全关键约束）：**
   - **绝对不可选择“单页应用 (Single-page application - SPA)”**：根据微软官方 Entra ID 规定，SPA 类型的 Refresh Token 强制锁死在 **24 小时生命周期**，即使声明 `offline_access` 也会每天失效。
   - **必须选择“移动和桌面应用 (Mobile and desktop applications)”** 或 **“Web”**：此时 Refresh Token 具备默认 90 天滑动非活动窗口（只要每 90 天内有使用，即可永久轮换刷新续期）。
   - 支持多租户账户（"Accounts in any organizational directory and personal Microsoft accounts"）。
   - 重定向 URI (Redirect URI): `https://login.microsoftonline.com/common/oauth2/nativeclient` 或 `http://localhost`。
2. **生成用户授权 URL:**
   ```
   https://login.microsoftonline.com/common/oauth2/v2.0/authorize?
     client_id={client_id}
     &response_type=code
     &redirect_uri=https://login.microsoftonline.com/common/oauth2/nativeclient
     &response_mode=query
     &scope=offline_access%20Files.ReadWrite
     &state={state}
   ```
3. **换取 Token (POST `https://login.microsoftonline.com/common/oauth2/v2.0/token`):**
   - 请求体包含 `code`, `client_id`, `client_secret` (若是 Web 应用), `redirect_uri`, `grant_type=authorization_code`。
   - 响应包含：`access_token` (通常 60~90 分钟有效) 和 `refresh_token`。
4. **Token 自动续期 (Token Refresh):**
   - 定期或在遇到 401 Unauthorized 时，使用 `refresh_token` 换取新的 `access_token`。

---

## 4. Microsoft Graph API 读写机制与流式分片 (I/O & Chunking)

XDRIVE 核心是基于 HTTP 请求将网络数据包切片为小块文件并存入远程目录，另一端通过轮询拉取并重组流。

### 4.1 文件上传策略
1. **简单上传 (Simple Upload - ≤4MB):**
   - `PUT /me/drive/root:/{remote_folder}/{segment_name}:/content`
   - 请求体直接携带二进制 Payload。
   - 延迟极低，握手次数少（1 次 RTT），非常契合 XDRIVE 小分片（256KB~1MB）的设计哲学。
2. **可恢复大文件上传 (Upload Session - >4MB):**
   - `POST /me/drive/root:/{remote_folder}/{segment_name}:/createUploadSession`
   - 返回 `uploadUrl`，后续通过分块 `PUT` 上传（块大小必须是 320 KiB 的整数倍）。
   - **XDRIVE 选型结论**：XDRIVE 的 `segment_bytes` 设定在 256KB ~ 1MB 范围内，**应强制走 Simple Upload**，消除创建 UploadSession 的额外往返延迟。

### 4.2 文件下载与流式拉取
- **元数据与下载链接获取：**
  - `GET /me/drive/root:/{remote_folder}/{segment_name}?select=id,@microsoft.graph.downloadUrl`
  - 或直接请求内容：`GET /me/drive/root:/{remote_folder}/{segment_name}:/content`
  - 直接请求 `/content` 会返回 `302 Found` 重定向至底层 Azure Blob/SharePoint CDN 直连临时链接（免去中间代理瓶颈）。
- **下载后清理 (Delete):**
  - 数据消费完毕后调用 `DELETE /me/drive/root:/{remote_folder}/{segment_name}` 或通过批量请求 (Batch API) 批量删除已消费分片。

### 4.3 目录长轮询与增量变更 (Delta API vs List Children)
- `GET /me/drive/root:/{remote_folder}/children?$select=id,name,size`：用于拉取当前目录待消费的分片列表。
- `GET /me/drive/root:/{remote_folder}/delta`：支持返回 `deltaLink`，仅获取新增文件，能极大减少轮询开销。

---

## 5. 性能调优、限流与弹性设计 (Throttling & Resilience)

### 5.1 429 Too Many Requests 防御与微软官方规约
根据微软官方最新《Avoid getting throttled or blocked in SharePoint / OneDrive Online》指南：
- **流量装饰规范 (Traffic Decoration - 强约束)：** 微软明确警告，未携带 AppID 和规范 User-Agent 的裸流量会被视作 "undecorated traffic"，在服务器高负载时会首先遭到 429 降级或直接封阻。
  - Aegis 请求必须注入规范 User-Agent：`NONISV|AegisProject|XdriveClient/1.0`。
- **并发控制（极其敏感）：** 微软基于令牌桶算法累加 API 成本，**严禁大并发轰炸**（Rclone 用户在 OneDrive 上开高并发会遭遇严重的 `activityLimitReached` 报错）。
  - 写操作严格限制并发度（建议单线程排队或最多 4~8 并发），切不可套用 S3/R2 的 64 高并发。
- **必须解析响应头 `Retry-After`**：以秒为单位指示等待时长。
- **指数退避重试 (Exponential Backoff with Jitter)**：基础等待时间 100ms，上限 3000ms，附加随机抖动，避免多个并发连接同时重试引发惊群效应。

### 5.2 XDRIVE 参数预设 (Optimization Preset)
针对 Microsoft Graph 的 API 延迟（通常在 100ms~300ms 左右，略高于 Cloudflare R2，类似 Google Drive）：
- `segment_bytes`: `524288` (512KB) — 兼顾单次 Simple Upload 吞吐与内存占用。
- `poll_interval_ms`: `50` (50ms) — 避免过快空转拉低令牌桶配额。
- `max_poll_interval_ms`: `300` (300ms) — 规避空闲时的 429 限流风暴。
- `flush_interval_ms`: `10` (10ms) — 快速刷出首包。
- `eager_window_ms`: `10000` (10s) — 高并发传输期保持高频拉取。
- `concurrency`: `8` (默认建议 4~8，最大不宜超过 16；严防触发租户级限流)。

---

## 6. 伪装与抗封锁特性 (Domain Fronting & Censorship Circumvention)

### 6.1 域前置与 TLS SNI
- **首选 SNI / Host:** `graph.microsoft.com`
- **备用高信誉入口:** `login.microsoftonline.com`、`www.office.com`、`onedrive.live.com`
- **世纪互联特供:** `microsoftgraph.chinacloudapi.cn`
- **ALPN 协商:** 必须启用 `h2` (HTTP/2) + `http/1.1`，Graph API 强依赖 HTTP/2 多路复用。
- **TLS 伪装:** 模拟现代浏览器指纹（uTLS: `chrome` 或 `edge`），完全融入正常的 Office 365 办公协同流量。

---

## 7. Aegis / XDRIVE 架构与数据模型设计 (Architecture & Data Contracts)

### 7.1 服务端枚举扩展 (`XdriveService`)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdriveService {
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "google-drive")]
    GoogleDrive,
    #[serde(rename = "onedrive")]
    OneDrive,
    #[serde(rename = "sharepoint")]
    SharePoint,
    #[serde(rename = "template")]
    Template,
}
```

### 7.2 Secrets 与配置组织规范
对于 `XdriveService::OneDrive`:
- `secrets` 数组结构设计：
  - `secrets[0]`: `client_id` (App ID)
  - `secrets[1]`: `client_secret` (若无 secret，可为留空或特定占位符)
  - `secrets[2]`: `refresh_token`
  - `secrets[3]` (可选): `tenant_id` (默认 `common`，若为世纪互联或特定企业租户则传入)
- `remote_folder`: 存储分片的目录路径（如 `/xray_tunnel`）。

对于 `XdriveService::SharePoint`:
- `secrets` 额外承载 `site_id` 与 `drive_id`，或在 `remote_folder` 中遵循标准 URI 定位格式。

### 7.3 客户端订阅链接规范 (VLESS URI)
针对 OneDrive 生成的节点连接串示例：
```
vless://{uuid}@graph.microsoft.com:443?type=xdrive&service=onedrive&remoteFolder=%2Fxray_tunnel&encryption={encryption_key}&security=tls&sni=graph.microsoft.com&alpn=h2&mux=true&secrets={encoded_secrets}#{node_name}
```

---

## 8. 命令规范 (Commands)

- **构建与语法检查：** `cargo check --workspace`
- **格式化检查：** `cargo fmt --check`
- **静态代码审查：** `cargo clippy --all-targets -- -D warnings`
- **单元与集成测试：** `cargo nextest run --cargo-profile fast-test`
- **文档测试：** `cargo test --doc`

---

## 9. 质量与边界约束 (Boundaries)

### Always:
- 保证每个对外构造器（如 `new_onedrive`）均具有独立单测与参数边界测试。
- 保证国际版与世纪互联在 Endpoint/Tenant 上能清晰解耦。
- 保证错误日志脱敏，严禁输出 `client_secret` 或 `refresh_token` 原文。

### Ask first:
- 在 Xray-core transport 层引入 CGO 或外部原生依赖。
- 修改全局配置存储版本或引入向后不兼容的 URI 字段改动。

### Never:
- 绕过 429 重试机制进行无限重试造成租户封禁。
- 在未通过 `rust-lint-format` 门禁的情况下提交代码。

---

## 10. 验收标准与成功指标 (Success Criteria)

1. **协议规范完备性：** 数据结构清晰支持 `onedrive` 与 `sharepoint`，包含完备的参数调优预设。
2. **单测覆盖率：** 构造器生成、参数合法性检验（缺少 Secret、非法路径、跨越目录）、客户端 VLESS 链接生成测试 100% 通过。
3. **向导与多语言体验：** Telegram 机器人提供保姆级图文引导（从 Azure 门户创建应用、获取 Client ID、一键授权到填入 Refresh Token）。
4. **性能基线：** 在标准 E5 开发者租户下，单分片传输首包时间 ≤ 180ms，支持 8 并发稳定流转无 429 报错。

---

## 11. 待确认问题 (Open Questions)

1. **关于 Client Secret 的必要性：** Azure AD 允许将应用注册为“公共客户端 (Public client - mobile and desktop)”，此时无需 `client_secret` 即可凭借 PKCE 完成刷新；是否在向导中优先推荐免 Secret 的公共客户端流程以降低用户配置门槛？
2. **是否将 SharePoint 与 OneDrive 合并为同一个驱动器服务项：** 两者底层皆为 Microsoft Graph Drive API，仅入口资源 ID 解析方式有别，是否统一抽象为 `onedrive` 并通过参数智能识别？
