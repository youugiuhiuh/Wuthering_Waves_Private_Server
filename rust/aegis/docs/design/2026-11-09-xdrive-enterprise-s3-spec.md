# SPEC: XDRIVE 国际外企白名单 S3 阵列接入规范 (Oracle Cloud & Backblaze B2 & AWS 亚太)

## 1. 目标与背景
为应对严苛的跨国网络白名单环境与断网审查，依托商业级真实云业务流量进行穿透。本规范在现有 Cloudflare R2、Google Drive 与 AWS S3 (通用) 基础上，扩展**企业/外企白名单主流 S3 兼容存储**：
1. **Oracle Cloud (甲骨文云) Object Storage**：永久免费 20GB、韩国春川/日本东京节点在国内全境直连通畅、大量跨国中微企业后台依赖。
2. **Backblaze B2**：全球主流低成本独立 S3 存储、10GB 免费空间、API 接口在 GFW 环境下长期直连稳定。
3. **AWS S3 (亚太优质节点增强)**：强化 `ap-northeast-1` (东京)、`ap-east-1` (香港)、`ap-southeast-1` (新加坡) 的一键预设与推荐。

---

## 2. 国际服务商菜单布局与按钮规划 (`u_xdrive_menu`)

供应商菜单将组织为 4 排分类按钮：
- **第 1 排（零门槛 / 免费直连王牌）**：
  - ☁️ `Cloudflare R2` (`u_xdrive_p:r2`) —— 免出网流量费 / 10GB 免费
  - 📁 `Google Drive` (`u_xdrive_p:gdrive`) —— 谷歌网盘 / 域前置直连 / 免翻墙
- **第 2 排（跨国企业白名单顶级通道）**：
  - 🟠 `AWS S3 (国际版)` (`u_xdrive_p:aws`) —— 亚太东京/香港/新加坡 / 外企白名单 T0
  - 🏛️ `Oracle Cloud S3` (`u_xdrive_p:oracle`) —— 甲骨文云 20GB 免费 / 春川/东京直连
- **第 3 排（高性价比 / 区域白名单穿透）**：
  - 🔴 `Backblaze B2` (`u_xdrive_p:b2`) —— 10GB 免费 / 独立稳定 S3
  - 🇷🇺 `Yandex / Arvan S3` (`u_xdrive_p:regional`) —— 俄伊本地白名单
- **第 4 排（通用与系统）**：
  - ⚙️ `自定义 S3 配置` (`u_xdrive_p:s3`) —— 通用 S3 兼容存储
  - 🔙 `返回批量菜单` (`m_batch_menu`)

---

## 3. 技术规范与构建器设计

### 3.1 Oracle Cloud (甲骨文云) Object Storage
- **S3 Endpoint 规则**：
  `https://{namespace}.compat.objectstorage.{region}.oraclecloud.com`
- **默认推荐 Region**：
  - `ap-chuncheon-1` (韩国春川)
  - `ap-tokyo-1` (日本东京)
  - `ap-seoul-1` (韩国首尔)
  - `ap-singapore-1` (新加坡)
- **寻址方式 (Addressing Style)**：
  - `pathStyle: true`（必须使用 Path-Style：`{endpoint}/{bucket}/{object}`，甲骨文 S3 兼容 API 对 Virtual-Hosted 风格存在 SNI 与证书不匹配限制）
- **凭据机制**：
  - 甲骨文用户控制台生成的 **Customer Secret Key**（Access Key ID + Secret Key 组合）；
  - 必须提供用户的 **Object Storage Namespace**（租户命名空间字符串，通常为 10~14 位字符串）。
- **构造器签名 (`rust/aegis/src/core/xray/xdrive.rs`)**：
  ```rust
  pub fn new_oracle_cloud_s3(
      namespace: &str,
      region: &str,
      bucket: &str,
      access_key_id: &str,
      secret_access_key: &str,
      folder: &str,
  ) -> Self
  ```

### 3.2 Backblaze B2 (S3 兼容接口)
- **S3 Endpoint 规则**：
  `https://s3.{region}.backblazeb2.com`
- **默认推荐 Region**：
  - `us-west-004` (美西)
  - `us-east-005` (美东)
  - `eu-central-003` (欧洲)
- **寻址方式 (Addressing Style)**：
  - `pathStyle: false`（推荐 Virtual-Hosted：`{bucket}.s3.{region}.backblazeb2.com`）
- **凭据机制**：
  - Backblaze 颁发的 **Application Key**：
    - `keyID`（以十六进制组成的 25 位字符串）；
    - `applicationKey`（31 位主密钥）。
- **构造器签名 (`rust/aegis/src/core/xray/xdrive.rs`)**：
  ```rust
  pub fn new_backblaze_b2(
      region: &str,
      bucket: &str,
      key_id: &str,
      application_key: &str,
      folder: &str,
  ) -> Self
  ```

---

## 4. 对话式向导与整页文本智能解析器

在 `rust/aegis/src/core/xray/xdrive.rs` 中提供专用的全文本自动提取函数：

1. **`extract_oracle_cloud_tokens(input: &str)`**：
   - 自动检测并提取甲骨文控制台整页复制内容中的 `Namespace`、`Access Key`、`Secret Key`、`Region` 与 `Bucket`；
2. **`extract_backblaze_b2_tokens(input: &str)`**：
   - 自动检测并提取 `keyID` (`^[a-f0-9]{25}$`)、`applicationKey` (`^[a-zA-Z0-9]{31}$`)、`s3.<region>.backblazeb2.com`。

---

## 5. 客户端 `aegis_v2rayN` 联动设计

1. **`Global.XdriveProviders` 扩展**：
   - 追加 `"Oracle Cloud S3"`
   - 追加 `"Backblaze B2"`
2. **`AddServerViewModel.ApplyXdriveProviderPreset` 联动**：
   - 选择 `Oracle Cloud S3`：`XdriveService = "s3"`, `XdriveS3PathStyle = true`, 默认 Region 填充 `ap-chuncheon-1`；
   - 选择 `Backblaze B2`：`XdriveService = "s3"`, `XdriveS3PathStyle = false`, 默认 Region 填充 `us-west-004`。
3. **9 语言本地化**：
   - 在 `ResUI.*.resx` 中提供友好显示。

---

## 6. 测试与验收标准 (TDD)
1. **构造器单元测试**：验证 `new_oracle_cloud_s3` 和 `new_backblaze_b2` 生成的 S3 配置（Endpoint、Region、PathStyle、Secrets）符合标准；
2. **智能解析器单元测试**：验证各种格式粘贴文本的容错解析；
3. **向导会话状态测试**：模拟 Telegram 会话全流程交互，确保能正确生成出站及客户端分享链接；
4. **客户端往返测试**：在 `aegis_v2rayN` 中验证新预设的生成、保存、分享 URI 导入导出。
