# 架构设计规范：Aegis XDRIVE 隐蔽传输总框架 (SPEC-xdrive)

## 1. 目标与设计原则 (Objectives & Principles)

### 1.1 背景与目标
在现有直连协议（Vision, XHTTP, KcpMask, Hy2, TUIC）之上，为 Aegis 引入基于对象存储的 **XDRIVE 传输协议支持**：
1. **白名单穿透与防审查**：实现无公网 IP 暴露、无公网端口监听、流量伪装为合法公有云 API 请求；
2. **抽象与分步交付**：搭建统一的 `XdriveConfig` 框架，首期重点打通 **S3 兼容后端（以 Cloudflare R2 / AWS S3 为主）** 与 **Local 本地回环**，预留 Google Drive 结构体与枚举，但**暂不接入 Google Drive** 的 OAuth2 / RefreshToken 交互；
3. **零特权零暴露**：在端口分配器和防火墙管理器中将 XDRIVE 视为 `ZeroPort` 虚拟入站，不向外部防火墙暴露任何入站规则。

### 1.2 非目标 (Non-Goals)
- 暂不实现 Google Drive API 鉴权流程与 Token 刷新后台服务；
- 暂不对 `/home/ub/Dark/Xray-core` 的 Go 代码进行二次修改（Xray-core 现有 `transport/internet/xdrive/` 的 24 个文件功能已完备）。

---

## 2. 模块能力与依赖图谱 (Capability Map)

```text
┌─────────────────────────────────────────────────────────────┐
│ 1. xdrive_model (rust/aegis/src/core/xray/xdrive.rs)        │
│    └─ 数据结构、S3 签名参数校验、Cloudflare R2 预设构建器    │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ 2. config_builder (rust/aegis/src/core/xray/config.rs)      │
│    ├─ 扩展 Proto::XDrive 枚举变体                            │
│    ├─ build_xdrive_vless_inbound (生成 Xray 合规 JSON)       │
│    └─ generate_client_link (格式化 vless:// 导出链接)       │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ 3. lifecycle_adapt (port_allocator.rs & firewall.rs)        │
│    └─ 端口分配与防火墙管理安全豁免 (零公网端口)              │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. 详细设计规范 (Detailed Specification)

### 3.1 数据模型规范 (`rust/aegis/src/core/xray/xdrive.rs`)
对照 Xray-core `config.proto` 与 `s3.go` 约束：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdriveService {
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "google-drive")]
    GoogleDrive, // 预留，暂不接通
    #[serde(rename = "template")]
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub path_style: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XdriveConfig {
    pub service: XdriveService,
    pub remote_folder: String,
    pub secrets: Vec<String>, // S3 模式: [AccessKeyID, SecretAccessKey]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3: Option<S3Config>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_bytes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_poll_interval_ms: Option<u32>,
}
```

### 3.2 入站 JSON 规范 (`rust/aegis/src/core/xray/config.rs`)
根据 Xray-core PR #7182 的强制约束：
```json
{
  "tag": "inbound-vless-xdrive",
  "listen": "127.0.0.1",
  "port": 0,
  "protocol": "vless",
  "settings": {
    "clients": [{
      "id": "{{CLIENT_UUID}}",
      "encryption": "{{VLESS_ENCRYPTION_KEY}}",
      "level": 0
    }],
    "decryption": "none"
  },
  "streamSettings": {
    "network": "xdrive",
    "xdriveSettings": {
      "service": "s3",
      "remoteFolder": "aegis-xdrive",
      "secrets": ["{{ACCESS_KEY}}", "{{SECRET_KEY}}"],
      "s3": {
        "endpoint": "https://<account-id>.r2.cloudflarestorage.com",
        "region": "auto",
        "bucket": "my-bucket",
        "pathStyle": true
      },
      "pollIntervalMs": 25
    }
  }
}
```
* **硬约束断言**：禁止指定 `flow`；客户端必须启用 `mux` 与 `encryption`。

### 3.3 客户端订阅链接规范
```text
vless://<UUID>@127.0.0.1:0?type=xdrive&service=s3&endpoint=https%3A%2F%2F<ID>.r2.cloudflarestorage.com&region=auto&bucket=<BUCKET>&pathStyle=true&remoteFolder=aegis-xdrive&encryption=<ENCRYPTION_KEY>#Aegis-XDRIVE-R2
```

### 3.4 端口与防火墙协同规范
* `PortAllocator`：识别 `Proto::XDrive` 时直接返回虚拟端口 `0`，不将其写入 `.port_alloc` 占用主机网络资源，不分配连续跳跃端口（`HOP_SIZE`）；
* `FirewallManager`：识别到端口为 `0` 或协议为 `XDrive` 时，跳过系统 UFW / Firewalld 的 `ALLOW` 命令执行，从源头杜绝不必要的安全暴露。
