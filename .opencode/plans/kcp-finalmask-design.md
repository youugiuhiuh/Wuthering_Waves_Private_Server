# KCP + FinalMask 支持改进设计

## 背景

官方 Xray-core 对 KCP (mKCP) 传输协议做了重大改动：

1. `kcpSettings.header` 和 `kcpSettings.seed` **已移除** — 改用 `finalmask.udp[]` 配置
2. `congestion`、`readBufferSize`、`writeBufferSize` **已移除** — 改用 `cwndMultiplier`、`maxSendingWindow`
3. REALITY **不支持** KCP — 仅支持 TCP/XHTTP/gRPC
4. 项目自定义 fork (wwps-core) 仍支持 KCP+Reality+xdns

**当前问题**：项目中的 XDNS mKCP 配置使用旧格式，需要更新；同时需要新增通用 KCP 批量创建功能。

## 设计决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 安全层 | TLS 或 none | 官方 Xray-core 不支持 KCP+Reality |
| FinalMask 类型 | 支持所有官方类型 + xdns | 完全遵循官方实现 |
| UI 方式 | 分组选择 | 加密组/伪装组/特殊组，清晰易用 |
| XDNS 兼容 | 保留现有 XDNS 功能，仅更新格式 | 向后兼容，wwps-core 仍支持 KCP+Reality |

## 一、类型定义（`core/types.rs`）

### 1.1 `KcpSecurity` 枚举

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KcpSecurity {
    Tls,
    None,
}
```

### 1.2 `KcpFinalMask` 枚举

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum KcpFinalMask {
    // 加密组
    MkcpOriginal,                           // mkcp-original (无密码，XOR)
    MkcpAes128Gcm { password: String },     // mkcp-aes128gcm (需要密码)

    // 伪装组
    HeaderDns { domain: String },            // header-dns (默认 domain: www.baidu.com)
    HeaderWechat,                            // header-wechat
    HeaderSrtp,                              // header-srtp
    HeaderUtp,                               // header-utp
    HeaderDtls,                              // header-dtls
    HeaderWireguard,                         // header-wireguard

    // 特殊组
    Xdns { domain: String },                 // xdns (项目自定义)
}
```

实现方法：
- `fn type_str(&self) -> &'static str` — 返回 finalmask type 字符串
- `fn group_name(&self) -> &'static str` — 返回分组名
- `fn as_json(&self) -> Value` — 生成 finalmask 单项 JSON
- `fn display_name(&self) -> &'static str` — UI 显示名

## 二、配置生成（`logic/config.rs`）

### 2.1 更新 `build_xdns_mkcp_inbound()`

**kcpSettings 从旧格式更新为：**

```json
{
  "mtu": 1350,
  "tti": 50,
  "uplinkCapacity": 5,
  "downlinkCapacity": 20,
  "cwndMultiplier": 1,
  "maxSendingWindow": 2097152
}
```

移除：`congestion`、`readBufferSize`、`writeBufferSize`。

### 2.2 新增 `build_kcp_inbound()`

```rust
fn build_kcp_inbound(
    tag: &str,
    port: i32,
    uuid: &str,
    email: &str,
    ip_version: IpVersion,
    security: KcpSecurity,
    finalmask: &KcpFinalMask,
    tls_sni: Option<&str>,
    tls_cert_path: Option<&str>,
    tls_key_path: Option<&str>,
) -> Value
```

**security = "none" 时：**
```json
{
  "listen": "<listen_ip>",
  "port": <port>,
  "protocol": "vless",
  "tag": "<tag>",
  "settings": { "clients": [{"id": "<uuid>", "email": "<email>"}], "decryption": "none" },
  "streamSettings": {
    "network": "kcp",
    "kcpSettings": { "mtu": 1350, "tti": 50, "uplinkCapacity": 5, "downlinkCapacity": 20, "cwndMultiplier": 1, "maxSendingWindow": 2097152 },
    "security": "none",
    "finalmask": { "udp": [<finalmask_item>] }
  },
  "sniffing": { "enabled": true, "destOverride": ["http", "tls", "quic"], "metadataOnly": false }
}
```

**security = "tls" 时：**
```json
{
  "streamSettings": {
    "network": "kcp",
    "kcpSettings": { ... },
    "security": "tls",
    "tlsSettings": {
      "certificates": [{"certificateFile": "<cert_path>", "keyFile": "<key_path>"}],
      "alpn": ["http/1.1"]
    },
    "finalmask": { "udp": [<finalmask_item>] }
  }
}
```

### 2.3 新增 `generate_kcp_client_link()`

| 安全层 | 链接格式 |
|--------|----------|
| TLS | `vless://UUID@host:port?encryption=none&security=tls&type=kcp&sni=SNI&fp=chrome&fm=ENCODED#email` |
| None | `vless://UUID@host:port?encryption=none&type=kcp&security=none&fm=ENCODED#email` |

`fm=` 参数为 finalmask JSON 的 URL 编码，与现有 `generate_xdns_client_link()` 一致。

### 2.4 新增 `batch_create_kcp()`

```rust
pub async fn batch_create_kcp(
    count: usize,
    standalone: bool,
    ip_version: IpVersion,
    security: KcpSecurity,
    finalmask: KcpFinalMask,
) -> Result<BatchCreationResult>
```

遵循现有批量创建模式：
- 随机分配端口（10000-60000）
- 生成 UUID
- 对于 TLS：需要 SNI 和证书路径
- 生成配置和链接
- 保存配置文件并重载核心

### 2.5 重命名 `RealityProto` 为 `Proto`

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proto {
    Vision,      // 原 RealityProto::Vision
    XHTTP,       // 原 RealityProto::XHTTP
    XdnsMkcp,    // 原 RealityProto::XdnsMkcp
    Kcp,         // 新增：通用 KCP
}
```

需要更新所有引用 `RealityProto` 的代码。

## 三、Telegram Bot UI（`main.rs`）

### 3.1 菜单入口

在 Xray-core 管理菜单新增按钮：
```
🚀 KCP 批量配置  → u_kcp_init
```

### 3.2 KCP 创建流程

```
步骤 1: u_kcp_init → 选择安全层
  [🔒 TLS 加密] [🔓 无加密] [⬅️ 返回]

步骤 2: u_kcp_sec:t 或 u_kcp_sec:n → 选择 FinalMask 分组
  [🔐 加密层] [🎭 伪装层] [🌟 特殊层] [⬅️ 返回]

步骤 3: 选择具体 FinalMask
  加密组: [🔄 mkcp-original] [🔐 mkcp-aes128gcm] [⬅️ 返回]
  伪装组: [📡 srtp] [📥 utp] [🔒 dtls] [💬 wechat] [📶 wireguard] [🌐 dns] [⬅️ 返回]
  特殊组: [🌐 xdns] [⬅️ 返回]

步骤 4: 选择 IP 版本
  [🌐 IPv4 (0.0.0.0)] [🌐 IPv6 (::)]

步骤 5: 选择数量
  [1] [3] [5] [10] [20] [50] [⬅️ 返回]

步骤 6: 执行批量创建
```

参数自动处理：
- `mkcp-aes128gcm`：自动生成随机密码
- `header-dns`：使用默认域名 www.baidu.com
- `xdns`：从 SNI 选择器自动选择域名
- 其他：无额外参数

### 3.3 回调数据编码

| 编码 | 含义 |
|------|------|
| `sec` | 安全层: `t`=TLS, `n`=none |
| `fm` | FinalMask: `mo`=mkcp-original, `ma`=mkcp-aes128gcm, `sr`=header-srtp, `ut`=header-utp, `dt`=header-dtls, `wc`=header-wechat, `wg`=header-wireguard, `hd`=header-dns, `xd`=xdns |

回调数据示例：
- `u_kcp_sec:t`
- `u_kcp_grp:t:enc`
- `u_kcp_fm:t:ma`
- `u_kcp_ip:t:ma:4`
- `u_kcp_ex:t:ma:4:3`

## 四、TLS 证书处理

对于 `security = "tls"` 模式：
- 前置检查证书是否可用
- 证书路径从环境变量或配置文件读取
- 不可用时给出明确错误提示

## 五、kcpSettings 默认值

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `mtu` | 1350 | 最大传输单元 |
| `tti` | 50 | 传输间隔 (ms) |
| `uplinkCapacity` | 5 | 上行容量 (Mbps) |
| `downlinkCapacity` | 20 | 下行容量 (Mbps) |
| `cwndMultiplier` | 1 | 拥塞窗口乘数 |
| `maxSendingWindow` | 2097152 | 最大发送窗口 (2MB) |

XDNS mKCP 使用低 MTU (130) 和 TTI (20) 是因为 DNS 伪装场景的特殊需求，保持不变。

## 六、文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `rust/tgbot/src/core/types.rs` | 修改 | 新增 `KcpSecurity`、`KcpFinalMask` 枚举及实现 |
| `rust/tgbot/src/logic/config.rs` | 修改 | 新增 KCP 配置生成函数；更新 XDNS kcpSettings 格式；重命名 `RealityProto` 为 `Proto` |
| `rust/tgbot/src/main.rs` | 修改 | 新增 KCP 菜单入口和回调处理流程 |

## 七、不影响现有功能

- XDNS (mKCP+Reality+xdns) 功能保留，仅更新 kcpSettings 格式
- Reality Vision 和 XHTTP 功能完全不变
- 新增的 KCP 功能是独立的新菜单入口