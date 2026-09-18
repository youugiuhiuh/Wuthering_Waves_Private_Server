# 设计：白名单自愈（TOTP 成为凭据，contactId 降级为标签）

> 模式：**strict**（安全逻辑变更）｜ 状态：**待批准** ｜ 日期：2026-09-18
> 分支基线：main（#339 合并后）

## 1. 问题

`simplex_admin_id` 把管理员钉在一个 **contactId** 上，而 contactId 是 simplex-chat 本地
联系人记录的编号 —— **删掉联系人再重加、重装客户端、换设备都会变**（实测 3 → 4）。

后果：每次身份变化都要手工 `aegis --set-simplex-admin <新ID>` + 重启。
而在纯 `--simplex` 形态下这就是**完全无法操作**（控制面就是 SimpleX）。

根因不是白名单写错了，而是**把权限挂在了一个易变的标识上**。
SimpleX 的隐私模型里**没有全局用户标识**，所以「用稳定 ID 认人」在这个平台上不存在。

## 2. 目标

重连后无需任何手工步骤即可恢复管理员权限，且不因此把公开地址变成可无限试密码的靶子。

非目标：
- 不改 Telegram 侧（TG chat id 稳定，无此问题）
- 不做多管理员/角色体系
- 不处理公开地址带来的「联系人无限增长」与 journal 噪声（独立问题，见 §7）

## 3. 必须同时放开**两道**门禁（只放开一道会做成半成品）

| 门禁 | 位置 | 现状 | 自愈路径为何被它挡住 |
| --- | --- | --- | --- |
| ① contactId 白名单 | `main/runtime.rs:317` | `if simplex_admin != Some(msg.contact_id) { warn; continue }` | 新 contactId 的消息**根本进不了 dispatch**，6 位码也送不进去 |
| ② `check_auth` | `shared/dispatch.rs:77-79` | `if !state.is_admin_user(user_id) { return false }` | 即使消息进来了，也会在 TOTP 之前被挡回 |

注意 `check_auth` 里那段「TOTP codes allowed when not authorized」（`:89-96`）**只对已
通过 `is_admin_user` 的人生效** —— 它对陌生人是不可达的。这正是上一轮讨论里
「TOTP 是第二因子、不是独立门」的代码级证据。

改法：
- ①：6 位纯数字文本的消息**放行**（其余仍按现状丢弃并记 `未授权联系人`）
- ②：6 位纯数字文本**不受 `is_admin_user` 约束**放行，交给 `auth::process_auth_code`

## 4. 自愈动作

`app/auth.rs::process_auth_code`（`:20-31` 现在是「非管理员直接回无权限」）：

1. 去掉开头的 `is_admin_user` 拒绝
2. 先查限流冷却（§5）
3. `verify_totp(code)` 成功 →
   - **重新钉定**：`crate::bootstrap::set_simplex_admin_id(&config_dir(), user_id)` 落盘
     （`src/lib.rs:6` 已有 `pub mod bootstrap;`，**lib 可直接调用，无需注入 sink**）
   - **更新内存态**，使当前进程立即生效（不能只写盘等重启 —— 那正是要消灭的手工步骤）
   - `record_auth_success(user_id, now)` 建立会话
   - **高声记日志**（这是一次权限变更，必须可审计）
4. 失败 → 走既有 `record_auth_failure`

`set_simplex_admin_id` 是既有的**定点字段替换**（保留其余字段、不轮换 TOTP），直接复用。

## 5. 限流：本设计最关键的一处

### 现状

`failed_attempts: Mutex<HashMap<i64, FailedRecord>>`（`state.rs:79`）—— **按 user_id 独立计数**，
5 次 / 600 秒，升级梯度 `[900, 3600, 86400, 172800]`（`dispatch.rs:140-146`）。

白名单存在时，攻击者只能有 **1** 个身份，所以「per-user 限流」等价于全局限流。
**门禁一放开，这个等价关系就没了**：公开地址 + 自动接受 ⇒ 攻击者可造 N 个联系人，
拿到 N 份独立预算。

算术（6 位 / 30 秒 / `skew=1` ⇒ 3 个码同时有效 ⇒ 单次命中 3/10⁶，期望 ~23 万次成功猜测）：

| 场景 | 猜测吞吐 | 50% 命中所需 |
| --- | --- | --- |
| 现状（1 个身份，5 次/600s） | ~720 次/天 | ~460 天 |
| 门禁放开后不补限流（1000 联系人） | ~72 万 次/天 | **~11 小时** |

### 必须加的两条

1. **全局（按 bot）计数器**，与 per-contact 计数器并存
2. **锁定时长封顶** ⚠️ **这是最容易做错的一处**

`record_auth_failure` 用 `lockout_durations.get(lock_level).or_else(|| lockout_durations.last())`
取值 —— 即**梯度最后一级是有效封顶，而它是 `172800s = 48 小时`**。
若全局限流复用它，攻击者只要烧够次数，就能把**真管理员锁在门外 48 小时**（自 DoS）。

**本设计取全局封锁封顶 = `900s`（15 分钟）**，与梯度第一级一致。

### 为什么 15 分钟封顶是可接受的

`failed_attempts` 是**纯内存态**（`state.rs:79/108`；全仓无任何持久化引用，已核实）⇒
`systemctl restart wwps-aegis` 立即清零。管理员在服务器上有 root，
「被锁 → 等 ≤15 分钟 或 重启」两条路都在自己手上。自 DoS 因此被封住。

## 6. 安全边界的诚实陈述

**这个改动之后，SimpleX 路径的权限完全由 TOTP 密钥承担，contactId 只是日志标签。**

必须同时接受：

| 事实 | 影响 |
| --- | --- |
| 6 位码成为**对公网可达的预言机** | 由 §5 的全局限流承担；不再是白名单 |
| TOTP 密钥成为**bearer 凭据** | 它存在 `config.enc`（0600），且**安装时被打印到终端**。泄露即失守 |
| 地址泄露的性质变了 | 从「低危噪声」变为「限流下的暴力破解面」 |
| 公开地址 + 自动接受仍在 | 联系人无限增长、journal 噪声（#339 起每个连接者都进日志）—— 本设计**不解决** |

若要降低这些，唯一的正解是**配完就关门**：`Bot::delete_address()` 或
`APISetAddressSettings` 关掉 `autoAccept`（`simploxide-client` 已暴露
`delete_address`/`configure_address`/`accept_contact`/`reject_contact`）。**建议作为后续独立改动。**

## 7. 待你裁决

| # | 决策 | 我的建议 |
| --- | --- | --- |
| D1 | 重新钉定是**覆盖**单个 `simplex_admin_id`，还是保留**有界列表**（旧 ID 仍有效）？ | **覆盖** —— 沿用既有字段，不改 config schema（YAGNI） |
| D2 | 哪些平台允许自愈？ | **仅 SimpleX**（TG chat id 稳定，无此需求） |
| D3 | `auth.rs` 如何知道「这是 SimpleX 的 contactId」？ `user_id` 命名空间是 TG/SimpleX 共用的 | 由调用方**显式传平台标记**（比 adapter 内省更好测）；具体形式待定 |
| D4 | 全局封锁封顶 | **900s**（理由见 §5） |
| D5 | 自愈是否要求 `simplex_admin_id` 为空时才生效？ | **不要求** —— 否则「已有 pin 后重连」无法自愈，正是要修的场景 |
| D6 | #339 是否先合并，本改动另开分支？ | **先合并 #339**（已端到端验证通过），再从新 main 开 `fix/simplex-admin-selfheal` |

## 8. 验证

| 层 | 断言 |
| --- | --- |
| 单测 | 新 contactId + 正确码 → 重新钉定 + 会话建立 + 落盘被调用；错码 → 计入失败；全局封锁达 5 次后**对所有人都封锁**，且 ≤900s |
| 回归 | `is_admin_user`/`is_authorized` 的原语义不破（既有测试须全绿） |
| 门禁 | Rust 全量门禁（fmt / clippy `-D warnings` / nextest / doctest），基线 **926 passed, 1 skipped** |
| 真实主机（硬断言） | 删掉联系人重连 → 发 6 位码 → **无需任何手工步骤**即可用；`journalctl` 出现权限变更日志 |
| 真实主机（限流） | 连发 6 次错码 → 第 6 次被拒且提示等待时长；`systemctl restart` 后立即恢复可试 |

## 9. 本设计不覆盖（留给后续）

1. 公开地址 + 自动接受导致的**联系人无限增长**与 journal 噪声
2. 配完关门（`delete_address` / 关 `autoAccept`）
3. 非管理员发消息的**静默丢弃**（`runtime.rs:317` 只记日志不回话；`auth.rs` 已有
   `auth.no_permission` 文案可复用）
4. 欢迎语对所有连接者宣称「发送 /menu」，而只有管理员能用
