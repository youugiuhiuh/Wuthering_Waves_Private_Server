# SPEC: XDRIVE 国际版 AWS S3 预设与纯国际服务商体系

## 1. 目标与背景
用户明确指示：**「先不实现阿里云这种中国的厂商」**，规避中国大陆境内云厂商的实名制与账号审计风险。
本规范聚焦于**纯国际化顶级云存储中转方案**，在现有 Cloudflare R2 与 Google Drive 的基础上，正式接入 **AWS S3 (国际版)** 专属极速预设，构建国际企业通道。

---

## 2. 国际服务商菜单布局（`u_xdrive_menu`）
- **第 1 排**：
  - ☁️ `Cloudflare R2` (`u_xdrive_p:r2`) —— 免备案 / 零出网流量费 / 每月 10G 免费
  - 📁 `Google Drive` (`u_xdrive_p:gdrive`) —— 谷歌网盘 / 域前置直连 / 免翻墙
- **第 2 排**：
  - 🟠 `AWS S3 (国际版)` (`u_xdrive_p:aws`) —— 国际企业白名单黄金通道 / 全球区域
  - 🇷🇺 `Yandex Cloud S3` (`u_xdrive_p:yandex`) —— 俄罗斯本土 / 白名单穿透
- **第 3 排**：
  - 🇮🇷 `ArvanCloud S3` (`u_xdrive_p:arvan`) —— 伊朗本土 / 白名单穿透
  - ⚙️ `自定义 S3 配置` (`u_xdrive_p:s3`) —— 通用 S3 兼容存储
- **第 4 排**：
  - `🔙 返回批量菜单` (`m_batch_menu`)

---

## 3. AWS S3 (国际版) 技术规范

### 3.1 核心配置与寻址风格
- **Endpoint 规则**：`https://s3.{region}.amazonaws.com`
- **默认/推荐区域**：`ap-northeast-1`（东京）、`ap-southeast-1`（新加坡）、`us-east-1`（美东）
- **寻址方式 (Addressing Style)**：`pathStyle: false`（Virtual-hosted 风格，符合 AWS S3 现代区域规范）
- **凭据要求**：
  - `AccessKeyID`：以 `AKIA...` 开头的 20 位 IAM 访问密钥 ID；
  - `SecretAccessKey`：40 位密钥。

### 3.2 预设构建器 (`xdrive.rs`)
```rust
impl XdriveConfig {
    pub fn new_aws_s3(
        region: &str,
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
                endpoint: format!("https://s3.{region}.amazonaws.com"),
                region: region.to_string(),
                bucket: bucket.to_string(),
                path_style: false,
            }),
            segment_bytes: Some(1024 * 1024),
            poll_interval_ms: Some(25),
            max_poll_interval_ms: Some(800),
            flush_interval_ms: None,
            eager_window_ms: None,
            concurrency: None,
        }
    }
}
```

---

## 4. 交互与问答流程 (`message.rs`)
- **Step 1: 区域 (Region)**
  - 提示文案列出常用区域代码（`ap-northeast-1`, `ap-southeast-1`, `us-east-1`），默认若用户直接回车或输入常用地名自动转换；
- **Step 2: 存储桶名称 (Bucket)**
- **Step 3: Access Key ID**（校验以 `AKIA` 开头并给出 IAM 指引）
- **Step 4: Secret Access Key**（40 位密钥）
- **一键全文解析**：支持粘贴 AWS 控制台创建完成的 `credentials.csv` 或文本块自动提取。
