# SPEC: XDRIVE 傻瓜式逐步问答向导与保姆级开通指引

## 1. 目标与背景
当前 XDRIVE 存储中转向导仅展示了单条静态卡片，要求用户一次性按换行输入 3~5 项敏感凭据，存在以下痛点：
1. **用户不知道去哪里开通**：小白用户对各云厂商的对象存储（S3/R2）没有概念，缺乏开通指引与控制台直达入口；
2. **格式容易错**：换行分隔或手滑少了一行会导致解析失败；
3. **缺乏状态闭环**：未接通会话输入监听，发送凭据后没有被消费或保存。

本规范定义一套**逐步问答式向导（Conversational Step-by-Step Wizard）**，辅以**保姆级 30 秒开通指引**，将 XDRIVE 节点创建门槛降至零。

---

## 2. 交互流程设计

### 2.1 地区选择与保姆级指引卡片
在 `u_xdrive_menu` 菜单点击地区（CN / RU / IR / Custom）后，Bot 回复对应云厂商的保姆级开通指引与第一步提问：

#### 🇨🇳 Cloudflare R2（推荐中国大陆用户）
- **特点说明**：免备案 / 零出网流量费 / 每月 10GB 免费 / 免绑定信用卡
- **3 步极速开通**：
  1. 登录 [Cloudflare 控制台](https://dash.cloudflare.com/) 并在首页右侧复制 **账户 ID (Account ID)**；
  2. 点击左侧导航栏 **「R2 对象存储」** -> **「创建存储桶」**（名称如 `my-proxy`，其余默认）；
  3. 点击右侧 **「管理 R2 API 令牌」** -> **「创建 API 令牌」** -> 权限选 **「对象读写」** -> 获得 Access Key ID 与 Secret Access Key。
- **逐步提问序列**：
  - 第 1 步：请输入您的 **Cloudflare 账户 ID (Account ID)**（32位字符串）
  - 第 2 步：请输入刚刚创建的 **存储桶名称 (Bucket)**
  - 第 3 步：请输入 **Access Key ID**
  - 第 4 步：请输入 **Secret Access Key**

#### 🇷🇺 Yandex Cloud S3（推荐俄罗斯用户）
- **控制台链接**：[Yandex Cloud Console](https://console.yandex.cloud/)
- **开通步骤**：
  1. 进入 Object Storage，创建存储桶（Bucket）；
  2. 进入 Service Accounts，创建账号并生成 Static Access Key。
- **逐步提问序列**：
  - 第 1 步：请输入 **存储桶名称 (Bucket)**
  - 第 2 步：请输入 **Access Key ID**
  - 第 3 步：请输入 **Secret Access Key**

#### 🇮🇷 ArvanCloud S3（推荐伊朗用户）
- **控制台链接**：[ArvanCloud Storage](https://panel.arvancloud.ir/)
- **开通步骤**：创建 Bucket 并于 API Keys 页面生成 S3 密钥。
- **逐步提问序列**：同 Yandex S3（Bucket -> AK -> SK）。

#### ⚙️ 自定义 S3 兼容存储
- **逐步提问序列**：
  - 第 1 步：请输入 **S3 接入点 (Endpoint)**，例如 `https://s3.us-east-1.amazonaws.com`
  - 第 2 步：请输入 **区域 (Region)**，例如 `us-east-1`
  - 第 3 步：请输入 **存储桶名称 (Bucket)**
  - 第 4 步：请输入 **Access Key ID**
  - 第 5 步：请输入 **Secret Access Key**

---

### 2.2 会话控制与辅助按钮
在每一步提问的下方附带 Inline 键盘：
- `[❌ 取消配置]` (`u_xdrive_cancel`)：立即中止向导、清除会话缓存，并返回 XDRIVE 地区菜单。
- 会话有效期：180 秒（超时自动失效）。

---

## 3. 架构设计与状态机

### 3.1 状态数据模型 (`AppState`)
```rust
#[derive(Debug, Clone)]
pub struct XdriveWizardSession {
    pub region: String,
    pub step: u8, // 1, 2, 3...
    pub account_id: Option<String>,
    pub endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub bucket: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub started_at: std::time::Instant,
}
```
在 `AppState` 中维护 `xdrive_wizards: Arc<RwLock<HashMap<String, XdriveWizardSession>>>`（以 `target_id` 为键）。

### 3.2 消息路由器 (`src/shared/dispatch.rs` & `message.rs`)
在收到普通文本消息时：
1. 优先检查 `state.get_xdrive_wizard(&target)`；
2. 若存在且未超时（< 180s）：
   - 提取输入文本并修剪两端空白；
   - 匹配当前 `step` 校验并写入对应字段；
   - 若还有下一步：`step += 1`，发送下一步的指引文案；
   - 若所有步骤已完成：
     - 构建 `XdriveConfig`；
     - 校验配置合法性（`config.validate()`）；
     - 调用 `ConfigManager::create_xdrive_node` 写盘、重启核心、生成订阅链接；
     - 清除会话状态；
     - 向用户发送生成成功的节点链接卡片与二维码。
3. 若用户发送 `/cancel` 或点击 `[❌ 取消配置]`，直接清除会话。

---

## 4. 实施阶段计划
- **Phase 1: 状态机与核心解析**
  - 在 `src/app/state.rs` 增加 `XdriveWizardSession` 管理函数；
  - 编写状态流转单元测试（TDD RED -> GREEN）。
- **Phase 2: 引导文案与多语言 (`i18n`)**
  - 在 `zh.yml`, `en.yml`, `ja.yml` 中补充各地区的保姆级步骤文案与官方直达链接。
- **Phase 3: 消息输入钩子与端到端落盘**
  - 在 `src/shared/handlers/xray.rs` 与 `message.rs` 中接通文本输入分支；
  - 完成节点创建、链接下发与回退取消流程。
- **Phase 4: 质量把关**
  - 运行 `cargo fmt`, `cargo clippy`, `cargo nextest`。
