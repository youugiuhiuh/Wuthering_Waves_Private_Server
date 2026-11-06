# SPEC: XDRIVE 按服务商/存储平台重构与 Google Drive 深度集成

## 1. 背景与目标
原先 XDRIVE 以地区代码（`🇨🇳 CN`, `🇷🇺 RU`, `🇮🇷 IR`）分类存在以下不足：
1. **概念错位**：Cloudflare R2 和 Google Drive 是全球通用服务，强行归类为特定国家导致认知偏差；
2. **缺乏 Google Drive**：Xray-core 原生强推的 Google Drive 网盘中转（免公网端口 + `google.com` 域前置 + 零元购个人云存储）尚未向用户开放。

本规范将 XDRIVE 全面重构为**「按云存储服务商/平台分类」**体系，并依据《XDRIVE with Google Drive setup guide》正式接入 Google Drive 网盘中转。

---

## 2. 菜单架构设计

### 2.1 主菜单选项（`u_xdrive_menu`）
- ☁️ **Cloudflare R2**（免备案 / 零出网流量费 / 每月 10G 免费）-> `u_xdrive_p:r2`
- 📁 **Google Drive**（谷歌云盘 / 免翻墙域前置 / 个人项目无限免费）-> `u_xdrive_p:gdrive`
- 🇷🇺 **Yandex Cloud S3**（俄罗斯本土 / 白名单穿透）-> `u_xdrive_p:yandex`
- 🇮🇷 **ArvanCloud S3**（伊朗本土 / 白名单穿透）-> `u_xdrive_p:arvan`
- ⚙️ **自定义 S3 兼容存储**（AWS / MinIO / 阿里云 / 腾讯云等）-> `u_xdrive_p:s3`
- `🔙 返回批量菜单`

兼容性：原回调 `u_xdrive_r:*` 映射重定向至对应服务商。

---

## 3. Google Drive 集成设计（严格依据 DOCX 官方指南）

### 3.1 核心数据结构与预设（`xdrive.rs`）
```rust
impl XdriveConfig {
    pub fn new_google_drive(
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
        folder_id: &str,
    ) -> Self {
        Self {
            service: XdriveService::GoogleDrive,
            remote_folder: folder_id.to_string(),
            secrets: vec![
                client_id.to_string(),
                client_secret.to_string(),
                refresh_token.to_string(),
            ],
            s3: None,
            segment_bytes: Some(524288),   // 512KB
            poll_interval_ms: Some(50),    // 50ms
            max_poll_interval_ms: Some(200),
            flush_interval_ms: Some(10),
            eager_window_ms: Some(10000),
            concurrency: Some(64),
        }
    }
}
```

### 3.2 节点构建与客户端链接导出（`config.rs`）
- **服务端 Inbound**：
  - `listen`: `127.0.0.1`，分配专属本地端口；
  - `settings.decryption`: `mlkem768x25519plus.native.600s...`；
  - `streamSettings.xdriveSettings`: `service: "google-drive"`, `secrets: [client_id, secret, refresh_token]`；
- **客户端 Outbound 链接**：
  - 遵循指南采用 `google.com` 域前置：
  ```text
  vless://{uuid}@google.com:443?type=xdrive&service=google-drive&remoteFolder={folder_id}&encryption={encryption}&security=tls&sni=google.com&alpn=h2&mux=true
  ```

---

## 4. 逐步向导与全文解析

### 4.1 Google Drive 分步流程（Step-by-Step）
1. **Step 1: Folder ID**（Google Drive 中专为 XDRIVE 建立的文件夹 ID，URL `/folders/` 后的字符串）
2. **Step 2: Client ID**（OAuth 2.0 客户端 ID）
3. **Step 3: Client Secret**（OAuth 2.0 客户端密钥）
4. **Step 4: Refresh Token**（OAuth 刷新令牌）
5. **智能全文识别**：支持一次性粘贴提取全部 4 项凭据。
