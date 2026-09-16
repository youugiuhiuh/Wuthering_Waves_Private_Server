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

#[cfg(test)]
mod tests {
    use super::*;
    use simploxide_client::types::{
        CIDeleteMode, CIFileStatus, FileProtocol, JsonObject, MsgContent,
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
