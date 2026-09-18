# SimpleX Chat 平台接入 — 部署与验收

记录 aegis 第四个平台（SimpleX Chat）的部署结构、版本约束与手工验收清单。SimpleX 与前三个平台（Telegram / Matrix / Discord）的结构差异是：没有中心服务器 token，身份是**本机 `simplex-chat` 数据库里的联系人**，收发全部经由该进程暴露的 **WebSocket bot API**。因此部署多出一个常驻进程与一个 systemd 单元。

## 组成

| 组件 | 位置 | 说明 |
|---|---|---|
| `simplex-chat` CLI | `/etc/wwps/aegis/simplex-chat` | 由安装器下载并校验 SHA-256 |
| `wwps-simplex.service` | `/etc/systemd/system/wwps-simplex.service` | 常驻运行上述 CLI，暴露 WebSocket bot API |
| `wwps-aegis.service` | `/etc/systemd/system/wwps-aegis.service` | `ExecStart=.../aegis --simplex` |
| SimpleX 数据库 / 文件 | `/etc/wwps/aegis/simplex_store/simplex_v1`、`.../files` | 权限 `0700` |
| aegis 加密配置 | `/etc/wwps/aegis/config.enc` | 含 `simplex_port` / `simplex_admin_id` |

### 为什么必须同机

SimpleX 的 WebSocket API **不做任何鉴权**（上游明确说明），`simplex-chat` 只绑定 `127.0.0.1`。这两点决定了：

- 两个单元必须在同一台主机上，且 **`-p` 端口必须保持防火墙关闭**。把该端口暴露到公网等于交出一个无鉴权的 bot 控制接口。
- 发送文件依赖**共享文件系统**：aegis 把待发内容写入本地临时路径，`simplex-chat` 从同一路径读取。跨机部署下发文件必然失败。

安装器生成的单元只传端口，不含任何改监听地址的参数（见 `simplexSystemdUnitContent`）；端口在落盘前经过严格校验（只接受 1–65535 的纯数字串），因为该值会被字符串拼接进 root 拥有的单元文件 —— 若不加校验，换行可注入任意指令，而 `5225 --host 0.0.0.0` 会把 API 暴露出去。

## 版本锁定（最重要的一条）

`simplex-chat` 必须使用 **v7.0.0**。这不是保守建议，是硬约束：

- Rust 客户端 `simploxide-client 0.14.0` 的可接受版本范围由 `simploxide-core` 的常量决定：
  `MIN_SUPPORTED_VERSION = 7.0.0.0`，`MAX_SUPPORTED_VERSION = 7.0.0.99`。
  连接建立时会先取服务端版本，范围外直接返回 `VersionMismatch` 并拒绝连接。
- 该比较按 `MAJOR.MINOR.PATCH.HOTFIX` 逐字段进行，因此 **上游已发布的 v7.0.1 / v7.0.2 同样落在范围外**，升级即断连。
- 安装器固定下载 `releases/tags/v7.0.0`（**不是** `/releases/latest`），并校验 release 中的 SHA-256。

> ⚠️ 不要把 `simplex-chat` 交给 unattended-upgrades、`apt` 或任何自动升级机制。一旦它离开 `7.0.0.x`，aegis 会以版本不匹配拒绝启动 SimpleX 网关。恢复方式：重跑安装器，它会重新下载并覆盖锁定版本。

### 平台产物仅提供 24.04

安装器按架构选择上游产物：

| 架构 | 产物名 |
|---|---|
| amd64 | `simplex-chat-ubuntu-24_04-x86_64` |
| arm64 | `simplex-chat-ubuntu-24_04-aarch64` |

**已知限制**：只固定了 Ubuntu 24.04 的构建。在 Ubuntu 22.04 等较老 glibc 的主机上该二进制可能无法启动，而单元是 `Restart=always`，表现为**无声崩溃重启循环**（`systemctl status wwps-simplex` 会显示反复 restart，但没有明显报错）。22.04 主机请勿使用 SimpleX 平台。未知架构会被当作硬错误拒绝，而不是静默换一个架构的二进制。

## 首次配置：把管理员 contactId 写进配置

SimpleX 没有"管理员 user id"这种全局标识，管理员身份是**本机的 contactId**。引导流程：

1. 启动两个服务。`simplex-chat` 由安装器的单元首次启动时创建 bot profile（`--create-bot-display-name Aegis`）与文件目录，无需人工交互。
2. aegis 连接后按 `auto_accept_with` 建立/复用自己的地址，并**自动接受**新联系人，同时发送一条欢迎语（文案取自 i18n `simplex.welcome`，按 `config.enc` 中配置的语言本地化）。
3. 管理员用 SimpleX 客户端通过该地址主动联系 bot。
4. bot 对**非管理员**发来的消息只记日志、不派发：

   ```
   SimpleX 未授权联系人 contactId=NN 尝试发消息，已忽略
   ```

   取回该编号：

   ```bash
   journalctl -u wwps-aegis | grep 未授权联系人
   ```

   （该日志行固定为中文，与被配置的语言无关，因此这个 grep 片段是稳定的。）
5. 把编号写入 `config.enc` 的 `simplex_admin_id`，然后：

   ```bash
   systemctl restart wwps-aegis
   ```

未配置 `simplex_admin_id` 时 aegis 会明确报错并跳过调度器与启动通知：

```
SimpleX 未配置 simplex_admin_id，调度器与启动通知不会发送；请管理员先向 bot 发一条消息，从日志中取得 contactId 后写入配置
```

> 该 id 是 **本地数据库的 contactId**，不是可携带的账号标识。管理员换设备、或 `simplex_store` 被重建后，contactId 会变化，必须重新走一遍上面的流程并更新配置。

配置字段（非交互安装）：

- JSON / stdin：`"simplex_port"`、`"simplex_admin_id"`
- key=value：`simplex_port=5225`、`simplex_admin_id=42`
- 交互式安装：平台选择器选 SimpleX，随后提示端口与管理员 contactId

`config.enc` 中对这两个字段存的是密文（与 `discord_admin_id` 一致的加密存储约定）。

## 平台选择语义

一次启动运行**一种**部署形态：Telegram、Matrix、SimpleX，或两种组合之一
（Telegram + Matrix、Telegram + SimpleX）。**自动探测（无 flag）最多只启用一个平台**，
组合必须显式传 flag：

| 命令行 | config.enc 状态 | 结果 |
|---|---|---|
| 无 flag | 仅 `matrix_*` | Matrix |
| 无 flag | 仅 `simplex_*` | SimpleX |
| 无 flag | 两者都没有 | Telegram |
| 无 flag | 两者都有 | **启动失败**（配置歧义） |
| `--matrix` | 任意 | 仅 Matrix（不再自动启用 SimpleX） |
| `--simplex` | 任意 | 仅 SimpleX（同时关闭 Telegram 与 Matrix） |
| `--tg-simplex` | 任意 | Telegram + SimpleX（TG 主，敏感内容落 SimpleX） |
| `--all` | 任意 | Telegram + Matrix（永远不含 SimpleX） |
| `--tg-only` | 任意 | 仅 Telegram（不做任何自动启用） |

要点：

- **SimpleX 独立运行时仍然强制关闭 Telegram**：此时 SimpleX 适配器是主适配器，而
  Telegram dispatcher 用同一个 `state.adapter` 派发，二者同开会把 Telegram 的回复发到
  SimpleX；无 token 时还会触发 `Bot::new` 的 panic。`--tg-simplex` 是唯一允许二者共存的
  形式，此时 Telegram 仍是主适配器。
- **`--tg-simplex` 的语义与 `--all` 对等**：Telegram 是控制入口，`is_sensitive` 命中的
  文本改投 SimpleX 管理员联系人（`simplex_admin_id`）。SimpleX 侧仍可发命令，回包回到
  SimpleX。scheduler 与启动通知全局只启动一个实例，不会重复发送。
- 敏感内容的判定与 `--all` 一致（`vmess://` / `vless://` / `trojan://` / `ss://` /
  `hysteria://` / `hysteria2://` / `tuic://` 以及 `"privateKey"` / `"secretKey"` /
  `"password":` 字段）。**仅转发文本**，文件下发仍走主平台。
- `matrix_*` 与 `simplex_*` **同时齐备**时，无显式 flag 会直接报错退出，而不是悄悄二选一。
  报错信息给出两条出路：显式传 `--matrix` 或 `--simplex`，或从 `config.enc` 中移除其中
  一份配置。迁移平台时请清掉旧平台字段。
- `--tg-simplex` 的判定排在 `--simplex` **之前**，因此 flag 冲突时 `--simplex --tg-simplex`
  与 `--all --tg-simplex` 都得到 Telegram + SimpleX（沿用 aegis 既有的「按判定顺序首个
  命中者胜」行为，未新增冲突校验）。

## 手工验收清单

在真实主机上按顺序执行：

- [ ] `systemctl status wwps-simplex` 为 active，且**没有**反复重启计数
- [ ] `ss -ltnp | grep <port>` 只看到 `127.0.0.1:<port>`，不是 `0.0.0.0`
- [ ] `systemctl start wwps-aegis` 后 `journalctl -u wwps-aegis` 无 SimpleX 相关错误
- [ ] 管理员发送 `/menu`，能收到菜单（而非无响应）
- [ ] 菜单以文本命令列表渲染，形如：

      📋 **可用操作:**
      1. <按钮文本> — send: `<命令>`

      （SimpleX 无 inline keyboard，按钮渲染为文本命令，发送对应命令即可触发；这是设计行为，不是缺陷）
- [ ] 非管理员发消息被忽略，且日志出现 `未授权联系人 contactId=...`
- [ ] 触发一次文件下发（如导出配置）能收到文件
- [ ] **失败的文件发送不在 `/tmp/aegis-simplex/` 留下残留**（正常路径发送后即删；异常路径请人工确认）
- [ ] **`config.enc` 中不含 `simplex_port` / `simplex_admin_id` 的明文**：

      grep -c 5225 /etc/wwps/aegis/config.enc   # 期望 0

- [ ] 重跑一次安装器，确认 simplex-chat 被**原子替换**（不会因 `ETXTBSY` 失败），且自定义端口被保留（安装器会从既有单元回读端口）

## 已知限制

| 限制 | 说明 |
|---|---|
| **断线不自动重连** | 与 `simplex-chat` 的 WebSocket 断开后，aegis **不会**自动重连（simploxide 的重试只发生在启动阶段）。事件循环结束时日志出现 `SimpleX 事件流已结束`。恢复方式：`systemctl restart wwps-aegis`。 |
| **不支持接收文件** | `download_file` 未实现（能力位 `has_file_transfer=false`），因此 `setsecurityfile`（管理员上传文件给 bot）在 SimpleX 上不可用。发送文件不受影响。 |
| **群聊 / 本地笔记不支持** | 只处理直聊（`ChatInfo::Direct`）。群聊、本地笔记、联系人请求消息在映射阶段即被跳过；若一个事件映射不出任何消息，日志会给出 `NewChatItems 未映射出任何消息` 以提示排查。 |
| **无 typing 指示** | 未发现可用的 typing API（`can_send_typing=false`）。 |
| **无 threading** | `can_thread=false`；`send_message_threaded` 退化为普通发送。 |
| **平台产物仅 24.04** | 见上文。 |
| **self-destruct 未覆盖 simplex 单元** | aegis 的自毁清单（`rust/aegis/src/core/paths.rs` 的 `DESTRUCT_TARGETS` / `DESTRUCT_SERVICES`）未包含 `wwps-simplex`，自毁后该单元会保持 enabled 并对已删除的二进制反复重启。安装器的卸载路径（`uninstallServices` / `uninstallPaths`）**已**包含 `wwps-simplex`，因此主动卸载是干净的。 |

### 能力位对照（`PlatformCapabilities::SIMPLEX`）

```text
can_edit_message   = true      has_inline_keyboard = false
can_delete_message = true      has_slash_commands  = false
can_send_file      = true      has_file_transfer   = false
can_send_image     = true      has_e2ee            = true
can_send_reaction  = true
can_send_voice     = false     can_send_typing     = false
can_thread         = false
```

## 故障排查

| 现象 | 可能原因 |
|---|---|
| 启动即失败，日志含 `is unsupported` | `simplex-chat` 版本不在 `7.0.0.0..=7.0.0.99`（多为被自动升级到 7.0.1+）。重跑安装器恢复锁定版本。 |
| 启动即失败，含 `连接 SimpleX WebSocket 失败` | `wwps-simplex` 未运行。检查 `systemctl status wwps-simplex`；同时确认 `simplex_port` 与单元里的 `-p` 端口一致。 |
| bot 收到消息但无任何回复 | 检查日志中 `未授权联系人` —— 说明发信人 contactId 与 `simplex_admin_id` 不一致。 |
| 消息完全无反应且日志无 `未授权联系人` | 看是否有 `NewChatItems 未映射出任何消息`（群聊/非文本/协议不匹配）。 |
| 菜单发出后第一屏正常、后续屏报错 | 已知边界：文本命令路径为子命令回调合成 `MessageId("0")`，编辑该消息会失败。以报错日志为准并反馈。 |
| 调度器 / 启动通知不发送 | `simplex_admin_id` 未配置或配错（日志有明确提示）。 |

## 明确不在范围

- 群聊、channel、本地笔记（只支持直聊）。
- 接收文件与 `setsecurityfile`（需实现 `download_file` 并在目标端解密落盘）。
- typing 指示、threading。
- 断线自动重连（需在事件循环外层加重连与退避）。
- Ubuntu 22.04 等旧 glibc 主机。
