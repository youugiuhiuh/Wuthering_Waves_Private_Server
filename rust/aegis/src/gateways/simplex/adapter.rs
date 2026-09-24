use crate::common::markup::render_markup_buttons;
use crate::common::routing::is_sensitive;
use crate::common::{
    BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use simploxide_client::prelude::{
    CIDeleteMode, ChatId, ContactId, ContactRequestId, MessageId as SxMessageId,
    NewChatItemsResponse, Reaction,
};
use simploxide_client::types::{AChatItem, CIContent, CIFile, ChatInfo, MsgContent};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
///
/// 用 `try_from` 而非 `from_raw`：SimpleX id 以 NonZeroI64 表示，`from_raw(0)`
/// 会 panic；release profile 为 `panic = "abort"`，即整个 bot 进程被终止。
fn parse_chat_id(target: &TargetId) -> Result<ChatId> {
    let raw = target
        .0
        .parse::<i64>()
        .with_context(|| format!("SimpleX target 非整数: {}", target.0))?;
    let contact_id = ContactId::try_from(raw)
        .with_context(|| format!("SimpleX target 非法(0): {}", target.0))?;
    Ok(contact_id.into())
}

/// 把 MessageId 解析为 SimpleX 消息 id。
///
/// 同上：`MessageId("0")` 由 Matrix 文本命令路径合成（`gateways/matrix/commands.rs`），
/// SimpleX 复用该路径，因此必须在此拒绝而不是 panic。
fn parse_message_id(msg_id: &MessageId) -> Result<SxMessageId> {
    let raw = msg_id
        .0
        .parse::<i64>()
        .with_context(|| format!("SimpleX msg_id 非整数: {}", msg_id.0))?;
    SxMessageId::try_from(raw).with_context(|| format!("SimpleX msg_id 非法(0): {}", msg_id.0))
}

/// 解析 contactRequestId。`ContactRequestId::try_from` 只拒 0（NonZeroI64），
/// 负数会被放行；审批接口需要正整数，故显式拒绝非正值。
fn parse_contact_request_id(raw: i64) -> Result<ContactRequestId> {
    if raw <= 0 {
        anyhow::bail!("contactRequestId 必须为正整数: {raw}");
    }
    ContactRequestId::try_from(raw).map_err(|_| anyhow::anyhow!("非法 contactRequestId: {raw}"))
}

/// 清理调用方提供的临时文件名：只保留最后一个路径分量，拒绝 `.`、`..` 与空名。
///
/// 返回值可直接拼接到临时目录下，不含路径分隔符，防止 `../` 或 `/` 逃逸目录。
fn sanitize_file_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next()?.trim().to_string();
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    Some(base)
}

/// 把字节写入仅本进程可读的临时文件（simplex-chat 与服务同机、同 root，可读到该路径）。
///
/// - 目录 `/tmp/aegis-simplex/` 权限强制为 `0700`
/// - 文件以 `0600` 创建，且用 `create_new` 保证不与既有文件冲突
/// - 文件名含 PID 与纳秒时间戳，同一名字并发写入也不会碰撞
fn write_temp_file(name: &str, data: &[u8]) -> Result<PathBuf> {
    let base = sanitize_file_name(name).context("SimpleX 临时文件名非法")?;
    let dir = std::env::temp_dir().join("aegis-simplex");
    std::fs::create_dir_all(&dir).context("创建 SimpleX 临时目录失败")?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .context("设置 SimpleX 临时目录权限失败")?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!("{}-{}-{}", std::process::id(), nanos, base));

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .context("创建 SimpleX 临时文件失败")?;
    file.write_all(data).context("写入 SimpleX 临时文件失败")?;
    Ok(path)
}

/// 敏感文本改投的附件名/类型（与 `MatrixAdapter` 保持一致）。
const SENSITIVE_FILE_NAME: &str = "batch_result.txt";
const SENSITIVE_FILE_MIME: &str = "text/plain";

/// 渲染 markup 后的最终外发文本（SimpleX 无 inline keyboard，按钮降级为文本命令）。
///
/// 敏感判定必须用**渲染后**文本：按钮 `data` 会被并入正文（`common/markup.rs`），
/// 只判 `content.text` 会漏掉「敏感内容只藏在按钮里」的情况。
fn outgoing_text(content: &MessageContent) -> String {
    match &content.markup {
        Some(markup) => render_markup_buttons(content.text.clone(), markup),
        None => content.text.clone(),
    }
}

/// 从发送响应中取第一条 chat item 的 itemId。
///
/// 响应不含 chat item 时返回错误：0 不是合法 SimpleX id，若把它当作
/// `MessageId("0")` 交给后续 edit/delete 会触发 panic/abort。
fn first_item_id(resp: &NewChatItemsResponse) -> Result<i64> {
    resp.chat_items
        .first()
        .map(|ci| ci.chat_item.meta.item_id)
        .context("SimpleX 发送响应缺少 chat item，无法确定消息 ID")
}

#[async_trait]
impl BotAdapter for SimplexAdapter {
    fn platform(&self) -> Platform {
        Platform::Simplex
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        let text = outgoing_text(&content);
        // 命中敏感判据 → 改投 `.txt` 附件，与 `MatrixAdapter` 对称：普通消息可被
        // 转发/复制且以纯文本落在消息列表，附件则不会（真机已确认敏感链接会以
        // 普通消息落到 SimpleX）。
        if is_sensitive(&text) {
            return self
                .send_file(
                    target,
                    SENSITIVE_FILE_NAME,
                    text.into_bytes(),
                    SENSITIVE_FILE_MIME,
                )
                .await;
        }
        let chat_id = parse_chat_id(target)?;
        let resp = self
            .bot
            .send_msg(chat_id, text)
            .await
            .context("发送 SimpleX 消息失败")?;
        Ok(MessageId(first_item_id(&resp)?.to_string()))
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

    async fn accept_contact_request(&self, contact_request_id: i64) -> Result<()> {
        let crid = parse_contact_request_id(contact_request_id)?;
        self.bot
            .accept_contact(crid)
            .await
            .context("接受 SimpleX 联系人请求失败")?;
        Ok(())
    }

    async fn reject_contact_request(&self, contact_request_id: i64) -> Result<()> {
        let crid = parse_contact_request_id(contact_request_id)?;
        self.bot
            .reject_contact(crid)
            .await
            .context("拒绝 SimpleX 联系人请求失败")?;
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
        Ok(MessageId(first_item_id(&resp)?.to_string()))
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
        Ok(MessageId(first_item_id(&resp)?.to_string()))
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

    // ── C1：敏感文本改投附件（判定对象必须是**渲染后**文本）──

    fn message_with_buttons(text: &str, buttons: &[(&str, &str)]) -> MessageContent {
        MessageContent {
            text: text.to_string(),
            markup: if buttons.is_empty() {
                None
            } else {
                Some(crate::common::Markup {
                    buttons: vec![
                        buttons
                            .iter()
                            .map(|(t, d)| crate::common::InlineButton {
                                text: t.to_string(),
                                data: d.to_string(),
                            })
                            .collect(),
                    ],
                })
            },
        }
    }

    #[test]
    fn outgoing_text_renders_markup_buttons_into_body() {
        let got = outgoing_text(&message_with_buttons("正文", &[("点我", "m_mon")]));
        assert!(got.contains("点我"), "按钮文案应并入正文: {got}");
        assert!(got.contains("m_mon"), "按钮 data 应并入正文: {got}");
    }

    #[test]
    fn sensitive_plain_text_is_flagged() {
        let got = outgoing_text(&message_with_buttons(
            "vless://abc@1.2.3.4:443?security=reality",
            &[],
        ));
        assert!(crate::common::routing::is_sensitive(&got));
    }

    /// 关键回归：敏感内容只藏在按钮 `data` 里时，判未渲染的 `content.text` 会漏判。
    #[test]
    fn sensitive_only_in_button_data_is_flagged() {
        let c = message_with_buttons(
            "点击复制订阅",
            &[("复制", "vless://abc@1.2.3.4:443?security=reality")],
        );
        assert!(
            crate::common::routing::is_sensitive(&outgoing_text(&c)),
            "按钮 data 里的敏感内容必须被判定为敏感"
        );
    }

    #[test]
    fn normal_text_is_not_flagged() {
        let got = outgoing_text(&message_with_buttons(
            "今天的天气不错",
            &[("菜单", "m_mon")],
        ));
        assert!(!crate::common::routing::is_sensitive(&got));
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
    fn sanitize_file_name_keeps_final_component() {
        assert_eq!(sanitize_file_name("a.txt").as_deref(), Some("a.txt"));
        assert_eq!(
            sanitize_file_name("dir/sub/b.txt").as_deref(),
            Some("b.txt")
        );
        assert_eq!(
            sanitize_file_name("..\\..\\c.txt").as_deref(),
            Some("c.txt")
        );
        assert_eq!(sanitize_file_name("/etc/passwd").as_deref(), Some("passwd"));
    }

    #[test]
    fn sanitize_file_name_rejects_traversal_and_empty() {
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name("."), None);
        assert_eq!(sanitize_file_name("/"), None);
        assert_eq!(sanitize_file_name(""), None);
        assert_eq!(sanitize_file_name("   "), None);
    }

    #[test]
    fn write_temp_file_names_are_unique() {
        let a = write_temp_file("same.txt", b"a").unwrap();
        let b = write_temp_file("same.txt", b"b").unwrap();
        assert_ne!(a, b);
        assert!(a.exists());
        assert!(b.exists());
        std::fs::remove_file(&a).unwrap();
        std::fs::remove_file(&b).unwrap();
    }

    #[test]
    fn write_temp_file_is_private() {
        let path = write_temp_file("unit.txt", b"x").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn temp_dir_is_private() {
        let path = write_temp_file("unit.txt", b"x").unwrap();
        let dir = std::env::temp_dir().join("aegis-simplex");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        std::fs::remove_file(&path).unwrap();
    }

    /// 空响应不得制造 id=0（旧实现 `unwrap_or(0)` 会返回 Ok(0)，
    /// 下游 edit/delete 再 `from_raw(0)` → panic，release 下 abort）。
    #[test]
    fn first_item_id_rejects_response_without_chat_items() {
        let empty: NewChatItemsResponse = serde_json::from_value(serde_json::json!({
            "user": {
                "userId": 1,
                "agentUserId": "1",
                "userContactId": 1,
                "localDisplayName": "aegis",
                "profile": {
                    "profileId": 1,
                    "displayName": "aegis",
                    "fullName": "aegis",
                    "localAlias": ""
                },
                "fullPreferences": {
                    "timedMessages": { "allow": "always" },
                    "fullDelete": { "allow": "always" },
                    "reactions": { "allow": "always" },
                    "voice": { "allow": "always" },
                    "files": { "allow": "always" },
                    "calls": { "allow": "always" },
                    "sessions": { "allow": "always" },
                    "commands": []
                },
                "activeOrder": 1
            },
            "chatItems": []
        }))
        .unwrap();

        assert!(first_item_id(&empty).is_err());
    }

    /// `MessageId("0")` 由 Matrix 文本命令路径合成（`gateways/matrix/commands.rs`），
    /// SimpleX 复用该路径，因此必须返回错误而不是 panic。
    #[test]
    fn parse_message_id_rejects_zero() {
        assert!(parse_message_id(&MessageId("0".to_string())).is_err());
    }

    #[test]
    fn parse_message_id_accepts_non_zero() {
        assert_eq!(
            parse_message_id(&MessageId("7".to_string())).unwrap().raw(),
            7
        );
    }

    #[test]
    fn parse_message_id_rejects_non_numeric() {
        assert!(parse_message_id(&MessageId("abc".to_string())).is_err());
    }

    #[test]
    fn parse_chat_id_rejects_zero() {
        assert!(parse_chat_id(&TargetId("0".to_string())).is_err());
    }

    #[test]
    fn parse_chat_id_accepts_non_zero() {
        assert_eq!(
            parse_chat_id(&TargetId("42".to_string())).unwrap().raw(),
            42
        );
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
