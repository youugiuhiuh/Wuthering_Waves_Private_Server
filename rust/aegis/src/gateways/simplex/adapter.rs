use crate::common::markup::render_markup_buttons;
use crate::common::{
    BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use simploxide_client::prelude::{
    CIDeleteMode, ChatId, ContactId, MessageId as SxMessageId, NewChatItemsResponse, Reaction,
};
use simploxide_client::types::{AChatItem, CIContent, CIFile, ChatInfo, MsgContent};
use std::path::PathBuf;

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

#[derive(Debug, Clone)]
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

/// SimpleX 适配器：把一个 `simplex-chat` WebSocket bot 句柄包装为 `BotAdapter`。
///
/// 与 Matrix 不同，`target` 是每条消息自带的 chat id（直聊即对方 contactId），
/// 不使用构造期固定房间。
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

/// 把 TargetId 解析为 SimpleX 直聊 id。
fn parse_chat_id(target: &TargetId) -> Result<ChatId> {
    let raw = target
        .0
        .parse::<i64>()
        .with_context(|| format!("SimpleX target 非整数: {}", target.0))?;
    Ok(ContactId::from_raw(raw).into())
}

/// 把 MessageId 解析为 SimpleX 消息 id。
fn parse_message_id(msg_id: &MessageId) -> Result<SxMessageId> {
    let raw = msg_id
        .0
        .parse::<i64>()
        .with_context(|| format!("SimpleX msg_id 非整数: {}", msg_id.0))?;
    Ok(SxMessageId::from_raw(raw))
}

/// 把字节写入临时文件（simplex-chat 与服务同机，可读到该路径）。
fn write_temp_file(name: &str, data: &[u8]) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("aegis-simplex");
    std::fs::create_dir_all(&dir).context("创建 SimpleX 临时目录失败")?;
    let path = dir.join(format!("{}-{}", std::process::id(), name));
    std::fs::write(&path, data).context("写入 SimpleX 临时文件失败")?;
    Ok(path)
}

/// 从发送响应中取第一条 chat item 的 itemId。
fn first_item_id(resp: &NewChatItemsResponse) -> i64 {
    resp.chat_items
        .first()
        .map(|ci| ci.chat_item.meta.item_id)
        .unwrap_or(0)
}

#[async_trait]
impl BotAdapter for SimplexAdapter {
    fn platform(&self) -> Platform {
        Platform::Simplex
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let chat_id = parse_chat_id(target)?;
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
        let chat_id = parse_chat_id(target)?;
        let mid = parse_message_id(msg_id)?;
        self.bot
            .update_msg(chat_id, mid, MsgContent::make_text(content.text))
            .await
            .context("编辑 SimpleX 消息失败")?;
        Ok(())
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        let chat_id = parse_chat_id(target)?;
        let mid = parse_message_id(msg_id)?;
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
        let chat_id = parse_chat_id(target)?;
        let mid = parse_message_id(msg_id)?;
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
        let chat_id = parse_chat_id(target)?;
        let path = write_temp_file(name, &data)?;
        let result = self
            .bot
            .send_msg(chat_id, simploxide_client::messages::File::new(&path))
            .await;
        let _ = std::fs::remove_file(&path);
        let resp = result.context("发送 SimpleX 文件失败")?;
        Ok(MessageId(first_item_id(&resp).to_string()))
    }

    async fn send_image(&self, target: &TargetId, data: Vec<u8>, _mime: &str) -> Result<MessageId> {
        let chat_id = parse_chat_id(target)?;
        let path = write_temp_file("image", &data)?;
        let result = self
            .bot
            .send_msg(chat_id, simploxide_client::messages::Image::new(&path))
            .await;
        let _ = std::fs::remove_file(&path);
        let resp = result.context("发送 SimpleX 图片失败")?;
        Ok(MessageId(first_item_id(&resp).to_string()))
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::SIMPLEX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simploxide_client::types::{CIFileStatus, FileProtocol, JsonObject};

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

    fn local_chat_info() -> ChatInfo {
        ChatInfo::Local {
            note_folder: serde_json::from_value(serde_json::json!({
                "noteFolderId": 1,
                "userId": 1,
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-01-01T00:00:00Z",
                "chatTs": "2026-01-01T00:00:00Z"
            }))
            .unwrap(),
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
            Some((
                "cap".to_string(),
                Some("9".to_string()),
                Some("a.txt".to_string())
            ))
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
        assert_eq!(direct_contact_id(&local_chat_info()), None);
    }

    /// 完整的 mergedPreferences：7 个键各需 enabled/userPreference/contactPreference。
    fn merged_preferences() -> serde_json::Value {
        let one = serde_json::json!({
            "enabled": { "forUser": false, "forContact": false },
            "userPreference": { "type": "user", "preference": { "allow": "always" } },
            "contactPreference": { "allow": "always" }
        });
        serde_json::json!({
            "timedMessages": one,
            "fullDelete": one,
            "reactions": one,
            "voice": one,
            "files": one,
            "calls": one,
            "sessions": one
        })
    }

    #[test]
    fn map_new_chat_items_maps_direct_text_message() {
        let item: AChatItem = serde_json::from_value(serde_json::json!({
            "chatInfo": {
                "type": "direct",
                "contact": {
                    "contactId": 42,
                    "localDisplayName": "admin",
                    "profile": {
                        "profileId": 2,
                        "displayName": "admin",
                        "fullName": "admin",
                        "localAlias": ""
                    },
                    "contactUsed": true,
                    "contactStatus": "active",
                    "chatSettings": { "enableNtfs": "all" },
                    "userPreferences": {},
                    "mergedPreferences": merged_preferences(),
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                    "chatTags": []
                }
            },
            "chatItem": {
                "chatDir": { "type": "directRcv" },
                "meta": {
                    "itemId": 7,
                    "itemTs": "2026-01-01T00:00:00Z",
                    "itemText": "/menu",
                    "itemStatus": { "type": "rcvNew" },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z"
                },
                "content": {
                    "type": "rcvMsgContent",
                    "msgContent": { "type": "text", "text": "/menu" }
                },
                "mentions": {},
                "reactions": []
            }
        }))
        .unwrap();

        let mapped = map_new_chat_items(&[item]);
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].contact_id, 42);
        assert_eq!(mapped[0].target.0, "42");
        assert_eq!(mapped[0].user_id, 42);
        assert_eq!(mapped[0].text.as_deref(), Some("/menu"));
        assert_eq!(mapped[0].file_id, None);
        assert_eq!(mapped[0].file_name, None);
    }

    #[test]
    fn map_new_chat_items_skips_non_direct_chats() {
        let item: AChatItem = serde_json::from_value(serde_json::json!({
            "chatInfo": {
                "type": "local",
                "noteFolder": {
                    "noteFolderId": 1,
                    "userId": 1,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                    "chatTs": "2026-01-01T00:00:00Z"
                }
            },
            "chatItem": {
                "chatDir": { "type": "localRcv" },
                "meta": {
                    "itemId": 5,
                    "itemTs": "2026-01-01T00:00:00Z",
                    "itemText": "/menu",
                    "itemStatus": { "type": "rcvNew" },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z"
                },
                "content": {
                    "type": "rcvMsgContent",
                    "msgContent": { "type": "text", "text": "/menu" }
                },
                "mentions": {},
                "reactions": []
            }
        }))
        .unwrap();

        assert!(map_new_chat_items(&[item]).is_empty());
    }

    #[test]
    fn adapter_reports_simplex_capabilities() {
        // 能力位由 PlatformCapabilities::SIMPLEX 决定，adapter 只是透传
        let caps = crate::common::PlatformCapabilities::SIMPLEX;
        assert!(!caps.has_inline_keyboard);
        assert!(caps.can_send_file);
    }

    /// SimplexAdapter 必须实现 BotAdapter（类型级断言，无需连接 simplex-chat）。
    #[test]
    fn simplex_adapter_implements_bot_adapter() {
        fn assert_impl<T: crate::common::BotAdapter>() {}
        assert_impl::<SimplexAdapter>();
    }

    #[test]
    fn write_temp_file_writes_bytes_that_can_be_removed() {
        let path = write_temp_file("unit-test.txt", b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn incoming_message_targets_the_contact_id() {
        let msg = IncomingMessage {
            contact_id: 42,
            target: TargetId("42".to_string()),
            user_id: 42,
            text: Some("/menu".to_string()),
            file_id: None,
            file_name: None,
        };
        assert_eq!(msg.target.0, "42");
        assert_eq!(msg.user_id, msg.contact_id);
    }
}
