# P2 实施计划：TG 带外审批 + 本地 CLI 旁路 + 永久关闭 autoAccept

> 模式：**strict** ｜ 状态：**待批准** ｜ 日期：2026-09-18
> Spec（设计）：`docs/superpowers/specs/2026-09-18-simplex-approval-selfheal-design.md` §3（P2 阶段）
> 前置：P1（自愈 + 全局限流）已合并于 `aa27353`

## 0. 目标

| # | 交付 | 验收（可观测） |
| --- | --- | --- |
| G1 | `autoAccept` 永久关闭 | `_show_address` 不含 `autoAccept`；陌生人连接**不自动成为联系人** |
| G2 | 待批准请求在 TG 可见并一键批准/拒绝 | TG 收到带 `[允许][拒绝]` 的消息；点击需已有 TOTP 会话 |
| G3 | 无 TG 时也能审批（防 TG 单点） | `aegis --list-contact-requests` / `--approve-contact <id>` / `--reject-contact <id>` |
| G4 | `--tg-simplex` 下自愈可用且不串台 | SimpleX 发码可重钉 `simplex_admin_id`；TG 发码**绝不**改 `simplex_admin_id` |

**核心安全命题**：`autoAccept` 关闭后，公开地址泄露只换来「一条待批准请求」，不再换来一个联系人。

## 1. 事实依据（来自设计 §1/§2，非推断）

- **E1**：关掉 `autoAccept` 后，接受前双方**完全无法通信** ⇒ 审批决策必须换到另一信道（TG 或本地 CLI）。
- **E2**：关掉的确切方式是 `/_address_settings <uid> {"autoAccept":null}`（显式 `null`）。
- **E3**：`/_get chats <uid> pcc=on` 可观察待批准请求。
- **E4**：第二个 WS 客户端（本地 CLI）不影响正在运行的 aegis。
- **API**：`/_accept <contactReqId>`、`/_reject <contactReqId>`（静默）、`ReceivedContactRequest.contactRequest.contactRequestId`。`ContactConnected` **不带** `contactRequestId` ⇒ 必须在 `ReceivedContactRequest` 到达时捕获。

## 2. 设计要点

1. **启动即关 autoAccept**：`BotBuilder::auto_accept_with()` 用于保留/创建地址，但它每次启动都会重新打开 autoAccept；随后必须显式 `configure_address` 关闭。
   - 坑：`AddressSettings.auto_accept` 是 `Option<AutoAccept>` + `skip_serializing_if = "Option::is_none"`，传 `None` 只会**省略**该键（= 不修改）。必须用 `#[serde(flatten)] undocumented: serde_json::Value` 注入显式 `null`。
2. **审批能力进 `BotAdapter`**：新增两个**默认方法**（默认 `bail!`），`SimplexAdapter` 实现，`RoutingAdapter` 转发到 secondary。不改 `AppState::new` 签名（9 个调用点），不改 `MessageEvent`/`CommandEvent` 字段。
3. **TG 通知 + 回调**：SimpleX 事件循环收到 `ReceivedContactRequest` → 用 `state.adapter`（tg-simplex 下即 RoutingAdapter）发到 TG 管理员；回调 data 为 `sx_approve:<contactRequestId>` / `sx_reject:<contactRequestId>`。**授权白送**：回调同样走 `check_auth`，要求 TG 管理员已有 TOTP 会话。
4. **CLI 旁路**：独立进程连 simplex-chat 的 WS（`127.0.0.1:<port>`），用 `_get chats pcc=on` 列请求、`_accept`/`_reject` 操作。不构造 `Bot`（`Bot::init` 会在 `auto_accept=None` 时删除地址）。
5. **平台来源感知的重钉**：`process_auth_code` 已经收到 `adapter`，直接判 `adapter.platform() == Platform::Simplex` 才重钉，无需新增参数、无需改任何调用点。`main.rs` 对 `--simplex` **和** `--tg-simplex` 都开启 `with_simplex_repin()`。

## 3. Scope / Non-goals

**In scope**：G1–G4；相关单测；i18n（zh/en/ja）；设计文档勘误。

**Non-goals**（P3 或另议）：`delete_address` 撤销地址；限时开窗；敲门通知去重/限流；`ContactConnected` 之外的群事件；Matrix 侧任何改动。

## 4. Global Constraints

- **工作树**：新建 `.worktrees/simplex-p2-approval`，分支 `feat/simplex-p2-approval`（基线 `main@36d96a7`）。
- **Rust 目标目录必须复用**：每条 cargo 命令前 `export CARGO_TARGET_DIR=/home/ub/Dark/Wuthering_Waves_Private_Server/rust/aegis/target`。
- **完整质量门（每任务提交前全绿）**：`cargo fmt --check` ｜ `cargo clippy --all-targets --all-features -- -D warnings` ｜ `cargo nextest run --cargo-profile fast-test` ｜ `cargo test --doc`。
- **回归基线**：**945 passed, 1 skipped**（`aa27353` 实测）。预计 +12~+16。
- **不新增依赖**（`serde_json` 已是直接依赖）。
- **不许改动**：`--discord` 拒绝路径；`AppState::new` 签名；`MessageEvent` / `CommandEvent` 字段；`BotAdapter` 既有方法签名；`--tg-simplex` 的 `simplex_admin_id` 硬校验（`main.rs` 适配器装配处）。
- **`bootstrap.rs` 同时编进 lib 与 bin** ⇒ 那里的测试各跑两遍，属预期。
- **CLI 参数名**：设计文档 §3 写 `--approve-contact <contactId>` 系笔误 —— `_accept` 需要的是 `contactRequestId`。本计划一律用 `<contactRequestId>` 并同步勘误设计文档。

---

### Task 1: `autoAccept` 启动即关（保留地址）

**Files**
- Modify: `rust/aegis/src/main/simplex.rs`（`connect_simplex` 102-134、测试模块）

**Interfaces**
- Produces: `settings_with_auto_accept_disabled() -> simploxide_client::prelude::AddressSettings`
- Consumes: 无

- [ ] **Step 1: 失败测试**（追加到 `simplex.rs` 测试模块）

```rust
    /// E2 实证：关闭 autoAccept 必须发**显式** `null`。
    /// `AddressSettings.auto_accept` 是 `Option` 且 `skip_serializing_if = is_none`，
    /// 传 `None` 只会省略该键（不修改），因此靠 flatten 的 `undocumented` 注入 null。
    #[test]
    fn disabled_auto_accept_settings_serialize_explicit_null() {
        let json = serde_json::to_string(&settings_with_auto_accept_disabled()).unwrap();
        assert_eq!(json, r#"{"businessAddress":false,"autoAccept":null}"#);
    }
```

- [ ] **Step 2: 确认失败**：`cargo nextest run --cargo-profile fast-test -E 'test(disabled_auto_accept_settings_serialize_explicit_null)'`（编译失败：函数不存在）。

- [ ] **Step 3: 实现**（`simplex.rs`，`decode_port_and_admin` 之后）

```rust
use simploxide_client::prelude::AddressSettings;

/// 关闭 bot 地址的自动接受。
///
/// `auto_accept_with()` 的目的是「保留/创建地址」，副作用是每次启动都把 autoAccept
/// 重新打开。故必须在 connect 之后显式关闭，否则「永久关闭」只成立于首次安装。
///
/// 用 `undocumented`（flatten 的 `serde_json::Value`）注入**显式** `null`：
/// `AddressSettings.auto_accept = None` 会被 serde 省略该键（= 不修改），而设计 E2
/// 已实证必须 `autoAccept: null` 才真正关闭。
pub(crate) fn settings_with_auto_accept_disabled() -> AddressSettings {
    AddressSettings {
        business_address: false,
        auto_accept: None,
        auto_reply: None,
        undocumented: serde_json::json!({ "autoAccept": null }),
    }
}
```

在 `connect_simplex` 的 `.connect().await...?` 之后、读取地址之前插入：

```rust
    // P2：autoAccept 永久关闭。失败即启动失败（fail closed）—— 这个控制是
    // 「地址泄露近乎无用」的全部依据，静默继续等于把公开地址留在自动接受状态。
    bot.configure_address(settings_with_auto_accept_disabled())
        .await
        .map_err(|e| anyhow::anyhow!("关闭 SimpleX autoAccept 失败，拒绝以不安全状态启动: {e}"))?;
```

- [ ] **Step 4: 门禁 + 提交**（`cargo fmt --check; clippy; nextest`）。预期 945 → 946。

---

### Task 2: 审批能力进 `BotAdapter`（默认 + Simplex 实现 + Routing 转发）

**Files**
- Modify: `rust/aegis/src/common/trait.rs`（trait 101-177）
- Modify: `rust/aegis/src/gateways/simplex/adapter.rs`（impl 178+）
- Modify: `rust/aegis/src/common/routing.rs`（impl 50+）

**Interfaces**
- Produces: `BotAdapter::accept_contact_request(&self, i64) -> Result<()>`、`reject_contact_request(&self, i64) -> Result<()>`（默认 `bail!`）

- [ ] **Step 1: 失败测试**
  - `common/routing.rs` 测试模块：用 `MockBotAdapter` 断言 secondary 被调用、无 secondary 时报错。
  - `gateways/simplex/adapter.rs`：扩展现有类型级断言 `simplex_adapter_implements_bot_adapter`（已有，无需改；两个新方法由默认实现满足）。新增一个 `RoutingAdapter` 转发测试即可。

```rust
    #[tokio::test]
    async fn routing_forwards_contact_approval_to_secondary() {
        use crate::common::MockBotAdapter;
        use std::sync::Arc;
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        let mut secondary = MockBotAdapter::new();
        secondary
            .expect_accept_contact_request()
            .with(mockall::predicate::eq(5))
            .times(1)
            .returning(|_| Ok(()));
        let adapter = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
        adapter.accept_contact_request(5).await.unwrap();
    }

    #[tokio::test]
    async fn routing_contact_approval_without_secondary_errors() {
        use crate::common::MockBotAdapter;
        use std::sync::Arc;
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        let adapter = RoutingAdapter::new(Arc::new(primary), None);
        assert!(adapter.accept_contact_request(5).await.is_err());
    }
```

- [ ] **Step 2: 确认失败**（编译失败：方法不存在）。

- [ ] **Step 3: 实现**

`common/trait.rs`，在 `send_reaction` 之后：

```rust
    /// 批准一个 SimpleX 联系人请求（TG 带外审批 / 本地 CLI 用）。
    /// 默认不支持：只有 SimpleX 适配器实现；RoutingAdapter 转发到 secondary。
    async fn accept_contact_request(&self, _contact_request_id: i64) -> Result<()> {
        anyhow::bail!("当前平台不支持联系人审批")
    }

    /// 拒绝一个 SimpleX 联系人请求（对请求方静默）。
    async fn reject_contact_request(&self, _contact_request_id: i64) -> Result<()> {
        anyhow::bail!("当前平台不支持联系人审批")
    }
```

`gateways/simplex/adapter.rs`，`impl BotAdapter for SimplexAdapter` 内（`send_reaction` 附近），并补 `use simploxide_client::prelude::ContactRequestId;`：

```rust
    async fn accept_contact_request(&self, contact_request_id: i64) -> Result<()> {
        let crid = ContactRequestId::try_from(contact_request_id)
            .map_err(|_| anyhow::anyhow!("contactRequestId 必须为正整数: {contact_request_id}"))?;
        self.bot
            .accept_contact(crid)
            .await
            .context("接受 SimpleX 联系人请求失败")?;
        Ok(())
    }

    async fn reject_contact_request(&self, contact_request_id: i64) -> Result<()> {
        let crid = ContactRequestId::try_from(contact_request_id)
            .map_err(|_| anyhow::anyhow!("contactRequestId 必须为正整数: {contact_request_id}"))?;
        self.bot
            .reject_contact(crid)
            .await
            .context("拒绝 SimpleX 联系人请求失败")?;
        Ok(())
    }
```

`common/routing.rs`，`impl BotAdapter for RoutingAdapter` 内：

```rust
    async fn accept_contact_request(&self, contact_request_id: i64) -> Result<()> {
        match &self.secondary {
            Some(secondary) => secondary.accept_contact_request(contact_request_id).await,
            None => anyhow::bail!("未配置 SimpleX 次级适配器，无法审批联系人"),
        }
    }

    async fn reject_contact_request(&self, contact_request_id: i64) -> Result<()> {
        match &self.secondary {
            Some(secondary) => secondary.reject_contact_request(contact_request_id).await,
            None => anyhow::bail!("未配置 SimpleX 次级适配器，无法审批联系人"),
        }
    }
```

- [ ] **Step 4: 门禁 + 提交**。预期 946 → 948。

---

### Task 3: `ReceivedContactRequest` → TG 通知（带按钮）

**Files**
- Modify: `rust/aegis/src/main/runtime.rs`（事件匹配 286-302、纯函数区 538+、测试）
- Modify: `resources/i18n/{zh,en,ja}.yml`（`simplex:` 段）

**Interfaces**
- Produces: `contact_request_notification(i64, &str) -> (String, Markup)`

- [ ] **Step 1: 失败测试**（`runtime.rs` 测试模块）

```rust
    #[test]
    fn contact_request_notification_has_approve_and_reject_buttons() {
        let (text, markup) = contact_request_notification(5, "alice");
        assert!(text.contains('5'), "{text}");
        assert!(text.contains("alice"), "{text}");
        let datas: Vec<&str> = markup
            .buttons
            .iter()
            .flatten()
            .map(|b| b.data.as_str())
            .collect();
        assert_eq!(datas, vec!["sx_approve:5", "sx_reject:5"]);
    }

    /// 显示名由陌生人自设：换行/控制字符不得污染通知（日志/消息注入）。
    #[test]
    fn contact_request_notification_escapes_display_name() {
        let (text, _) = contact_request_notification(7, "evil\nINJECTED");
        assert!(!text.contains('\n'), "换行必须被转义: {text}");
    }
```

- [ ] **Step 2: 确认失败**。

- [ ] **Step 3: 实现**

`resources/i18n/zh.yml`（`simplex:` 段追加）：

```yaml
  contact_request: "🔔 SimpleX 有人请求连接\n显示名: %{0}\n请求 ID: %{1}"
  approve: "✅ 允许"
  reject: "🚫 拒绝"
```

`en.yml` / `ja.yml` 同步（`contact_request` / `approve` / `reject`）。

`runtime.rs` 顶部 import 扩展为：

```rust
use aegis::common::{InlineButton, Markup, MessageContent, MessageId, TargetId};
```

事件匹配里，在 `ContactConnected` 分支之后插入（注意：`continue` 语义与 `NewChatItems` 分支一致）：

```rust
                    // P2：autoAccept 已关闭，敲门变成 ReceivedContactRequest（不再是
                    // ContactConnected）。此事件**必须在此刻捕获** —— 它携带的
                    // contactRequestId 是 _accept/_reject 的唯一凭据，接受后无法回溯。
                    simploxide_client::events::Event::ReceivedContactRequest(ev) => {
                        let req = &ev.contact_request;
                        log::info!(
                            "SimpleX 待批准联系人请求: contactRequestId={} display_name={:?}",
                            req.contact_request_id,
                            req.local_display_name
                        );
                        if enable_telegram
                            && let Some(tg_admin) = admin_id
                        {
                            let (text, markup) = contact_request_notification(
                                req.contact_request_id,
                                &req.local_display_name,
                            );
                            let target = TargetId(tg_admin.to_string());
                            if let Err(e) = state_for_events
                                .adapter
                                .send_message(
                                    &target,
                                    MessageContent {
                                        text,
                                        markup: Some(markup),
                                    },
                                )
                                .await
                            {
                                log::error!("向 Telegram 推送待批准联系人请求失败: {e}");
                            }
                        }
                        continue;
                    }
```

纯函数（放在 `contact_connected_log_line` 旁）：

```rust
/// 组装「有人敲门」的 TG 通知。回调 data 携带的是 `contactRequestId`
/// （`_accept` / `_reject` 需要的 ID），不是 contactId。
/// 显示名由陌生人自设，用 `{:?}` 风格加引号使其无法伪造可信文案。
fn contact_request_notification(contact_request_id: i64, display_name: &str) -> (String, Markup) {
    let text = rust_i18n::t!(
        "simplex.contact_request",
        "0" => format!("{display_name:?}"),
        "1" => contact_request_id.to_string()
    )
    .into_owned();
    let markup = Markup {
        buttons: vec![vec![
            InlineButton {
                text: rust_i18n::t!("simplex.approve").into_owned(),
                data: format!("sx_approve:{contact_request_id}"),
            },
            InlineButton {
                text: rust_i18n::t!("simplex.reject").into_owned(),
                data: format!("sx_reject:{contact_request_id}"),
            },
        ]],
    };
    (text, markup)
}
```

> `display_name` 用 `{display_name:?}` 引号化，使嵌入换行以 `\n` 字面出现（测试断言 `!text.contains('\n')` 因此成立）。

- [ ] **Step 4: 门禁 + 提交**。预期 948 → 950。

---

### Task 4: 回调处理（批准 / 拒绝）

**Files**
- Add: `rust/aegis/src/shared/handlers/approval.rs`
- Modify: `rust/aegis/src/shared/handlers/mod.rs`（`CallbackRoute` 8-16、`route_callback` 46-122、`dispatch` 124-137、测试）
- Modify: `resources/i18n/{zh,en,ja}.yml`

**Interfaces**
- Consumes: `BotAdapter::{accept,reject}_contact_request`
- Produces: `CallbackRoute::Approval`

- [ ] **Step 1: 失败测试**
  - `handlers/mod.rs`：`route_callback("sx_approve:5") == Some(Approval)`、`route_callback("sx_reject:5") == Some(Approval)`、`route_callback("sx_approve") == None`。
  - `handlers/approval.rs`：`MockBotAdapter` 断言 `sx_approve:7` 调用 `accept_contact_request(7)`；`sx_reject:7` 调用 reject；失败时发错误消息且不 panic。

```rust
    #[tokio::test]
    async fn approve_callback_calls_adapter() {
        use crate::common::{MessageId, MockBotAdapter, Platform};
        use std::sync::Arc;
        let mut mock = MockBotAdapter::new();
        mock.expect_platform().returning(|| Platform::Telegram);
        mock.expect_accept_contact_request()
            .with(mockall::predicate::eq(7))
            .times(1)
            .returning(|_| Ok(()));
        mock.expect_answer_callback().returning(|_, _, _| Ok(()));
        mock.expect_send_message()
            .returning(|_, _| Ok(MessageId("1".into())));
        let event = CallbackEvent {
            adapter: Arc::new(mock),
            target: TargetId("42".into()),
            user_id: "42".into(),
            msg_id: MessageId("1".into()),
            data: "sx_approve:7".into(),
            callback_id: "cb1".into(),
            session_timeout_secs: 600,
        };
        assert_eq!(handle(&event).await.unwrap(), crate::shared::types::HandlerAction::Done);
    }
```

- [ ] **Step 2: 确认失败**。

- [ ] **Step 3: 实现**

`handlers/approval.rs`（新文件）：

```rust
use crate::common::MessageContent;
use crate::shared::types::{CallbackEvent, HandlerAction, HandlerResult};
use rust_i18n::t;

/// TG 带外审批回调：`sx_approve:<contactRequestId>` / `sx_reject:<contactRequestId>`。
///
/// 授权由 `check_auth` 提前承担：回调与命令同路，要求 Telegram 管理员已有活跃 TOTP
/// 会话（见 `shared/dispatch.rs`）。因此「点一下即批准」= 已经需要 TOTP，无需新机制。
pub async fn handle(event: &CallbackEvent) -> HandlerResult {
    let (approve, raw_id) = match event.data.split_once(':') {
        Some(("sx_approve", id)) => (true, id),
        Some(("sx_reject", id)) => (false, id),
        _ => return Ok(HandlerAction::Done),
    };
    let Ok(id) = raw_id.parse::<i64>() else {
        event
            .adapter
            .answer_callback(
                &event.target,
                &event.callback_id,
                Some(t!("simplex.bad_request_id").into_owned()),
            )
            .await?;
        return Ok(HandlerAction::Done);
    };

    let result = if approve {
        event.adapter.accept_contact_request(id).await
    } else {
        event.adapter.reject_contact_request(id).await
    };

    let (text, answer) = match result {
        Ok(()) => (
            t!("simplex.approval_done", "0" => id.to_string()).into_owned(),
            if approve {
                t!("simplex.approved").into_owned()
            } else {
                t!("simplex.rejected").into_owned()
            },
        ),
        Err(e) => {
            log::error!("SimpleX 联系人审批失败 (id={id}, approve={approve}): {e}");
            (
                t!("simplex.approval_failed", "0" => e.to_string()).into_owned(),
                t!("simplex.approval_failed_short").into_owned(),
            )
        }
    };

    event
        .adapter
        .answer_callback(&event.target, &event.callback_id, Some(answer))
        .await?;
    event
        .adapter
        .send_message(
            &event.target,
            MessageContent {
                text,
                markup: None,
            },
        )
        .await?;
    Ok(HandlerAction::Done)
}
```

`handlers/mod.rs`：`pub mod approval;`；`CallbackRoute` 加 `Approval`；`route_callback` 最前面加：

```rust
    if data.starts_with("sx_approve:") || data.starts_with("sx_reject:") {
        return Some(CallbackRoute::Approval);
    }
```

`dispatch` 加：

```rust
        Some(CallbackRoute::Approval) => Ok(Some(approval::handle(event).await?)),
```

i18n 追加（zh；en/ja 同步）：

```yaml
  approval_done: "✅ 已处理联系人请求 #%{0}"
  approved: "已允许"
  rejected: "已拒绝"
  approval_failed: "❌ 处理失败: %{0}"
  approval_failed_short: "处理失败"
  bad_request_id: "无效的请求 ID"
```

- [ ] **Step 4: 门禁 + 提交**。预期 950 → 954。

---

### Task 5: 平台来源感知的重钉（`--tg-simplex` 自愈）

**Files**
- Modify: `rust/aegis/src/app/auth.rs`（`process_auth_code` 34-55、测试）
- Modify: `rust/aegis/src/main.rs`（140-144 与其上方注释）
- Modify: `rust/aegis/src/app/state.rs`（`with_simplex_repin` 注释，可选）

**Interfaces**
- 无签名变化：`process_auth_code` 已有的 `adapter` 参数即为平台来源。

- [ ] **Step 1: 失败测试**（`app/auth.rs` 测试模块）

```rust
    struct TelegramRecordingAdapter(aegis::common::Platform);

    #[async_trait::async_trait]
    impl BotAdapter for TelegramRecordingAdapter {
        fn platform(&self) -> aegis::common::Platform {
            self.0
        }
        async fn send_message(&self, _t: &TargetId, _c: MessageContent) -> anyhow::Result<MessageId> {
            Ok(MessageId("0".into()))
        }
        async fn edit_message(&self, _t: &TargetId, _m: &MessageId, _c: MessageContent) -> anyhow::Result<()> { Ok(()) }
        async fn delete_message(&self, _t: &TargetId, _m: &MessageId) -> anyhow::Result<()> { Ok(()) }
        fn capabilities(&self) -> aegis::common::PlatformCapabilities {
            aegis::common::PlatformCapabilities::TELEGRAM
        }
    }

    /// TG 发来的码绝不改 simplex_admin_id —— 否则敏感内容落点会被覆写成 TG chat id。
    #[tokio::test]
    async fn telegram_code_does_not_repin_simplex_admin() {
        let (state, _dir) = state_with_repin(true);
        let code = state.generate_current_totp().expect("有 TOTP 管理器");
        let adapter = TelegramRecordingAdapter(aegis::common::Platform::Telegram);
        let ok = process_auth_code(
            &adapter,
            &TargetId("42".into()),
            42,
            &code,
            &state,
            5,
            Duration::from_secs(600),
            &[Duration::from_secs(900)],
        )
        .await
        .unwrap();
        assert!(ok, "TG 验证仍应成功建会话");
        assert_eq!(state.simplex_admin_id(), Some(3), "TG 不得重钉 SimpleX 管理员");
    }
```

- [ ] **Step 2: 确认失败**：当前实现会重钉为 42。

- [ ] **Step 3: 实现**

`app/auth.rs` 顶部补 `use aegis::common::Platform;`（或 `aegis::common::{..., Platform}`），并把重钉条件改为：

```rust
        // 自愈：只有**来自 SimpleX** 的 TOTP 成功才重钉 simplex_admin_id。
        // `is_admin_user` 的 user_id 命名空间与 Telegram 共用，若不加平台判据，
        // `--tg-simplex` 下 TG 管理员的码会把 simplex_admin_id 覆写成 TG chat id，
        // 破坏「敏感内容落点」。平台判据取自事件自身的 adapter（无需改调用点）。
        if state.simplex_repin_enabled() && adapter.platform() == Platform::Simplex {
            // ...（原重钉块不变）
        }
```

`main.rs` 140-144 改为：

```rust
    // 自愈开关：两种 SimpleX 形态都开启；平台串台由 `process_auth_code` 内的
    // `adapter.platform() == Platform::Simplex` 判据兜住（TG 的码不会重钉）。
    let state = if selection.simplex {
        state.with_simplex_repin()
    } else {
        state
    };
```

`state.rs`：更新 `with_simplex_repin` 的文档注释，去掉「只允许纯 --simplex」，改为「重钉只对 SimpleX 来源生效（由 auth.rs 判据保证）」。不新增单测（纯注释）。

- [ ] **Step 4: 门禁 + 提交**。预期 954 → 955。

---

### Task 6: 本地 CLI 旁路

**Files**
- Modify: `rust/aegis/src/main/simplex.rs`（新增 CLI 客户端辅助 + `decode_port_and_admin` 改 `pub(crate)`）
- Modify: `rust/aegis/src/main/cli.rs`（`CliMode` 6-25、`try_cli_mode` 27-53、`execute_cli_mode` 55-95、unknown 文案、测试）

**Interfaces**
- Produces:
  - `pub(crate) struct PendingContactRequest { pub contact_request_id: i64, pub display_name: String }`
  - `pub(crate) async fn list_pending_contact_requests() -> Result<Vec<PendingContactRequest>>`
  - `pub(crate) async fn approve_contact_request(i64) -> Result<()>`
  - `pub(crate) async fn reject_contact_request(i64) -> Result<()>`
  - `CliMode::{ListContactRequests, ApproveContact(Option<String>), RejectContact(Option<String>)}`

- [ ] **Step 1: 失败测试**（`cli.rs`）

```rust
    #[test]
    fn recognises_contact_approval_flags() {
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--list-contact-requests"])),
            Some(CliMode::ListContactRequests)
        ));
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--approve-contact", "7"])),
            Some(CliMode::ApproveContact(Some(v))) if v == "7"
        ));
        // 缺参数仍须返回 Some（否则 main 会继续正常启动 bot）
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--approve-contact"])),
            Some(CliMode::ApproveContact(None))
        ));
        assert!(matches!(
            try_cli_mode(&args(&["aegis", "--reject-contact", "7"])),
            Some(CliMode::RejectContact(Some(v))) if v == "7"
        ));
    }
```

- [ ] **Step 2: 确认失败**。

- [ ] **Step 3: 实现**

`main/simplex.rs`：`decode_port_and_admin` 改 `pub(crate) fn`，并追加：

```rust
use simploxide_client::prelude::{
    ApiGetChats, ChatInfo, ChatListQuery, ClientApi as _, ClientApiExt as _, PaginationByTime,
};

pub(crate) struct PendingContactRequest {
    pub contact_request_id: i64,
    pub display_name: String,
}

/// CLI 旁路专用：连正在运行的 simplex-chat WS（第二个客户端，E4 实证无影响）。
/// 不构造 `Bot` —— `Bot::init` 在 auto_accept=None 时会**删除地址**。
async fn cli_client() -> Result<simploxide_client::ws::Client> {
    let (app_config, security) = crate::main::config::load_and_validate()?;
    let (port, _) = decode_port_and_admin(&security, &app_config.decrypted.encrypted_config)?;
    let (client, _events) = simploxide_client::ws::connect(format!("ws://127.0.0.1:{port}"))
        .await
        .map_err(|e| anyhow::anyhow!("连接 simplex-chat (127.0.0.1:{port}) 失败: {e}"))?;
    Ok(client)
}

async fn active_user_id(client: &simploxide_client::ws::Client) -> Result<i64> {
    client
        .users()
        .await?
        .into_iter()
        .find(|u| u.user.active_user)
        .map(|u| u.user.user_id)
        .context("找不到活跃的 SimpleX 用户")
}

pub(crate) async fn list_pending_contact_requests() -> Result<Vec<PendingContactRequest>> {
    let client = cli_client().await?;
    let user_id = active_user_id(&client).await?;
    let resp = client
        .api_get_chats(ApiGetChats {
            user_id,
            pending_connections: true,
            pagination: PaginationByTime::make_last(100),
            query: ChatListQuery::make_filters(false, false),
        })
        .await
        .context("读取待批准联系人请求失败")?;
    let mut out = Vec::new();
    for chat in &resp.chats {
        if let ChatInfo::ContactRequest { contact_request, .. } = &chat.chat_info {
            out.push(PendingContactRequest {
                contact_request_id: contact_request.contact_request_id,
                display_name: contact_request.local_display_name.clone(),
            });
        }
    }
    Ok(out)
}

fn contact_request_id(raw: i64) -> Result<simploxide_client::prelude::ContactRequestId> {
    simploxide_client::prelude::ContactRequestId::try_from(raw)
        .map_err(|_| anyhow::anyhow!("contactRequestId 必须为正整数: {raw}"))
}

pub(crate) async fn approve_contact_request(id: i64) -> Result<()> {
    let client = cli_client().await?;
    client
        .accept_contact(contact_request_id(id)?)
        .await
        .context("接受联系人请求失败")?;
    Ok(())
}

pub(crate) async fn reject_contact_request(id: i64) -> Result<()> {
    let client = cli_client().await?;
    client
        .reject_contact(contact_request_id(id)?)
        .await
        .context("拒绝联系人请求失败")?;
    Ok(())
}
```

`cli.rs`：`CliMode` 加三个变体；`try_cli_mode` 加分支：

```rust
        "--list-contact-requests" => Some(CliMode::ListContactRequests),
        "--approve-contact" => Some(CliMode::ApproveContact(args.get(2).cloned())),
        "--reject-contact" => Some(CliMode::RejectContact(args.get(2).cloned())),
```

`execute_cli_mode` 加：

```rust
        CliMode::ListContactRequests => {
            let pending = crate::main::simplex::list_pending_contact_requests().await?;
            if pending.is_empty() {
                println!("没有待批准的联系人请求");
            }
            for p in pending {
                println!(
                    "contactRequestId={} display_name={:?}",
                    p.contact_request_id, p.display_name
                );
            }
            Ok(())
        }
        CliMode::ApproveContact(raw) => {
            let raw = raw.context("用法: aegis --approve-contact <contactRequestId>")?;
            let id = parse_contact_id(&raw)?;
            crate::main::simplex::approve_contact_request(id).await?;
            println!("已批准联系人请求 contactRequestId={id}");
            Ok(())
        }
        CliMode::RejectContact(raw) => {
            let raw = raw.context("用法: aegis --reject-contact <contactRequestId>")?;
            let id = parse_contact_id(&raw)?;
            crate::main::simplex::reject_contact_request(id).await?;
            println!("已拒绝联系人请求 contactRequestId={id}");
            Ok(())
        }
```

更新 `CliMode::Unknown` 的可用参数文案，加入三个新 flag。

- [ ] **Step 4: 门禁 + 提交**。预期 955 → 956。

---

### Task 7: 接线检查 + 全量门禁 + 文档

**Files**
- Modify: `docs/superpowers/specs/2026-09-18-simplex-approval-selfheal-design.md`（§3 CLI 参数勘误、§6 P2 状态）

- [ ] **Step 1**: 全量门禁（见 Global Constraints），确认零诊断、全绿。
- [ ] **Step 2**: 设计文档勘误：
  - `--approve-contact <contactId>` → `<contactRequestId>`，并注明与 `_accept` 的对应。
  - §6 标记「P2 已实施」。
  - 若采纳本计划 Task 5，删除 P1 已知边界 #1 中「留待 P2」的表述或改为「P2 已解决」。
- [ ] **Step 3**: 原子提交（Conventional Commits，中文正文）。

## 5. 验证

### 5.1 自动化

| 门 | 断言 |
| --- | --- |
| 单测 | Task 1 的 JSON 精确匹配；Task 2 转发/无 secondary；Task 3 按钮 data 与转义；Task 4 回调→adapter；Task 5 TG 不重钉 / SimpleX 重钉 |
| 静态 | `fmt --check` ｜ `clippy -D warnings` ｜ `cargo test --doc` |
| 回归 | `nextest --cargo-profile fast-test` ≥ 945 passed, 1 skipped |

### 5.2 真实主机（控制器执行）

```bash
# 1) autoAccept 已关（地址不变）
ssh <host> 'curl -s ... _show_address'   # 或经 WS：/_show_address → 无 autoAccept 字段
# 2) 陌生人连接 → TG 收到带按钮消息；未登录 TOTP 时点按钮无任何效果（check_auth 拒绝）
# 3) 登录 TOTP 后点「允许」→ 对方成为联系人，但发普通消息仍被门禁①丢弃（连接 ≠ 管理员）
# 4) 本地旁路（不需 TG）
aegis --list-contact-requests
aegis --approve-contact <contactRequestId>
aegis --reject-contact  <contactRequestId>
# 5) --simplex：关 autoAccept → CLI 批准自己 → 发 6 位码 → journal 出现「管理员已重钉」
# 6) --tg-simplex：TG 点批准 → 在 SimpleX 发 6 位码 → simplex_admin_id 重钉；
#    再在 TG 发 6 位码 → simplex_admin_id 不变
```

## 6. 已知边界（写进 PR，不得隐去）

1. **启动瞬时窗口**：`BotBuilder::auto_accept_with()` 在 connect 期间会先打开 autoAccept，我们随后才关闭 —— 窗口为两个 WS 往返（毫秒级）。彻底消除需修改/替换 `simploxide-client` 的 init 语义，超出本计划。
2. **通知可被刷**：陌生人可反复发起请求 → TG 消息风暴（TG Bot API 约 30 msg/s）。本计划不去重、不限流；如需，另开任务（按 `contactRequestId` 去重或令牌桶）。
3. **CLI 依赖 simplex-chat 在跑**：`--list/--approve/--reject` 经 WS 端口操作，服务未运行时给出连接错误。
4. **`--simplex` 仍无 TG 提示**：纯 CLI，符合设计；限时开窗留待 P3。
5. **拒绝静默**：请求方不会收到任何通知（SimpleX API 语义，非本实现选择）。

## 7. 待你裁决（不反对即按默认实施）

| # | 决策 | 默认值 |
| --- | --- | --- |
| D-P2-1 | 关闭 autoAccept 失败时 | **硬失败**（拒绝以不安全状态启动） |
| D-P2-2 | 是否一并打通 `--tg-simplex` 自愈（Task 5） | **是**（平台来源判据，符合设计 D4） |
| D-P2-3 | 敲门通知去重/限流 | **否**（列为已知边界，另开任务） |
| D-P2-4 | CLI 参数名 | `--approve-contact <contactRequestId>`（勘误设计文档） |
| D-P2-5 | 执行方式 | subagent-driven-development，每任务独立 implementer + reviewer（同 P1） |
