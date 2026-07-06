# Subscription Server — 设计文档

> 基于对 18 个开源项目的分析（Xray-core 生态 9 个 Web Panel + 9 个 One-Click 脚本），
> 结合 Aegis 现有架构定制的订阅链接方案。
>
> 日期: 2026-07-06

## 1. 动机与目标

### 现状

Aegis 目前通过 Telegram/Matrix Bot 生成并投递原始 client link（`vless://`, `hysteria2://`, `tuic://`），
这是 One-Click 脚本级别的交付方式。缺少：

- HTTP 订阅端点（自动更新配置）
- 多格式输出（Clash YAML / Sing-box JSON / Base64 URI list）
- 统一订阅管理（创建/吊销 token、关联配置、查看历史）

### 目标

为 Aegis 增加标准 HTTP 订阅服务，同时保持：

- Aegis 为主程序，Rust 生态不变
- Go 构建轻量订阅服务（独立二进制，可选组件）
- gRPC 分界通信，安全边界清晰
- 支持有域名 / 无域名两种场景
- 订阅 token 与 admin 解耦，独立管理

## 2. 架构

```
┌──────────────────────────────────────────────────┐
│  客户端 (v2rayNG / Clash / Sing-box / Nekoray)    │
│  GET /sub/:token  ← User-Agent 自动格式检测        │
└────────────────────┬─────────────────────────────┘
                     │ HTTPS
┌────────────────────▼─────────────────────────────┐
│  Go sub-server                                    │
│  - HTTP 服务器 (chi router)                        │
│  - TLS: acme.sh / rcgen 证书                      │
│  - 格式转换: Clash YAML / Sing-box JSON / Base64  │
│  - 速率限制 per-token                             │
│  - LRU 缓存 (默认 60s TTL)                        │
│  监听: 0.0.0.0:8443 (独立模式)                     │
│        127.0.0.1:8080 (反向代理模式)               │
└───────────────┬──────────────────────────────────┘
                │ gRPC over Unix Socket
┌───────────────▼──────────────────────────────────┐
│  Rust Aegis                                       │
│  - tonic gRPC 服务端                               │
│  - Token CRUD (SQLite)                            │
│  - 配置聚合 (Xray + SingBox config → ProxyConfig)  │
│  - Bot 交互问答 → 下载/验证/部署 Go sub-server      │
│  - minisign-verify 签名验证                        │
└──────────────────────────────────────────────────┘
```

### 设计原则

| 原则 | 说明 |
|------|------|
| **Rust 主导** | 部署、验证、管理均由 Aegis (Rust) 控制 |
| **Go 专注服务** | Go 只负责 HTTP 处理、格式转换、TLS 终止 |
| **gRPC 分界** | Unix Socket，不暴露网络，安全边界清晰 |
| **可选组件** | sub-server 不强制安装，是可选的增强功能 |
| **Token 独立** | Token 是独立实体，不与 admin 身份绑定 |

## 3. 组件详情

### 3.1 Proto 定义

`proto/subscription.proto`

```protobuf
syntax = "proto3";
package subscription;
option go_package = "tools/sub-server/proto/sub";

service SubscriptionService {
  rpc GetConfigs(GetConfigsRequest) returns (GetConfigsResponse);
  rpc GetTokenInfo(GetTokenInfoRequest) returns (TokenInfo);
}

service TokenService {
  rpc CreateToken(CreateTokenRequest) returns (TokenResponse);
  rpc ListTokens(ListTokensRequest) returns (ListTokensResponse);
  rpc RevokeToken(RevokeTokenRequest) returns (RevokeTokenResponse);
  rpc UpdateTokenConfigs(UpdateTokenConfigsRequest) returns (TokenResponse);
}

message SubscriptionToken {
  string token = 1;
  string label = 2;
  repeated string config_ids = 3;
  int64 created_at = 4;
  int64 expires_at = 5;
  bool revoked = 6;
}

message ProxyConfig {
  string config_id = 1;
  string protocol = 2;          // vless, hysteria2, tuic
  string host = 3;
  uint32 port = 4;
  string password = 5;
  string uuid = 6;
  string sni = 7;
  string pin_sha256 = 8;
  string public_key = 9;
  string short_id = 10;
  string transport = 11;       // tcp, xhttp, ws, grpc
  string path = 12;
  string flow = 13;
  string tag = 14;
  string obfs_type = 15;
  string obfs_password = 16;
  uint32 hop_port_start = 17;
  uint32 hop_port_end = 18;
  string alpn = 19;
  string congestion_control = 20;
  string cert_sha256 = 21;
}
```

### 3.2 Rust Aegis — gRPC 服务端

**新增模块**: `rust/aegis/src/core/subscription/`

| 文件 | 职责 |
|------|------|
| `mod.rs` | 模块入口，导出 `SubServerManager` |
| `server.rs` | tonic gRPC 服务端，监听 `/var/run/aegis/sub.sock` (Unix Socket) |
| `token.rs` | Token SQLite CRUD (`subscription_tokens` 表) |
| `aggregator.rs` | 读取 Xray/SingBox config JSON → 聚合为 `Vec<ProxyConfig>` |
| `deploy.rs` | 下载 GitHub Release → minisign 验证 → 部署 Go binary |
| `minisign.rs` | 封装 `minisign-verify` crate，硬编码公钥 |
| `cert.rs` | TLS 证书管理：acme.sh 调用 / rcgen 自签 |
| `config.rs` | 生成 Go sub-server 配置文件 |

**Bot 交互**: `rust/aegis/src/adapters/telegram/handlers/subscription.rs`

多步 callback 对话:

```
菜单 "📡 设置订阅服务"
  → Q1: 有域名？ [有域名] [仅 IP]
  → Q2: 端口 (默认 8443)
  → Q3: 速率限制 (默认 10/min)
  → 确认摘要 → [确认安装]
  → 下载 → 验证 → 部署 → 输出订阅 URL
```

**路径常量** (`rust/aegis/src/core/paths.rs` 新增):

```rust
pub mod sub_server {
    pub const BIN: &str = "/usr/local/bin/sub-server";
    pub const DIR: &str = "/etc/wwps/sub-server";
    pub const CERTS_DIR: &str = "/etc/wwps/sub-server/certs";
    pub const TLS_CERT: &str = "/etc/wwps/sub-server/certs/fullchain.pem";
    pub const TLS_KEY: &str = "/etc/wwps/sub-server/certs/privkey.pem";
    pub const GRPC_SOCK: &str = "/var/run/aegis/sub.sock";
    pub const SERVICE: &str = "wwps-sub-server";
}
```

### 3.3 Go sub-server

**目录**: `tools/sub-server/`

| 文件 | 职责 |
|------|------|
| `main.go` | 入口，flag 解析 + 启动 HTTP |
| `config/config.go` | Config 结构体 + flag/env 加载 |
| `handler/subscription.go` | `GET /sub/:token` UA detect → format routing |
| `handler/qr.go` | `GET /sub/:token/qr` |
| `handler/page.go` | `GET /sub/:token` (浏览器 → HTML 页面) |
| `format/v2ray.go` | Base64 URI list |
| `format/clash.go` | Clash YAML (`text/template`) |
| `format/singbox.go` | Sing-box JSON |
| `format/uri.go` | 纯文本 URI |
| `grpc/client.go` | gRPC client → Rust Aegis |
| `cache/lru.go` | LRU 缓存 `token → []ProxyConfig` |
| `middleware/ratelimit.go` | per-token rate limiting (golang.org/x/time/rate) |

**启动命令**:

```bash
sub-server \
  --listen-addr=:8443 \
  --tls-cert=/etc/wwps/sub-server/certs/fullchain.pem \
  --tls-key=/etc/wwps/sub-server/certs/privkey.pem \
  --aegis-grpc=unix:///var/run/aegis/sub.sock \
  --rate-limit=10 \
  --cache-ttl=60
```

**订阅端点的 User-Agent 格式检测**:

| User-Agent | 输出格式 |
|---|---|
| `clash`, `stash`, `surge`, `clash-verge` | Clash YAML |
| `sing-box`, `hiddify`, `karing` | Sing-box JSON |
| `shadowrocket`, `v2rayng`, `v2rayn`, `nekoray` | Base64 URI list |
| `Mozilla/...` (浏览器) | HTML 页面 + QR |
| 其他 + `?format=clash` | 指定格式 |
| 其他 | 纯文本 URI list |

### 3.4 TLS 模式

| 模式 | 条件 | 证书方案 | 有效期 | 推荐 |
|------|------|---------|--------|------|
| **A: 域名** | 有域名 + 80 端口开放 | acme.sh domain | 90天 | ✅ 推荐 |
| **B: IP-LE** | 仅 IP + 80 端口开放 | acme.sh shortlived IP | 6天 | ✅ 推荐 |
| **C: IP-自签** | 仅 IP + 80 端口不可用 | rcgen 自签 | 365天 | ⚠️ 高风险警告 |
| **D: 反代模式** | Nginx/Caddy 前置 | 由反代处理 | — | ✅ 推荐多服务 |

**自签模式高风险警告文案**（Bot deploy 输出 + token info 页面）：

```
⚠️ 高风险: 当前使用自签证书
您正使用自签证书运行订阅服务器。大多数客户端无法自动验证自签证书。
- v2rayNG/NekoBox/Furious: 需开启 allowInsecure
- Shadowrocket/Stash: 需手动安装 CA 证书
- Sing-box/Hiddify: 需配置 tls.accept_insecure_cert
建议: 使用 Let's Encrypt IP 证书（需要开放端口 80）或配置域名证书。
```

## 4. Token 模型

Token 是独立的订阅实体，不与 admin 身份绑定。

### 存储 (SQLite)

```sql
CREATE TABLE subscription_tokens (
    token       TEXT PRIMARY KEY,       -- crypto/rand 32字符
    label       TEXT NOT NULL DEFAULT '',
    config_ids  TEXT NOT NULL DEFAULT '[]',  -- JSON: ["*"] 或 ["HY2-*","TUIC-*"]
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL DEFAULT 0,  -- 0 = 永不过期
    revoked     INTEGER NOT NULL DEFAULT 0
);
```

### Bot 命令

| 命令 | 功能 |
|------|------|
| `/sub create [label]` | 创建新 token，自动附加所有现有配置 |
| `/sub list` | 列出所有 token（token 前 4 位 `abc1****` + 状态） |
| `/sub revoke <token>` | 吊销 token |
| `/sub info <token>` | 查看详情 + 订阅 URL |
| `/sub renew <token> <days>` | 续期 |
| `/sub setup` | 交互式部署 sub-server |

### 访问控制

- Token 作用域: 只能访问自己的 `ProxyConfig`
- 速率限制: 每 token 默认 10 req/min，burst 20
- 日志: 不记录完整 token，仅记录前 4 位

## 5. 部署流程

```
Admin → Bot "📡 设置订阅服务"
  │
  ├─ Q&A 交互（3 步问答）
  │
  ├─ 下载 sub-server (reqwest → GitHub API)
  │   └─ 下载 sub-server.minisig
  │
  ├─ 验证 minisign 签名
  │   └─ 失败 → 报错终止
  │
  ├─ 获取证书 (acme.sh / rcgen)
  │   └─ 失败 → 回退方案提示
  │
  ├─ 部署二进制 + systemd service
  │   ├─ /usr/local/bin/sub-server
  │   ├─ /etc/wwps/sub-server/certs/
  │   └─ /etc/systemd/system/wwps-sub-server.service
  │
  ├─ 开放防火墙端口
  │
  ├─ 启动服务
  │
  ├─ 创建默认 token
  │
  └─ Bot 输出:
      ✅ 订阅 URL: https://1.2.3.4:8443/sub/abc1...
      🌐 订阅页面: https://1.2.3.4:8443/sub/abc1...?info
```

## 6. 安全模型

| 层面 | 措施 |
|------|------|
| **传输层** | 独立模式: Go 自带 TLS；反代模式: 由 Nginx/Caddy 处理 |
| **Token** | `crypto/rand` 32 字符，熵 ≥ 192 bits |
| **速率限制** | per-token + global rate limiter |
| **gRPC 通信** | Unix Socket，权限 0600，仅 root 可访问 |
| **签名验证** | minisign 签名 Go 二进制，Rust `minisign-verify` 校验 |
| **日志安全** | token 仅记前 4 位，完整 token 不在日志中出现 |
| **二进制混淆** | Go 用 `garble -literals -tiny` 编译 |

## 7. CI/CD

`.build.yml` 新增:

```yaml
- build-sub-server: |
    cd tools/sub-server
    CGO_ENABLED=0 garble -literals -tiny -seed=random build \
      -ldflags="-s -w -X main.version=${NEW_VERSION}" \
      -o sub-server .
    minisign -S -s /path/to/secret.key -m sub-server \
      -t "${NEW_VERSION}:sub-server" -x sub-server.minisig
- prepare-dist: |
    cp tools/sub-server/sub-server dist/
    cp tools/sub-server/sub-server.minisig dist/
```

Release 产物:
- `sub-server` — Go 订阅服务器二进制
- `sub-server.minisig` — minisign 签名

## 8. 新依赖

### Rust (`Cargo.toml` 新增)

```toml
tonic = { version = "0.13", features = ["transport"] }
rcgen = { version = "0.14", features = ["pem", "x509-parser"] }
```

### Go (`tools/sub-server/go.mod`)

```
google.golang.org/grpc
google.golang.org/protobuf
golang.org/x/time/rate
github.com/go-chi/chi/v5
github.com/skip2/go-qrcode
aead.dev/minisign
```

### 已有（无需新增）

- `minisign-verify = "0.2.5"` ✅ 已在 Cargo.toml
- `prost = "0.14"` ✅ 已在 Cargo.toml
- `reqwest` ✅ 已在 Cargo.toml
- `zip` ✅ 已在 Cargo.toml
- `x509-parser` ✅ 已在 Cargo.toml
- `prost-build = "0.14"` ✅ 已在 build-dependencies

## 9. 实施计划

| # | 阶段 | 文件 |
|---|------|------|
| 1 | Proto 定义 | `proto/subscription.proto`, `rust/aegis/build.rs` |
| 2 | Rust gRPC 服务端 | `rust/aegis/src/core/subscription/{mod,server,token,aggregator,cert,config,deploy,minisign}.rs` |
| 3 | Bot 交互 | `rust/aegis/src/adapters/telegram/handlers/subscription.rs`, paths.rs |
| 4 | Go sub-server | `tools/sub-server/` 完整 Go 模块 |
| 5 | CI/CD | `.build.yml` 新增 sub-server 构建签名 |
| 6 | 集成测试 | Rust↔Go gRPC 端到端验证 |
