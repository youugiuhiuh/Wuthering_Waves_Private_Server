# SimpleX 平台接入设计（aegis 第四个平台）

日期：2026-09-16
状态：待评审

## 1. 背景与目标

aegis 目前支持三个平台：Telegram、Matrix、Discord（`Platform` 枚举 / `BotAdapter` trait /
`gateways/` 目录 / `main/runtime.rs` 三条网关启动路径）。

目标：新增 **SimpleX Chat** 作为第四个平台，复用现有 `BotAdapter` 抽象，运营方通过
`--simplex` 独立运行（与 `--discord` 同级的 standalone 形态）。

SimpleX 与前三者的结构差异：没有中心服务器 token，身份是本地数据库中的联系人
（`contactId`），收发全部经由本机运行的 `simplex-chat` CLI 暴露的 WebSocket API。

## 2. 选型结论

- 客户端库：`simploxide-client` 0.14.0，feature `websocket`（不启用默认的 `cli`，
  不启用 `ffi`）。
  - 许可证：未启用 `ffi` 时为 `MIT OR Apache-2.0`，可用。
  - `ws::BotBuilder::new(name, port).connect().await -> (Bot, EventStream)`。
  - `Bot` 可廉价 clone，适合放进 `Arc<dyn BotAdapter>`。
- 进程管理：由 **systemd 独立单元** 运行 `simplex-chat -p <port>`，aegis 只连接、不 spawn。
- 版本耦合（重要）：simploxide-client 0.14.0 的版本兼容表写明 simplex-chat
  **min=max=7.0.0.0**（即 v7.0.0）。部署必须锁定该版本，见 §7 风险。

## 3. Rust 侧结构

新增：

- `src/gateways/simplex/mod.rs`
- `src/gateways/simplex/adapter.rs` — `SimplexAdapter`，`impl BotAdapter`
- `src/main/simplex.rs` — `has_simplex_config()` / `connect_simplex()` / `SimplexHandle`

修改：

- `src/gateways/mod.rs`：`pub mod simplex;`
- `src/common/trait.rs`：
  - `Platform::Simplex`
  - `PlatformCapabilities::SIMPLEX`（见 §6）
- `Cargo.toml`：
  `simploxide-client = { version = "0.14", default-features = false, features = ["websocket"] }`
- `src/main.rs`：
  - `--simplex` 解析；standalone 语义（同 `--discord`）
  - adapter 优先级：discord > simplex > `build_adapter(...)`
  - `AppState::new` 传入 `simplex_admin_id`
  - `runtime::run` 传入 `simplex_handle`
- `src/main/runtime.rs`：新增 `── SimpleX ──` 段；注册 scheduler/开机通知块；
  standalone 时用 `CancellationToken` 保活（照抄 matrix-only / discord-only 分支）
- `src/common/`：把 Matrix 的 `render_markup_buttons`（`gateways/matrix/adapter.rs`）
  提升为共享函数（`pub(crate)` 或移入 `common`），SimpleX 复用同一套「按钮渲染为文本命令」逻辑

`connect_simplex` 形状（与 `connect_matrix` 对齐）：

```rust
pub struct SimplexHandle {
    pub bot: simploxide_client::ws::Bot,
    pub events: simploxide_client::EventStream,
    pub adapter: Arc<dyn BotAdapter>,
}
```

连接：`ws::BotBuilder::new("Aegis", port).auto_accept_with(欢迎语).connect()`。

## 4. 配置与身份

`EncryptedConfig`（`src/bootstrap.rs`）新增两个字段（沿用既有「非密钥也加密存储」的
`discord_admin_id` 风格）：

- `simplex_port: Option<Vec<u8>>` — WebSocket 端口（默认 5225）
- `simplex_admin_id: Option<Vec<u8>>` — 管理员 `contactId`（i64）

同步改动：

- `SetupInput` 增加同名字段
- `impl Drop for EncryptedConfig` 增加 zeroize
- `run_setup()` / `run_setup_from_stdin()` 增加参数与加密写入
- `AppState`：新增 `simplex_admin_id`，`is_admin_user()` 增加该分支
- `main/config.rs`：`load_and_validate` 解密

身份获取流程（需要写进运维文档）：bot 用 `auto_accept_with` 建立地址；管理员主动连接；
bot 对非管理员发信人 **记录其 contactId 到日志并忽略**（绝不 TOFU 授权），运营方把该
`contactId` 写入配置。

## 5. 收发映射

接收（`EventStream` dispatcher）：

- `NewChatItems` → 遍历 `chatItems: Vec<AChatItem>`
  - `chatInfo: ChatInfo::Direct { contact }` → `contact.contactId: i64`
  - `meta: CIMeta { itemId, itemText, ... }`
  - `ChatContent` / `MsgContent`：`Text { text }`、`Image { .. }`、`File { .. }`
  - 仅接受 `is_admin_user(contactId)`；其余记录 contactId 后忽略
  - 构造 `MessageEvent { target: TargetId(contactId.to_string()), user_id: contactId, .. }`
  - 文本命令：走 Matrix 同款「文本命令解析」路径（SimpleX 无平台级命令注册）

发送（`SimplexAdapter`）：

- `send_message` → `bot.send_msg(chat_id, text)`；有 markup 时先渲染为文本命令
- `edit_message` → `bot.update_msg(chat_id, msg_id, text)`
- `delete_message` → `bot.delete_msg(chat_id, msg_id, mode)`
- `send_reaction` → `bot.update_msg_reaction(...)`
- `send_file` / `send_image` → 把字节写入临时文件 → `File::new(path)` / `Image::new(path)`
  → 发送后删除临时文件（simplex-chat 与服务同机，可读到该路径）
- `answer_callback` / `send_message_threaded` / `set_system_locale` → 默认实现
- `download_file`：SimpleX 接收文件需 `accept_file` + `RcvFile*` 事件落盘。**本期列为
  不支持**（trait 默认 bail），因此 `setsecurityfile` 在 SimpleX 上不可用，能力位据实标 false。

## 6. 能力矩阵（据实申报）

| 能力 | 值 | 依据 |
|---|---|---|
| `can_edit_message` | true | `Bot::update_msg` |
| `can_delete_message` | true | `Bot::delete_msg` |
| `has_inline_keyboard` | false | 无 inline keyboard，渲染为文本命令 |
| `has_slash_commands` | false | 不做平台命令注册 |
| `has_file_transfer` | false | 本期 `download_file` 不支持 |
| `can_send_file` | true | 临时文件 + `File::new` |
| `can_send_image` | true | 临时文件 + `Image::new` |
| `can_send_voice` | false | — |
| `can_send_typing` | false | 未发现 typing API |
| `can_send_reaction` | true | `Bot::update_msg_reaction` |
| `can_thread` | false | — |
| `has_e2ee` | true | SimpleX 原生 E2EE |

## 7. 部署（installer / systemd）

`go/installer/main.go`：

- `platformSelector` 增加 Simplex 选项；`parsePlatformChoice` /
  `platformSelection` / `selectDeploymentPlatforms` 增加第 4 个 bool
  （会触及 `main_test.go:816` 的既有断言，需同步更新）
- `writeSystemdService`：`case "simplex": platformFlag = "--simplex"` + Description
- `firstTimeSetup`：采集端口与管理 contactId
- `buildSetupPayload` / `installFromStdin`：新增 simplex 字段与自动识别
- 新增 `wwps-simplex.service` 单元：`simplex-chat -p <port>`（同机、仅 localhost）
- 下载 `simplex-chat` **v7.0.0**（`simplex-chat-ubuntu-{22_04,24_04}-{x86_64,aarch64}`），
  沿用现有 `configuredReleaseRepositories` + SHA256 校验模式
- i18n：`go/installer/i18n/{en,zh,ja}.json`
- README 平台列表

## 8. 测试策略

- 纯函数单测（与 Matrix adapter 现有风格一致）：
  - 事件 → `MessageEvent` 字段映射（含非管理员被拒、文件/图片分支）
  - `TargetId` ↔ `contactId` 解析
  - markup → 文本命令渲染
  - `PlatformCapabilities::SIMPLEX` 断言
- `has_simplex_config()` 单测，镜像 `has_matrix_config` 的用例集
- 配置往返单测（镜像 `discord_config_fields_round_trip`）
- 集成：对真实 `simplex-chat v7.0.0` 的**手工**验证步骤写入文档（CI 不可自动化）

## 9. 分期

- **阶段 1（本任务）**：Rust 侧全部（§3–§6、§8）。验收：手工启动
  `simplex-chat -p 5225`，`aegis --simplex` 能收发文本/文件、菜单编辑可用、非管理员被拒。
- **阶段 2**：安装器与 systemd（§7）。依赖阶段 1 的配置字段定稿。

## 10. 风险

1. **版本硬绑定**：simploxide 0.14 ↔ simplex-chat v7.0.0 精确匹配；CLI 被独立升级后
   可能返回未文档化响应导致客户端报错。缓解：安装器锁定版本 + 文档标注不要把
   simplex-chat 交给 unattended-upgrades。
2. **第三方库成熟度**：0.x、单一维护者。缓解：被 `BotAdapter` 隔离，必要时可替换为
   `tokio-tungstenite` 直连（协议为简单 JSON）。
3. **无鉴权**：WS 无认证、仅 localhost。必须同机；防火墙关闭该端口。
4. **身份引导**：管理员换设备后 `contactId` 可能变化，需重新写配置。
5. **许可证**：`simplex-chat` CLI 为 AGPL-3.0，以独立进程方式调用、服务端不修改不分发
   其源码；simploxide 在未启用 `ffi` 时按 MIT/Apache 使用。
