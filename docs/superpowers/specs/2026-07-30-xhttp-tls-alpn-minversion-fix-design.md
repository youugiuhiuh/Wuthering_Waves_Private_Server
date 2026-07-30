# xhttp+TLS: 补齐 alpn 与 minVersion 字段

## 背景

xhttp+TLS 模式下生成的服务器入站配置和客户端分享链接缺少关键 TLS 字段，导致 CDN 回源时 ALPN 协商异常，不符合 Xray-core 社区标准。

## 问题

- **服务端** `build_tls_xhttp_inbound` 生成的 `tlsSettings` 只包含 `serverName` + `certificates`，缺少 `alpn` 和 `minVersion`
- **客户端链接** `generate_client_link_tls` 生成的 URL 缺少 `&alpn=h2` 参数

## 修改方案

### 1. `build_tls_xhttp_inbound` — 服务端入站 TLS 配置

在 `tlsSettings` 中追加：

```json
"alpn": ["h2", "http/1.1"],
"minVersion": "1.2"
```

### 2. `generate_client_link_tls` — 客户端分享链接

在 URL query 参数中追加 `&alpn=h2`

### 文件范围

仅 `rust/aegis/src/core/xray/config.rs` 一个文件，2 处修改。

### 变更量

~5 行新增，0 行删除。

## 参考

- Xray-core #4118 xhttp 五合一配置规范
- Xray-core #716 分享链接标准提案 4.4.2 节 (alpn)
