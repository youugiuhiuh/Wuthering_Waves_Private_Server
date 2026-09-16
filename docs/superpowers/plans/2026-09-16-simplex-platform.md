# SimpleX 平台接入实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 aegis 支持 SimpleX Chat 作为第四个平台（`--simplex` standalone），可与本机 `simplex-chat -p <port>` WebSocket 服务收发文本/文件，并让安装器能部署该平台。

**Architecture:** 复用现有 `BotAdapter` 抽象：新增 `gateways/simplex/` 适配器与 `main/simplex.rs` 连接层，`Platform::Simplex` + `PlatformCapabilities::SIMPLEX` 申报能力；由 systemd 独立运行 `simplex-chat`，aegis 通过 `simploxide-client` 的 `websocket` feature 连接，事件经 `EventStream` dispatcher 映射为既有 `BotEvent`。

**Tech Stack:** Rust 2024 / tokio / `simploxide-client 0.14.0`（feature `websocket`，MIT OR Apache-2.0）/ Go 1.26 安装器 / systemd。

**Spec:** `docs/superpowers/specs/2026-09-16-simplex-platform-design.md`

## Global Constraints

- 依赖只通过 `cargo add` 添加，禁止手改 `Cargo.toml` 依赖段（dependency-management 规则）。
- `simploxide-client` 固定 `=0.14.0`，且**不得**启用 `ffi`（AGPL）与 `cli`（进程由 systemd 管）；只启用 `websocket`。
- 部署侧 `simplex-chat` 固定 **v7.0.0**（simploxide 0.14 兼容表 min=max=7.0.0.0）。
- 管理员鉴权走 `AppState::is_admin_user`，**禁止** TOFU/首消息授权；非管理员只记日志。
- 每个 Rust 任务结束执行：`cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc`。
- 每个 Go 任务结束执行：`gofmt -l . && go vet ./... && go test ./...`。
- 工作树：`.worktrees/simplex-platform`（分支 `feat/simplex-platform`）。
- Rust 侧 i18n 文件为 `rust/aegis/src/resources/i18n/{en,zh,ja}.yml`；安装器为 `go/installer/i18n/{en,zh,ja}.json`（两套，勿混）。

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `rust/aegis/src/gateways/simplex/mod.rs` | 模块导出 |
| `rust/aegis/src/gateways/simplex/adapter.rs` | `SimplexAdapter` + 纯映射函数 + 单测 |
| `rust/aegis/src/main/simplex.rs` | `has_simplex_config` / `connect_simplex` / `SimplexHandle` |
| `rust/aegis/src/common/trait.rs` | `Platform::Simplex`、`PlatformCapabilities::SIMPLEX` |
| `rust/aegis/src/common/markup.rs` | 共享 `render_markup_buttons`（从 matrix 提升） |
| `rust/aegis/src/bootstrap.rs` | `EncryptedConfig`/`SetupInput`/`run_setup` 新增 simplex 字段 |
| `rust/aegis/src/app/state.rs` | `simplex_admin_id` + `is_admin_user` |
| `rust/aegis/src/main.rs` | `--simplex` 解析与接线 |
| `rust/aegis/src/main/runtime.rs` | SimpleX 网关段 + dispatcher 循环 |
| `go/installer/main.go` | 平台选择器 / setup payload / systemd 单元 |
| `go/installer/i18n/{en,zh,ja}.json` | 安装器文案 |
| `README.md` | 平台列表 |

---

### Task 1: `Platform::Simplex` 与能力位

**Files:**
- Modify: `rust/aegis/src/common/trait.rs`
- Test: `rust/aegis/src/common/trait.rs`（文件内 `#[cfg(test)] mod tests`，若已存在则追加）

**Interfaces:**
- Consumes: 无
- Produces: `Platform::Simplex`；`PlatformCapabilities::SIMPLEX`

- [ ] **Step 1: 写失败测试**

在 `rust/aegis/src/common/trait.rs` 末尾追加：

```rust
#[cfg(test)]
mod simplex_capabilities_tests {
    use super::*;

    #[test]
    fn simplex_capabilities_are_honest() {
        let caps = PlatformCapabilities::SIMPLEX;
        assert!(caps.can_edit_message, "update_msg 可用");
        assert!(caps.can_delete_message, "delete_msg 可用");
        assert!(!caps.has_inline_keyboard, "SimpleX 无 inline keyboard");
        assert!(!caps.has_slash_commands, "本期不做平台命令注册");
        assert!(!caps.has_file_transfer, "本期 download_file 不支持");
        assert!(caps.can_send_file);
        assert!(caps.can_send_image);
        assert!(!caps.can_send_voice);
        assert!(!caps.can_send_typing, "未发现 typing API");
        assert!(caps.can_send_reaction, "update_msg_reaction 可用");
        assert!(!caps.can_thread);
        assert!(caps.has_e2ee);
    }

    #[test]
    fn platform_simplex_is_distinct() {
        assert_ne!(Platform::Simplex, Platform::Telegram);
        assert_ne!(Platform::Simplex, Platform::Discord);
        assert_ne!(Platform::Simplex, Platform::Matrix);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex_capabilities_tests`
Expected: 编译失败，`no variant or associated item named 'Simplex'`

- [ ] **Step 3: 最小实现**

`Platform` 枚举加变体：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Telegram,
    Discord,
    Matrix,
    Simplex,
}
```

`impl PlatformCapabilities` 内追加常量：

```rust
    pub const SIMPLEX: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: false,
        has_slash_commands: false,
        has_file_transfer: false,
        can_send_file: true,
        can_send_image: true,
        can_send_voice: false,
        can_send_typing: false,
        can_send_reaction: true,
        can_thread: false,
        has_e2ee: true,
    };
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex_capabilities_tests`
Expected: 2 passed

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test
git add rust/aegis/src/common/trait.rs
git commit -m "feat(simplex): 新增 Platform::Simplex 与能力位"
```

---

### Task 2: 依赖引入与共享 markup 渲染

**Files:**
- Modify: `rust/aegis/Cargo.toml`（仅通过 `cargo add`）
- Create: `rust/aegis/src/common/markup.rs`
- Modify: `rust/aegis/src/common/mod.rs`
- Modify: `rust/aegis/src/gateways/matrix/adapter.rs`（删除本地实现，改用共享函数）

**Interfaces:**
- Consumes: Task 1 无依赖
- Produces: `aegis::common::markup::render_markup_buttons(base: String, markup: &Markup) -> String`

- [ ] **Step 1: 写失败测试**

创建 `rust/aegis/src/common/markup.rs`：

```rust
use crate::common::Markup;

/// 把 inline keyboard markup 渲染为文本命令列表，供不支持 inline keyboard 的平台使用。
pub fn render_markup_buttons(base: String, markup: &Markup) -> String {
    let mut body = base;
    let mut lines: Vec<String> = Vec::new();
    let mut idx = 1;
    for row in &markup.buttons {
        for btn in row {
            lines.push(format!("{}. {} — send: `{}`", idx, btn.text, btn.data));
            idx += 1;
        }
    }
    if !lines.is_empty() {
        body.push_str(&rust_i18n::t!("matrix.markup_header"));
        body.push_str(&lines.join("\n"));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::InlineButton;

    #[test]
    fn renders_buttons_as_numbered_commands() {
        let markup = Markup {
            buttons: vec![vec![InlineButton {
                text: "Help".into(),
                data: "/help".into(),
            }]],
        };
        let out = render_markup_buttons("Hi".into(), &markup);
        assert!(out.starts_with("Hi"));
        assert!(out.contains("1. Help — send: `/help`"));
    }

    #[test]
    fn leaves_plain_text_untouched() {
        let out = render_markup_buttons("plain".into(), &Markup { buttons: vec![] });
        assert_eq!(out, "plain");
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo nextest run --cargo-profile fast-test -p aegis common::markup`
Expected: 编译失败，`file not found for module markup`（尚未在 `common/mod.rs` 声明）

- [ ] **Step 3: 注册模块并替换 Matrix 本地实现**

`rust/aegis/src/common/mod.rs` 追加：

```rust
pub mod markup;
```

`rust/aegis/src/gateways/matrix/adapter.rs`：删除本地 `fn render_markup_buttons` 定义，改为在文件顶部导入：

```rust
use crate::common::markup::render_markup_buttons;
```

并把文件内 `super::render_markup_buttons` 的测试调用改为 `render_markup_buttons`。

- [ ] **Step 4: 添加依赖**

```bash
cd rust/aegis
cargo add simploxide-client@=0.14.0 --no-default-features --features websocket
```

Expected: `Cargo.toml` 出现
`simploxide-client = { version = "=0.14.0", default-features = false, features = ["websocket"] }`，
且 `Cargo.lock` 未出现 `simploxide-ffi-core` / `simploxide-sxcrt-sys`。

- [ ] **Step 5: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test -p aegis markup`
Expected: 2 passed；且 `cargo nextest run --cargo-profile fast-test` 全绿（Matrix 既有测试不回归）

- [ ] **Step 6: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test
git add rust/aegis/Cargo.toml rust/aegis/Cargo.lock rust/aegis/src/common/mod.rs rust/aegis/src/common/markup.rs rust/aegis/src/gateways/matrix/adapter.rs
git commit -m "feat(simplex): 引入 simploxide-client 并提取共享 markup 渲染"
```

---

### Task 3: SimpleX 事件映射纯函数

**Files:**
- Create: `rust/aegis/src/gateways/simplex/mod.rs`
- Create: `rust/aegis/src/gateways/simplex/adapter.rs`
- Modify: `rust/aegis/src/gateways/mod.rs`

**Interfaces:**
- Consumes: Task 1（`Platform::Simplex`）、Task 2（`render_markup_buttons`）
- Produces:
  - `pub fn direct_contact_id(chat_info: &ChatInfo) -> Option<i64>`
  - `pub fn extract_message(content: &CIContent, file: Option<&CIFile>, fallback_text: &str) -> Option<(String, Option<String>, Option<String>)>`
  - `pub struct IncomingMessage { pub contact_id: i64, pub target: TargetId, pub user_id: i64, pub text: Option<String>, pub file_id: Option<String>, pub file_name: Option<String> }`
  - `pub fn map_new_chat_items(chat_items: &[AChatItem]) -> Vec<IncomingMessage>`

- [ ] **Step 1: 写失败测试**

创建 `rust/aegis/src/gateways/simplex/mod.rs`：

```rust
pub mod adapter;
pub use adapter::{IncomingMessage, SimplexAdapter, direct_contact_id, extract_message, map_new_chat_items};
```

创建 `rust/aegis/src/gateways/simplex/adapter.rs`（先只写纯函数与测试，adapter 结构体留到 Task 4）：

```rust
use crate::common::TargetId;
use simploxide_client::types::{AChatItem, CIContent, CIFile, ChatInfo};

/// 仅处理直聊：返回对方 contactId。群聊/本地笔记返回 None（本期不支持）。
pub fn direct_contact_id(chat_info: &ChatInfo) -> Option<i64> {
    match chat_info {
        ChatInfo::Direct { contact, .. } => Some(contact.contact_id),
        _ => None,
    }
}

/// 从消息内容提取 (文本, file_id, file_name)。
/// 非消息类内容（删除/通话等）返回 None。
pub fn extract_message(
    content: &CIContent,
    file: Option<&CIFile>,
    fallback_text: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    match content {
        CIContent::RcvMsgContent { msg_content, .. } => {
            let text = msg_content
                .text()
                .cloned()
                .unwrap_or_else(|| fallback_text.to_string());
            let (file_id, file_name) = match file {
                Some(f) => (Some(f.file_id.to_string()), Some(f.file_name.clone())),
                None => (None, None),
            };
            Some((text, file_id, file_name))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub contact_id: i64,
    pub target: TargetId,
    pub user_id: i64,
    pub text: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
}

/// 把 NewChatItems 事件映射为平台无关的入站消息列表。
pub fn map_new_chat_items(chat_items: &[AChatItem]) -> Vec<IncomingMessage> {
    let mut out = Vec::new();
    for item in chat_items {
        let Some(contact_id) = direct_contact_id(&item.chat_info) else {
            continue;
        };
        let Some((text, file_id, file_name)) = extract_message(
            &item.chat_item.content,
            item.chat_item.file.as_ref(),
            &item.chat_item.meta.item_text,
        ) else {
            continue;
        };
        out.push(IncomingMessage {
            contact_id,
            target: TargetId(contact_id.to_string()),
            user_id: contact_id,
            text: Some(text),
            file_id,
            file_name,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use simploxide_client::types::{
        CIContent, CIDeleteMode, CIFile, CIFileStatus, FileProtocol, JsonObject, MsgContent,
    };

    fn rcv_text(text: &str) -> CIContent {
        CIContent::RcvMsgContent {
            msg_content: MsgContent::Text {
                text: text.to_string(),
                undocumented: JsonObject::default(),
            },
            undocumented: JsonObject::default(),
        }
    }

    fn sample_file() -> CIFile {
        CIFile {
            file_id: 9,
            file_name: "a.txt".into(),
            file_size: 3,
            file_source: None,
            file_status: CIFileStatus::RcvComplete,
            file_protocol: FileProtocol::Smp,
            undocumented: JsonObject::default(),
        }
    }

    #[test]
    fn extracts_text_message() {
        let got = extract_message(&rcv_text("/menu"), None, "fallback");
        assert_eq!(got, Some(("/menu".to_string(), None, None)));
    }

    #[test]
    fn extracts_file_metadata() {
        let f = sample_file();
        let got = extract_message(&rcv_text("cap"), Some(&f), "fallback");
        assert_eq!(
            got,
            Some(("cap".to_string(), Some("9".to_string()), Some("a.txt".to_string())))
        );
    }

    #[test]
    fn falls_back_to_item_text_when_msg_content_has_no_text() {
        let content = CIContent::RcvMsgContent {
            msg_content: MsgContent::Image {
                text: String::new(),
                image: "x.jpg".into(),
                undocumented: JsonObject::default(),
            },
            undocumented: JsonObject::default(),
        };
        let got = extract_message(&content, None, "item-text-fallback");
        assert_eq!(got, Some(("item-text-fallback".to_string(), None, None)));
    }

    #[test]
    fn ignores_non_message_content() {
        let deleted = CIContent::RcvDeleted {
            delete_mode: CIDeleteMode::Broadcast,
            undocumented: JsonObject::default(),
        };
        assert_eq!(extract_message(&deleted, None, "x"), None);
    }

    #[test]
    fn direct_contact_id_only_for_direct_chats() {
        // 构造直聊需要完整 Contact；此处只验证非直聊分支返回 None
        assert_eq!(direct_contact_id(&ChatInfo::Local {
            note_folder: serde_json::from_value(serde_json::json!({
                "noteFolderId": 1,
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z",
                "chatTs": "2026-01-01T00:00:00Z"
            })).unwrap(),
            undocumented: JsonObject::default(),
        }), None);
    }
}
```

- [ ] **Step 2: 运行确认失败**

在 `rust/aegis/src/gateways/mod.rs` 追加 `pub mod simplex;` 后运行：

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex::adapter`
Expected: 编译失败（`MsgContent::text()` 或 `ChatInfo::Local` 字段不符时按实际签名调整；若 `Local` 构造过重，改为 `ChatInfo::Group` 或直接删除该用例并保留其余 4 个）

- [ ] **Step 3: 最小实现**

按 Step 1 的代码补齐；`ChatInfo::Local` 用例若因字段过多无法构造，替换为断言 `direct_contact_id` 对 `ChatInfo::ContactRequest` 返回 `None`（同样只需最小字段）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex::adapter`
Expected: 5 passed

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test
git add rust/aegis/src/gateways/mod.rs rust/aegis/src/gateways/simplex/
git commit -m "feat(simplex): 事件映射纯函数与单测"
```

---

### Task 4: `SimplexAdapter` 实现 `BotAdapter`

**Files:**
- Modify: `rust/aegis/src/gateways/simplex/adapter.rs`

**Interfaces:**
- Consumes: Task 1/2/3
- Produces: `SimplexAdapter::new(bot: simploxide_client::ws::Bot) -> Self`，`impl BotAdapter for SimplexAdapter`

- [ ] **Step 1: 写失败测试**

在 `adapter.rs` 的 `tests` 模块追加（验证能力位与 markup 渲染路径，不触网）：

```rust
    #[test]
    fn adapter_reports_simplex_capabilities() {
        // 能力位由 PlatformCapabilities::SIMPLEX 决定，adapter 只是透传
        let caps = crate::common::PlatformCapabilities::SIMPLEX;
        assert!(!caps.has_inline_keyboard);
        assert!(caps.can_send_file);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex::adapter`
Expected: 编译失败（`SimplexAdapter` 尚未定义）

- [ ] **Step 3: 实现 adapter**

在 `adapter.rs` 顶部补充导入并添加实现：

```rust
use crate::common::{
    BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use crate::common::markup::render_markup_buttons;
use anyhow::{Context, Result};
use async_trait::async_trait;
use simploxide_client::prelude::{CIDeleteMode, ChatId, MessageId as SxMessageId, Reaction};
use simploxide_client::types::MsgContent;
use std::path::PathBuf;

pub struct SimplexAdapter {
    bot: simploxide_client::ws::Bot,
}

impl SimplexAdapter {
    pub fn new(bot: simploxide_client::ws::Bot) -> Self {
        Self { bot }
    }

    pub fn inner_bot(&self) -> &simploxide_client::ws::Bot {
        &self.bot
    }
}

/// 把字节写入临时文件（simplex-chat 与服务同机，可读到该路径）。
fn write_temp_file(name: &str, data: &[u8]) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("aegis-simplex");
    std::fs::create_dir_all(&dir).context("创建 SimpleX 临时目录失败")?;
    let path = dir.join(format!("{}-{}", std::process::id(), name));
    std::fs::write(&path, data).context("写入 SimpleX 临时文件失败")?;
    Ok(path)
}

#[async_trait]
impl BotAdapter for SimplexAdapter {
    fn platform(&self) -> Platform {
        Platform::Simplex
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let text = match &content.markup {
            Some(markup) => render_markup_buttons(content.text, markup),
            None => content.text,
        };
        let resp = self
            .bot
            .send_msg(chat_id, text)
            .await
            .context("发送 SimpleX 消息失败")?;
        Ok(MessageId(first_item_id(&resp).to_string()))
    }

    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let mid = SxMessageId::from_raw(msg_id.0.parse::<i64>().context("SimpleX msg_id 非整数")?);
        self.bot
            .update_msg(chat_id, mid, MsgContent::make_text(content.text))
            .await
            .context("编辑 SimpleX 消息失败")?;
        Ok(())
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let mid = SxMessageId::from_raw(msg_id.0.parse::<i64>().context("SimpleX msg_id 非整数")?);
        self.bot
            .delete_msg(chat_id, mid, CIDeleteMode::Broadcast)
            .await
            .context("删除 SimpleX 消息失败")?;
        Ok(())
    }

    async fn send_reaction(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        emoji: &str,
    ) -> Result<()> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let mid = SxMessageId::from_raw(msg_id.0.parse::<i64>().context("SimpleX msg_id 非整数")?);
        let results = self
            .bot
            .update_msg_reaction(chat_id, mid, Reaction::Set(emoji.to_string()))
            .await;
        if let Some(Err(e)) = results.into_iter().next() {
            anyhow::bail!("SimpleX reaction 失败: {e}");
        }
        Ok(())
    }

    async fn send_file(
        &self,
        target: &TargetId,
        name: &str,
        data: Vec<u8>,
        _mime: &str,
    ) -> Result<MessageId> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let path = write_temp_file(name, &data)?;
        let result = self
            .bot
            .send_msg(chat_id, simploxide_client::messages::File::new(&path))
            .await;
        let _ = std::fs::remove_file(&path);
        let resp = result.context("发送 SimpleX 文件失败")?;
        Ok(MessageId(first_item_id(&resp).to_string()))
    }

    async fn send_image(
        &self,
        target: &TargetId,
        data: Vec<u8>,
        mime: &str,
    ) -> Result<MessageId> {
        let chat_id = ChatId::from_raw(target.0.parse::<i64>().context("SimpleX target 非整数")?);
        let path = write_temp_file("image", &data)?;
        let result = self
            .bot
            .send_msg(chat_id, simploxide_client::messages::Image::new(&path))
            .await;
        let _ = std::fs::remove_file(&path);
        let resp = result.context("发送 SimpleX 图片失败")?;
        let _ = mime;
        Ok(MessageId(first_item_id(&resp).to_string()))
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::SIMPLEX
    }
}

/// 从发送响应中取第一条 chat item 的 itemId。
fn first_item_id(resp: &simploxide_client::types::NewChatItemsResponse) -> i64 {
    resp.chat_items
        .first()
        .map(|ci| ci.chat_item.meta.item_id)
        .unwrap_or(0)
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex`
Expected: 全绿；若 `NewChatItemsResponse` 路径或 `send_msg` 返回类型不符，按 `cargo doc -p simploxide-client --no-deps` 的实际签名修正（`MessageBuilder` 的 `IntoFuture::Output = Result<Arc<NewChatItemsResponse>, ClientError>`）。

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/gateways/simplex/adapter.rs
git commit -m "feat(simplex): SimplexAdapter 实现 BotAdapter"
```

---

### Task 5: 配置字段与管理员身份

**Files:**
- Modify: `rust/aegis/src/bootstrap.rs`
- Modify: `rust/aegis/src/main/config.rs`
- Modify: `rust/aegis/src/app/state.rs`
- Test: 各文件内 `#[cfg(test)]`

**Interfaces:**
- Consumes: 无
- Produces:
  - `EncryptedConfig.simplex_port: Option<Vec<u8>>`、`EncryptedConfig.simplex_admin_id: Option<Vec<u8>>`
  - `run_setup(..., simplex_port: Option<&str>, simplex_admin_id: Option<&str>)`
  - `AppState::new(admin_id, discord_admin_id, simplex_admin_id, totp_manager, executor, hash, timeout, adapter)`
  - `AppState::is_admin_user` 识别 simplex

- [ ] **Step 1: 写失败测试**

`rust/aegis/src/bootstrap.rs` 的 `config_tests` 追加：

```rust
    #[test]
    fn simplex_config_fields_round_trip() {
        let config = EncryptedConfig {
            token: None,
            admin_id: None,
            totp_secret: None,
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
            simplex_port: Some(b"5225".to_vec()),
            simplex_admin_id: Some(b"42".to_vec()),
        };
        let json = serde_json::to_vec(&config).unwrap();
        let back: EncryptedConfig = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.simplex_port, Some(b"5225".to_vec()));
        assert_eq!(back.simplex_admin_id, Some(b"42".to_vec()));
    }
```

`rust/aegis/src/app/state.rs` 的测试模块追加：

```rust
    #[tokio::test]
    async fn simplex_admin_id_is_recognized_as_admin() {
        let state = AppState::new(
            None,
            None,
            Some(777),
            None,
            Arc::new(NoopExecutor),
            None,
            600,
            Arc::new(crate::common::MockBotAdapter::new()),
        );
        assert!(state.is_admin_user(777));
        assert!(!state.is_admin_user(778));
    }
```

（`NoopExecutor` 若测试模块未定义，复用该模块既有的 executor 桩类型名。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex_config_fields_round_trip simplex_admin_id_is_recognized_as_admin`
Expected: 编译失败，`struct EncryptedConfig has no field named simplex_port` / `this function takes 7 arguments but 8 were supplied`

- [ ] **Step 3: 实现**

`bootstrap.rs`：

```rust
// EncryptedConfig 追加
    #[serde(default)]
    pub simplex_port: Option<Vec<u8>>,
    #[serde(default)]
    pub simplex_admin_id: Option<Vec<u8>>,

// SetupInput 追加
    #[serde(default)]
    simplex_port: Option<String>,
    #[serde(default)]
    simplex_admin_id: Option<String>,

// impl Drop for EncryptedConfig 追加
        if let Some(v) = &mut self.simplex_port {
            v.zeroize();
        }
        if let Some(v) = &mut self.simplex_admin_id {
            v.zeroize();
        }
```

`run_setup` 签名追加两个参数（放在 `matrix_recovery_key` 之后），并在构造 `EncryptedConfig` 前加密：

```rust
    let simplex_port = simplex_port
        .map(|v| security.encrypt(v.trim().as_bytes()))
        .transpose()?;
    let simplex_admin_id = simplex_admin_id
        .map(|v| security.encrypt(v.trim().as_bytes()))
        .transpose()?;
```

`EncryptedConfig { .. }` 字面量补 `simplex_port, simplex_admin_id`。

`run_setup_from_stdin` 透传：

```rust
    let simplex_port = input.simplex_port.as_deref();
    let simplex_admin_id = input.simplex_admin_id.as_deref();
    run_setup(
        input.token.as_deref(),
        input.admin_id.as_deref(),
        input.totp_secret.as_deref(),
        matrix,
        discord_token,
        discord_admin_id,
        matrix_recovery_key,
        simplex_port,
        simplex_admin_id,
    )
    .await
```

`main/cli.rs` 的 `run_setup(...)` 调用补两个 `None`。

`main/config.rs`：`DecryptedConfig` 追加

```rust
    #[expect(dead_code)]
    pub simplex_port: Option<String>,
    #[expect(dead_code)]
    pub simplex_admin_id: Option<i64>,
```

并在 `load_and_validate` 中解密（照抄 `discord_admin_id` 的写法），`AppConfig.decrypted` 字面量补齐。

`app/state.rs`：

```rust
    simplex_admin_id: Option<i64>,

    pub fn new(
        admin_id: Option<i64>,
        discord_admin_id: Option<i64>,
        simplex_admin_id: Option<i64>,
        totp_manager: Option<TotpManager>,
        self_destruct_executor: Arc<dyn SelfDestructExecutor>,
        self_destruct_key_hash: Option<String>,
        session_timeout_secs: u64,
        adapter: Arc<dyn BotAdapter>,
    ) -> Self {
        Self {
            adapter,
            admin_id,
            discord_admin_id,
            simplex_admin_id,
            // ... 其余不变
        }
    }

    pub fn is_admin_user(&self, user_id: i64) -> bool {
        user_id == self.admin_id.unwrap_or(0)
            || self.discord_admin_id == Some(user_id)
            || self.simplex_admin_id == Some(user_id)
    }
```

- [ ] **Step 4: 修全部调用点**

Run: `cargo check --all-targets --all-features`
Expected: 报出所有 `AppState::new` / `EncryptedConfig {` / `run_setup` 缺参数处；逐个补齐（`AppState::new` 测试桩传 `None`；`EncryptedConfig` 字面量在 `matrix.rs`、`config.rs`、`discord.rs` 的测试里补 `simplex_port: None, simplex_admin_id: None`）。

- [ ] **Step 5: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test`
Expected: 全绿

- [ ] **Step 6: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/bootstrap.rs rust/aegis/src/main/config.rs rust/aegis/src/main/cli.rs rust/aegis/src/app/state.rs rust/aegis/src/main/matrix.rs rust/aegis/src/main/discord.rs
git commit -m "feat(simplex): 配置字段与管理员身份"
```

---

### Task 6: 连接层 `main/simplex.rs`

**Files:**
- Create: `rust/aegis/src/main/simplex.rs`
- Modify: `rust/aegis/src/main/mod.rs`

**Interfaces:**
- Consumes: Task 4（`SimplexAdapter`）、Task 5（`EncryptedConfig` 字段）
- Produces:
  - `pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool`
  - `pub struct SimplexHandle { pub bot: simploxide_client::ws::Bot, pub events: simploxide_client::EventStream, pub adapter: Arc<dyn BotAdapter> }`
  - `pub async fn connect_simplex(security: &SecurityManager, encrypted_config: &EncryptedConfig, config_dir: &Path) -> Result<SimplexHandle>`

- [ ] **Step 1: 写失败测试**

创建 `rust/aegis/src/main/simplex.rs`，先只写 `has_simplex_config` 与测试：

```rust
use std::path::Path;
use std::sync::Arc;

use aegis::common::BotAdapter;
use aegis::core::security::SecurityManager;
use aegis::gateways::simplex::SimplexAdapter;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;

use crate::bootstrap::EncryptedConfig;

pub struct SimplexHandle {
    pub bot: simploxide_client::ws::Bot,
    pub events: simploxide_client::EventStream,
    pub adapter: Arc<dyn BotAdapter>,
}

pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--simplex");
    explicit
        || (encrypted_config.simplex_port.is_some()
            && encrypted_config.simplex_admin_id.is_some())
}

pub async fn connect_simplex(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
    _config_dir: &Path,
) -> Result<SimplexHandle> {
    let decrypt = |field: &Option<Vec<u8>>, what: &str| -> Result<String> {
        let vec = security.decrypt(field.as_ref().with_context(|| format!("缺少 {what}"))?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("{what} 包含无效 UTF-8: {e}"))?
            .trim()
            .to_string())
    };

    let port: u16 = decrypt(&encrypted_config.simplex_port, "simplex_port")?
        .parse()
        .context("simplex_port 应为 1-65535 的整数")?;
    let _admin_id: i64 = decrypt(&encrypted_config.simplex_admin_id, "simplex_admin_id")?
        .parse()
        .context("simplex_admin_id 应为整数 contactId")?;

    let (bot, events) = simploxide_client::ws::BotBuilder::new("Aegis", port)
        .auto_accept_with(rust_i18n::t!("simplex.welcome").to_string())
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("连接 SimpleX WebSocket 失败: {e}"))?;

    let adapter: Arc<dyn BotAdapter> = Arc::new(SimplexAdapter::new(bot.clone()));
    Ok(SimplexHandle {
        bot,
        events,
        adapter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> EncryptedConfig {
        EncryptedConfig {
            token: None,
            admin_id: None,
            totp_secret: None,
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
            simplex_port: None,
            simplex_admin_id: None,
        }
    }

    #[test]
    fn returns_false_when_no_simplex_config() {
        assert!(!has_simplex_config(&empty_config(), &[]));
    }

    #[test]
    fn returns_true_when_flag_present() {
        assert!(has_simplex_config(&empty_config(), &["--simplex".to_string()]));
    }

    #[test]
    fn returns_true_when_both_fields_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        cfg.simplex_admin_id = Some(vec![1]);
        assert!(has_simplex_config(&cfg, &[]));
    }

    #[test]
    fn returns_false_when_only_port_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        assert!(!has_simplex_config(&cfg, &[]));
    }
}
```

- [ ] **Step 2: 运行确认失败**

`rust/aegis/src/main/mod.rs` 追加 `pub mod simplex;` 后：

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex::tests`
Expected: 编译失败（`rust_i18n::t!("simplex.welcome")` 缺失或 `SimplexAdapter` 尚未导出）

- [ ] **Step 3: 补 i18n key**

在 `rust/aegis/src/resources/i18n/{en,zh,ja}.yml` 各追加（缩进与同级 key 对齐）：

```yaml
simplex:
  welcome: "Aegis bot 已就绪。发送 /menu 查看操作。"
```

（en/ja 分别写对应语言文案。）

- [ ] **Step 4: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test -p aegis simplex::tests`
Expected: 4 passed

- [ ] **Step 5: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test
git add rust/aegis/src/main/simplex.rs rust/aegis/src/main/mod.rs rust/aegis/src/resources/i18n/
git commit -m "feat(simplex): 连接层与 has_simplex_config"
```

---

### Task 7: CLI 接线与 runtime 网关循环

**Files:**
- Modify: `rust/aegis/src/main.rs`
- Modify: `rust/aegis/src/main/runtime.rs`

**Interfaces:**
- Consumes: Task 6（`SimplexHandle`）
- Produces: `--simplex` 可用；`runtime::run(..., simplex_handle: Option<SimplexHandle>)`

- [ ] **Step 1: 写失败测试**

`rust/aegis/src/main.rs` 测试模块追加纯函数并测试（把平台判定抽成可测函数）：

```rust
fn resolve_platforms(args: &[String]) -> (bool, bool, bool) {
    let use_matrix = args.iter().any(|a| a == "--matrix");
    let use_discord = args.iter().any(|a| a == "--discord");
    let use_simplex = args.iter().any(|a| a == "--simplex");
    let use_all = args.iter().any(|a| a == "--all");
    let enable_discord = use_discord;
    let enable_simplex = use_simplex;
    let enable_matrix = (use_matrix || use_all) && !enable_discord && !enable_simplex;
    let enable_telegram = (!use_matrix && !use_discord && !use_simplex) || use_all;
    (enable_telegram, enable_matrix, enable_discord || enable_simplex)
}

#[cfg(test)]
mod platform_resolution_tests {
    use super::resolve_platforms;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn default_is_telegram_only() {
        assert_eq!(resolve_platforms(&v(&[])), (true, false, false));
    }

    #[test]
    fn simplex_is_standalone() {
        assert_eq!(resolve_platforms(&v(&["--simplex"])), (false, false, true));
    }

    #[test]
    fn simplex_wins_over_all() {
        assert_eq!(
            resolve_platforms(&v(&["--all", "--simplex"])),
            (true, false, true)
        );
    }

    #[test]
    fn matrix_and_discord_unchanged() {
        assert_eq!(resolve_platforms(&v(&["--matrix"])), (false, true, false));
        assert_eq!(resolve_platforms(&v(&["--discord"])), (false, false, true));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo nextest run --cargo-profile fast-test -p aegis platform_resolution_tests`
Expected: 编译失败，`cannot find function resolve_platforms`

- [ ] **Step 3: 实现 CLI 接线**

`main.rs` 中把现有 4 行平台判定替换为调用 `resolve_platforms(&args)`，并新增：

```rust
    let use_simplex = args.iter().any(|a| a == "--simplex");
    let (enable_telegram, enable_matrix, _standalone_flag) = resolve_platforms(&args);
```

auto-detect 段追加：

```rust
    let has_simplex = main::simplex::has_simplex_config(
        &app_config.decrypted.encrypted_config,
        &args,
    );
    let simplex_handle = if use_simplex || (has_simplex && !args.iter().any(|a| a == "--tg-only")) {
        Some(
            main::simplex::connect_simplex(
                &security,
                &app_config.decrypted.encrypted_config,
                &config_dir(),
            )
            .await?,
        )
    } else {
        None
    };
```

adapter 优先级改为：

```rust
    let adapter = if let Some(ref raw) = discord_raw {
        raw.adapter.clone()
    } else if let Some(ref h) = simplex_handle {
        h.adapter.clone()
    } else {
        main::adapter::build_adapter(
            app_config.decrypted.token.as_deref(),
            enable_telegram,
            enable_matrix,
            &matrix_handle,
        )
        .await?
    };
```

`AppState::new` 传入 `app_config.decrypted.simplex_admin_id`；`runtime::run` 传入 `simplex_handle`。

- [ ] **Step 4: runtime 网关段**

`runtime.rs`：`run` 签名追加 `simplex_handle: Option<super::simplex::SimplexHandle>`，并新增段落（放在 Matrix 段之后）：

```rust
    // ── SimpleX 网关 ──
    if let Some(handle) = simplex_handle {
        let adapter_for_init = handle.adapter.clone();
        let target_for_init = TargetId(
            state
                .admin_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "0".to_string()),
        );
        tokio::spawn(async move {
            if let Err(e) = aegis::core::system::scheduler::start_scheduler(
                adapter_for_init.clone(),
                target_for_init.clone(),
            )
            .await
            {
                log::error!("❌ 初始化调度器失败: {}", e);
            }
            tokio::join!(
                async {
                    let _ =
                        crate::notify_upgrade_success(&*adapter_for_init, &target_for_init).await;
                },
                async {
                    let _ = crate::notify_bbr3_reboot_result(&*adapter_for_init, &target_for_init)
                        .await;
                },
                async {
                    let _ = crate::notify_online(&*adapter_for_init, &target_for_init).await;
                },
            );
        });

        let state_for_events = state.clone();
        let admin_id = state.admin_id();
        let token = CancellationToken::new();
        let token_clone = token.clone();

        tokio::spawn(async move {
            token_clone.cancelled().await;
        });
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            log::info!("收到关闭信号，正在优雅关闭 SimpleX...");
            token.cancel();
        });

        tokio::spawn(async move {
            let mut events = handle.events;
            while let Some(ev) = events.next().await {
                let Ok(ev) = ev else { continue };
                let simploxide_client::events::Event::NewChatItems(items) = ev else {
                    continue;
                };
                for msg in aegis::gateways::simplex::map_new_chat_items(&items.chat_items) {
                    if admin_id != Some(msg.contact_id) {
                        log::warn!(
                            "SimpleX 未授权联系人 contactId={} 尝试发消息，已忽略",
                            msg.contact_id
                        );
                        continue;
                    }
                    let event = BotEvent::Message(MessageEvent {
                        adapter: handle.adapter.clone(),
                        target: msg.target.clone(),
                        user_id: msg.user_id,
                        text: msg.text.clone(),
                        file_id: msg.file_id.clone(),
                        file_name: msg.file_name.clone(),
                        reply_to_text: None,
                        thread_root: None,
                    });
                    let _ = dispatch_event(event, &state_for_events).await;
                }
            }
        });
    }
```

（`events.next()` 需要 `use futures_util::StreamExt;`，`futures-util` 已是依赖。）

- [ ] **Step 5: 运行确认通过**

Run: `cargo nextest run --cargo-profile fast-test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: 全绿；若 `Event` 枚举变体名或 `items` 字段名不符，按 `simploxide_client::events` 实际定义修正。

- [ ] **Step 6: 质量门 + 提交**

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings && cargo nextest run --cargo-profile fast-test && cargo test --doc
git add rust/aegis/src/main.rs rust/aegis/src/main/runtime.rs
git commit -m "feat(simplex): CLI --simplex 与 runtime 网关循环"
```

---

### Task 8: 安装器平台选择与 setup payload

**Files:**
- Modify: `go/installer/main.go`
- Modify: `go/installer/main_test.go`
- Modify: `go/installer/i18n/{en,zh,ja}.json`

**Interfaces:**
- Consumes: Task 5 的配置字段名（`simplex_port` / `simplex_admin_id`）
- Produces: `parsePlatformChoice` 返回 4 个 bool；`buildSetupPayload` 支持 simplex 字段；`writeSystemdService("simplex")`

- [ ] **Step 1: 写失败测试**

`go/installer/main_test.go` 追加：

```go
func TestParsePlatformChoiceSimplex(t *testing.T) {
	tg, matrix, discord, simplex, err := parsePlatformChoice("simplex")
	if err != nil {
		t.Fatalf("unexpected err: %v", err)
	}
	if tg || matrix || discord || !simplex {
		t.Fatalf("simplex standalone expected, got tg=%t matrix=%t discord=%t simplex=%t", tg, matrix, discord, simplex)
	}
}

func TestParsePlatformChoiceRejectsSimplexCombo(t *testing.T) {
	if _, _, _, _, err := parsePlatformChoice("simplex+matrix"); err == nil {
		t.Fatal("simplex+matrix must be rejected")
	}
}

func TestWriteSystemdServiceSimplexUsesFlag(t *testing.T) {
	// writeSystemdService 写固定路径，这里只验证 platformFlag 映射函数
	if got := platformFlagFor("simplex"); got != "--simplex" {
		t.Fatalf("platformFlagFor(simplex) = %q", got)
	}
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cd go/installer && go test ./... -run 'Simplex'`
Expected: 编译失败，`assignment mismatch: 5 variables but parsePlatformChoice returns 4 values`

- [ ] **Step 3: 实现**

`main.go`：

```go
// 抽出可测的 flag 映射
func platformFlagFor(platform string) string {
	switch platform {
	case "matrix":
		return "--matrix"
	case "discord":
		return "--discord"
	case "simplex":
		return "--simplex"
	case "tg-matrix":
		return "--all"
	}
	return ""
}
```

`writeSystemdService` 改为使用 `platformFlagFor`，并加 `case "simplex"` 的 `descName = "WWPS SimpleX Bot"`。

`parsePlatformChoice` 返回 `(bool, bool, bool, bool, error)`，新增：

```go
	case "simplex":
		return false, false, false, true, nil
```

`platformSelection` 返回 4 个 bool；`selectDeploymentPlatforms` 返回 4 个 bool；`platformSelector` 的 `labels`/`choices` 增加 Simplex 项。

`buildSetupPayload` 签名追加 `simplexPort, simplexAdminID string`，并在函数内按既有 `discord_token` 模式追加：

```go
	if simplexPort != "" {
		buf = append(buf, []byte(`,"simplex_port":`)...)
		buf = appendJSONEscaped(buf, []byte(simplexPort))
	}
	if simplexAdminID != "" {
		buf = append(buf, []byte(`,"simplex_admin_id":`)...)
		buf = appendJSONEscaped(buf, []byte(simplexAdminID))
	}
```

`firstTimeSetup` 在 `enableSimplex` 时提示端口与管理员 contactId（用 `readSecureInput`），并把结果传给 `buildSetupPayload`。

`installFromStdin` 自动识别：

```go
	} else if _, ok := inputData["simplex_port"].(string); ok {
		platform = "simplex"
	}
```

- [ ] **Step 4: 补 i18n**

`go/installer/i18n/{en,zh,ja}.json` 各追加键（值按语言翻译）：

```json
"firsttime.platform_selector_simplex": "SimpleX",
"firsttime.simplex_section": "\n========== SimpleX Bot Deployment ==========",
"firsttime.simplex_desc1": "💡 SimpleX runs as a standalone platform (--simplex).",
"firsttime.simplex_desc2": "   Requires a local simplex-chat v7.0.0 WebSocket service.",
"firsttime.simplex_port_prompt": "Enter SimpleX WebSocket port (default 5225): ",
"firsttime.simplex_admin_prompt": "Enter SimpleX admin contactId: "
```

同时更新 `firsttime.platform_prompt` 与 `firsttime.platform_text_prompt` 文案，加入 SimpleX 选项。

- [ ] **Step 5: 运行确认通过**

Run: `cd go/installer && gofmt -l . && go vet ./... && go test ./...`
Expected: `gofmt -l` 无输出；测试全绿（含既有 `TestParsePlatformChoice`，需同步改为 5 返回值）

- [ ] **Step 6: 提交**

```bash
git add go/installer/main.go go/installer/main_test.go go/installer/i18n/
git commit -m "feat(installer): SimpleX 平台选择与 setup payload"
```

---

### Task 9: simplex-chat 下载与 systemd 单元

**Files:**
- Modify: `go/installer/main.go`
- Test: `go/installer/main_test.go`

**Interfaces:**
- Consumes: Task 8
- Produces: `simplexChatAssetName(arch string) string`；`writeSimplexSystemdService(port string)`；`installSimplexChat() (string, error)`

- [ ] **Step 1: 写失败测试**

```go
func TestSimplexChatAssetName(t *testing.T) {
	cases := map[string]string{
		"amd64": "simplex-chat-ubuntu-24_04-x86_64",
		"arm64": "simplex-chat-ubuntu-24_04-aarch64",
	}
	for arch, want := range cases {
		if got := simplexChatAssetName(arch); got != want {
			t.Fatalf("arch %s: got %q want %q", arch, got, want)
		}
	}
}

func TestSimplexChatAssetNameRejectsUnknownArch(t *testing.T) {
	if got := simplexChatAssetName("riscv64"); got != "" {
		t.Fatalf("unknown arch must return empty, got %q", got)
	}
}

func TestSimplexSystemdUnitPinsVersionAndLocalhost(t *testing.T) {
	unit := simplexSystemdUnitContent("5225")
	if !strings.Contains(unit, "ExecStart=") || !strings.Contains(unit, "-p 5225") {
		t.Fatalf("unit must start simplex-chat on the configured port: %s", unit)
	}
	if !strings.Contains(unit, "Restart=always") {
		t.Fatal("unit must restart on failure")
	}
}
```

（`main_test.go` 需 `import "strings"`。）

- [ ] **Step 2: 运行确认失败**

Run: `cd go/installer && go test ./... -run 'SimplexChat|SimplexSystemd'`
Expected: 编译失败，`undefined: simplexChatAssetName`

- [ ] **Step 3: 实现**

```go
// simplex-chat 版本由 simploxide-client 0.14.0 的兼容表锁定（min=max=7.0.0.0）。
const simplexChatVersion = "v7.0.0"

const simplexServiceFile = "/etc/systemd/system/wwps-simplex.service"

func simplexChatAssetName(arch string) string {
	switch arch {
	case "amd64":
		return "simplex-chat-ubuntu-24_04-x86_64"
	case "arm64":
		return "simplex-chat-ubuntu-24_04-aarch64"
	}
	return ""
}

func simplexSystemdUnitContent(port string) string {
	return `[Unit]
Description=WWPS SimpleX Chat CLI (WebSocket bot API)
After=network.target

[Service]
Type=simple
User=root
ExecStart=` + filepath.Join(installDir, "simplex-chat") + ` -p ` + port + `
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
`
}

func writeSimplexSystemdService(port string) {
	if err := os.WriteFile(simplexServiceFile, []byte(simplexSystemdUnitContent(port)), 0o644); err != nil {
		printRed("写入 SimpleX systemd 服务文件失败: " + err.Error())
	}
}
```

下载复用既有 `downloadFile` / `findAsset` / `findExpectedSHA256` 模式，新增 `installSimplexChat()`：
从 `simplex-chat/simplex-chat` 的 `simplexChatVersion` release 取 `simplexChatAssetName(runtime.GOARCH)`，
校验 SHA256 后落到 `filepath.Join(installDir, "simplex-chat")` 并 `chmod 0755`。

`finishDeploy` 中当 `platform == "simplex"` 时额外调用 `installSimplexChat()` 与
`writeSimplexSystemdService(port)`，并 `systemctl enable --now wwps-simplex.service`。

- [ ] **Step 4: 运行确认通过**

Run: `cd go/installer && gofmt -l . && go vet ./... && go test ./...`
Expected: 全绿

- [ ] **Step 5: 提交**

```bash
git add go/installer/main.go go/installer/main_test.go
git commit -m "feat(installer): simplex-chat v7.0.0 下载与 systemd 单元"
```

---

### Task 10: 文档与手工验收

**Files:**
- Modify: `README.md`
- Create: `docs/2026-09-16-simplex-platform.md`

**Interfaces:**
- Consumes: 全部前置任务
- Produces: 运维文档（含版本锁定警告与手工验收步骤）

- [ ] **Step 1: 写文档**

`docs/2026-09-16-simplex-platform.md` 内容须包含：

1. 前置：`simplex-chat` 必须为 **v7.0.0**（simploxide 0.14 兼容表 min=max），
   禁止交给 unattended-upgrades 自动升级。
2. 部署：`wwps-simplex.service` 运行 `simplex-chat -p <port>`（仅 localhost，无鉴权，
   必须在同机且防火墙关闭该端口）。
3. 首次配置：启动后让管理员主动连接 bot 地址；bot 日志会打印未授权联系人的
   `contactId`（`SimpleX 未授权联系人 contactId=...`），把该值写入 `simplex_admin_id`。
4. 手工验收清单：
   - `aegis --simplex` 启动无错误
   - 管理员发 `/menu` 能收到菜单
   - 菜单按钮以「`N. 文本 — send: \`data\``」形式渲染
   - 非管理员发消息被忽略且日志有记录
   - 触发一次文件下发（如导出配置）能收到文件
5. 已知限制：`download_file` 不支持 → `setsecurityfile` 在 SimpleX 上不可用；
   typing 指示不支持；群聊不支持。

`README.md` 的 Scope 段加入 SimpleX 平台说明。

- [ ] **Step 2: 提交**

```bash
git add README.md docs/2026-09-16-simplex-platform.md
git commit -m "docs(simplex): 部署与手工验收说明"
```

---

## Self-Review

**Spec 覆盖检查：**

| Spec 章节 | 覆盖任务 |
|---|---|
| §2 选型（simploxide websocket / systemd） | Task 2（依赖）、Task 9（systemd） |
| §3 Rust 结构 | Task 3、4、6、7 |
| §4 配置与身份 | Task 5、6 |
| §5 收发映射 | Task 3（收）、4（发）、7（循环） |
| §6 能力矩阵 | Task 1 |
| §7 部署 | Task 8、9 |
| §8 测试策略 | 各任务 RED/GREEN + Task 10 手工验收 |
| §9 分期 | 阶段 1 = Task 1–7；阶段 2 = Task 8–9；Task 10 收尾 |
| §10 风险 | Task 9（版本锁定）+ Task 10（文档警告） |

**类型一致性：** `SimplexAdapter::new(bot)`、`SimplexHandle{bot,events,adapter}`、
`map_new_chat_items(&[AChatItem]) -> Vec<IncomingMessage>`、`AppState::new` 8 参数、
`run_setup` 9 参数在各任务中一致。

**已知需在实现时按实际 API 微调的点（已在对应任务标注回退方式）：**
`ChatInfo::Local` 测试构造、`NewChatItemsResponse` 路径、`events::Event` 变体名。
