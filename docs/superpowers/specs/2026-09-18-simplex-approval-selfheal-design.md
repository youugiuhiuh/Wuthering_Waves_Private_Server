# 设计：SimpleX 连接审批与管理员自愈（已实证版）

> 模式：**strict** ｜ 状态：**待批准** ｜ 日期：2026-09-18
> 本文档取代 `2026-09-18-simplex-admin-selfheal-design.md` 的假设部分 —— 关键结论已由真实主机实验证实。

## 1. 已实证的事实（实验，非推断）

实验环境：测试机 `murky-pull`，aegis = 本分支构建，SimpleX 客户端为管理员本人的客户端。
通过 SSH 隧道 + WebSocket 直连 simplex-chat 的 API（`127.0.0.1:5225`）执行，实验后已复原。

| # | 实验 | 结果 | 证据 |
| --- | --- | --- | --- |
| E1 | 关掉 `autoAccept` 后重连并尝试发消息 | **接受之前完全无法通信** | 客户端报「无法发送信息」；journal `新联系人连接` 1→1、`未授权联系人` 6→6（**均未增加**） |
| E2 | 关掉 `autoAccept` 的确切方式 | `autoAccept: null` | `/_address_settings 1 {"businessAddress":false,"autoAccept":null,...}` → 返回 `userContactLinkUpdated`；`/_show_address` 复核 `autoAccept` 已缺失 |
| E3 | 待批准请求是否可被观察 | **可以** | `/_get chats 1 pcc=on` → `chats[0].chatInfo.contact = {contactId: 5, localDisplayName: "Riyo Tsuihiji_2"}` |
| E4 | 第二个 WS 客户端是否影响 aegis | **无影响** | 改设置前后 `ActiveState=active`、`NRestarts=0` |
| E5 | 欢迎语与 `autoAccept` 的关系 | **是两个独立字段** | `autoReply` 独立存在；关 `autoAccept` 后 `autoReply` 仍在，但不再触发（无自动接受动作） |

**E1 的推论（本设计的基石）**：`输入连接 → 发 TOTP → 验证通过才接受` **在技术上不可能** ——
TOTP 码本身要靠消息送达，而消息在「接受前」传不过来。存在死锁：

```
要验证 → 需传码 → 需已接受　‖　要接受 → 想先验证 → 死锁
```

**E1 的解（本设计的核心思路）**：把审批决策**换到另一个信道**（Telegram）。
TG 不依赖 SimpleX 连接是否被接受，因此死锁解除，且「批准需要 TOTP」**白送** ——
TG 侧 `check_auth` 本来就要求所有命令/回调先有 TOTP 会话（`dispatch.rs:75-101`）。

## 2. API 能力面（已核实，零新增依赖）

| 能力 | API | 位置 |
| --- | --- | --- |
| 看到待批准请求 | `/_get chats <uid> pcc=on` → `apiChats` | `commands.rs:1752-1795` |
| 事件形式 | `Event::ReceivedContactRequest { user, contact_request, chat }` | `events.rs:614-627` |
| **批准** | `/_accept <contactReqId>` | `commands.rs:1610-1622` |
| **拒绝**（**不通知对方**） | `/_reject <contactReqId>` | `commands.rs:1640-1652` |
| 关/开自动接受 | `/_address_settings <uid> <json>`，`autoAccept: null` = 关 | `lib.rs:92-104` |
| 列出联系人（含 `created_at`/`chat_ts`） | `/_contacts <uid>` | `commands.rs:1670` |
| 删会话/联系人 | `/_delete <chatRef> <mode>` | `commands.rs:1810` |

**三个决定实现顺序的细节**：

1. `ReceivedContactRequest.contact_request.contact_request_id` 是批准所需的 ID
2. **`ContactConnected` 不带 `contactRequestId`**（`events.rs:475-488`）⇒ 必须在
   `ReceivedContactRequest` **到达那一刻捕获**，接受之后无法回溯
3. 拒绝**静默**（`commands.rs` 原文：The user who sent the request is **not notified**）

## 3. 设计

### 主路径：Telegram 带外审批（`--tg-simplex` / `--all`）

```
autoAccept 永久关闭
  → 有人连接 → ReceivedContactRequest（aegis 现在丢弃它）
  → 捕获 contactRequestId，TG 推一条带 [允许][拒绝] 的按钮消息
  → 你点「允许」→ 该回调需要 TOTP 会话（check_auth 自动拦截，无需新机制）
  → accept_contact(contactRequestId) → 对方成为联系人
  → 但仍是**普通联系人**：消息照样被门禁①丢弃（连接 ≠ 管理员）
  → 若这就是你重连的新身份 → 你发 6 位码 → 钉定为管理员（自愈）
```

### 必带的旁路：本地 CLI（防 Telegram 成为单点）

**整个方案的命门**：若 TG 不可用（token 失效 / 被限制 / bot 被 ban / 手机丢失），
就再也无法批准任何人，**包括你自己重连的新身份** → 永久锁死在 SimpleX 之外。
你刚经历过一次「配置对不上就完全没权限」，这条不能只有一条路。

```
aegis --list-contact-requests                # 列出在敲门的（读 pcc=on）
aegis --approve-contact <contactRequestId>  # 不依赖 TG
                                                          # 注：_accept 需要 contactRequestId，不是 contactId
aegis --reject-contact <contactRequestId>
```

### 自愈：TOTP 成为凭据

**必须同时放开两道门禁**（只放开一道是半成品）：

| 门禁 | 位置 | 现状 | 为何挡路 |
| --- | --- | --- | --- |
| ① contactId 白名单 | `main/runtime.rs:317` | `if simplex_admin != Some(msg.contact_id) { continue }` | 新 contactId 的 6 位码**进不了 dispatch** |
| ② `check_auth` | `shared/dispatch.rs:77-79` | `if !state.is_admin_user(user_id) { return false }` | 即使进来，也在 TOTP 之前被挡回 |

`check_auth` 里那段「TOTP codes allowed when not authorized」（`:89-96`）**只对已通过白名单的人生效**。

改法：6 位纯数字文本在①②**放行**；`auth.rs::process_auth_code`（`:20-31`）去掉开头的
`is_admin_user` 拒绝，改为先验证 TOTP，成功后：

- **重新钉定**：`crate::bootstrap::set_simplex_admin_id(&config_dir(), user_id)`
  （`src/lib.rs:6` 有 `pub mod bootstrap;` ⇒ **lib 可直接调用，无需注入 sink**）
- **更新内存态**（不能只写盘等重启 —— 那正是要消灭的手工步骤）
- `record_auth_success` 建会话；**高声记日志**（权限变更必须可审计）

### 限流：本设计最危险的一处

`failed_attempts: Mutex<HashMap<i64, FailedRecord>>`（`state.rs:79`）**按 user_id 独立计数**，
5 次 / 600 秒，梯度 `[900, 3600, 86400, 172800]`（`dispatch.rs:140-146`）。

白名单在时攻击者只有 1 个身份，per-user ≈ 全局。**门禁放开后这个等价关系消失**：

| 场景 | 猜测吞吐 | 50% 命中所需 |
| --- | --- | --- |
| 现状（1 身份） | ~720/天 | ~460 天 |
| 放开后不补限流（1000 联系人） | ~72 万/天 | **~11 小时** |

必须加**全局（按 bot）计数器**，且**锁定时长封顶** ⚠️

`record_auth_failure` 取 `lockout_durations.last()` 作封顶，而最后一级是 **172800s = 48 小时**
⇒ 若全局复用它，攻击者能把你**锁在门外 48 小时**。**本设计取全局封顶 = 900s。**

可接受的理由：`failed_attempts` 是**纯内存态**（`state.rs:79/108`，全仓零持久化引用，已核实）
⇒ `systemctl restart` 立即清零，而你在服务器上有 root。「等 ≤15 分钟 / 重启」两条路都在自己手上。

### 纯 `--simplex` 形态

没有 TG 可提示 ⇒ 只能靠**本地 CLI 审批**（同上旁路），或退回「限时登记窗口」
（本地命令开窗 N 分钟，到期自动关）。**建议先只做 CLI 审批**，窗口留待有需求再加。

## 4. 分阶段（避免把 4 个独立改动捆成一次）

| 阶段 | 内容 | 交付价值 | 依赖 |
| --- | --- | --- | --- |
| **P1** | **自愈 + 全局限流**（两道门禁 + 重钉 + 内存态 + 限流封顶） | 重连**不用读 journal 抄 ID**，发码即可 | 无 |
| **P2** | **连接审批**（`ReceivedContactRequest` → TG 按钮 + CLI 旁路 + 关 `autoAccept`） | 陌生人**连都连不上**；地址泄露近乎无用；联系人不再增长 | **P1**（先关自动接受而没有自愈 = 自己把自己锁死） |
| P3（可选） | 限时开窗（纯 simplex 无 TG 时）、敲门通知、清理陈旧联系人 | 体验 | P2 |

**P1 必须先做**：若先关 `autoAccept` 而没有自愈，你重连时会被挡在 `ReceivedContactRequest`，
而它又无法发 TOTP 码 —— 直接锁死。这个顺序不能颠倒。

## 5. 安全边界的诚实陈述（P1 之后）

| 事实 | 承担者 |
| --- | --- |
| TOTP 密钥成为 **bearer 凭据** | 存于 `config.enc`(0600)，但**安装时被打印到终端**；泄露即失守 |
| 6 位码成为**对公网可达的预言机** | 全局限流（封顶 900s） |
| 地址泄露 | P2 之前：限流下的暴力破解面；**P2 之后：近乎无用** |

## 6. 实施顺序（**不是选项，是事实约束**）

本方案为**一套**改动，分两步交，因为第二步硬依赖第一步：

1. **先**自愈 + 全局限流（P1）
2. **再**在此基础上接 TG 审批 + 关 `autoAccept`（P2）

为何不能颠倒：P2 的 `autoAccept` 关闭后，新连接会停在 `ReceivedContactRequest`；
而该状态下 **E1 实测无法传消息** ⇒ 若此时还没有自愈，你重连时既进不来、也无法发码 ⇒ 锁死。

=> **P1 完成并验证后直接进入 P2，无需再次征求顺序意见。**

> **实施状态（2026-09-18）**：P2 已实施于分支 `feat/simplex-p2-approval`
> （计划：`docs/superpowers/plans/2026-09-18-simplex-p2-approval.md`），含 TG 审批、
> 本地 CLI 旁路、autoAccept 永久关闭、平台来源感知重钉（`--tg-simplex` 自愈）。

## 7. 待你裁决（仅限设计细节）

| # | 决策 | 我的默认值（你不反对即按此实施） |
| --- | --- | --- |
| D2 | 重钉语义：**覆盖**单个 `simplex_admin_id` 还是有界列表？ | **覆盖**（不改 config schema） |
| D3 | 自愈平台范围 | **仅 SimpleX**（TG chat id 稳定） |
| D4 | `auth.rs` 如何知道「这是 SimpleX 的 contactId」（`user_id` 命名空间共用） | 调用方**显式传平台标记** |
| D5 | 全局封锁封顶 | **900s**（理由：`failed_attempts` 纯内存态，重启即清） |
| D6 | P2 的批准按钮走哪条适配器 | `--tg-simplex` 下走已是主适配器的 Telegram |
| D7 | 立即恢复可用（`--set-simplex-admin 4`）？ | **待你说一声**（会写 config.enc 并重启服务） |

## 8. 验证

| 层 | 断言 |
| --- | --- |
| 单测 | 正确码 + 新 contactId → 重钉 + 落盘被调用 + 建会话；错码计入失败；全局达 5 次对所有身份封锁且 ≤900s；`is_admin_user`/`is_authorized` 原语义不破 |
| 门禁 | `fmt --check` ｜ clippy `-D warnings` ｜ nextest（基线 926 passed, 1 skipped） ｜ doctest |
| 真实主机（自愈） | 删联系人重连 → 发 6 位码 → **无任何手工步骤**即可用；journal 出现权限变更日志 |
| 真实主机（限流） | 连发 6 次错码 → 被拒并提示等待；`systemctl restart` 后立即恢复可试 |
| 真实主机（P2） | `autoAccept` 关 → 陌生人连接 → TG 收到按钮；未登录 TOTP 时点按钮无效；批准后成为联系人但仍无权限 |

## 8. 本设计不覆盖

1. 「配完关门」的地址撤销（`delete_address`）—— 与关 `autoAccept` 不同，撤销会改变地址
2. 非管理员发消息的静默丢弃（`runtime.rs:317` 只记日志不回话；`auth.no_permission` 文案已有）
3. 欢迎语对所有连接者宣称「发送 /menu」—— P2 关掉自动接受后**自动消失**（E5）
4. `uninstallAegis` 不清理 `wwps-simplex` 单元
