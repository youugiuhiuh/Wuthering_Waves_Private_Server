use crate::common::markup::render_markup_buttons;
use crate::common::{
    BotAdapter, MessageContent, MessageId, Platform, PlatformCapabilities, TargetId,
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub struct RoutingAdapter {
    primary: Arc<dyn BotAdapter>,
    secondary: Option<Arc<dyn BotAdapter>>,
    /// secondary 的发送目标覆盖。SimpleX 适配器会用 `parse_chat_id(target)` 解读目标
    /// （`gateways/simplex/adapter.rs`），而 Matrix 适配器忽略目标、固定发往自己的 room
    /// （`gateways/matrix/adapter.rs`）。只有需要改写的 secondary（TG + SimpleX）才设置它。
    ///
    /// `Mutex`：运行时重钉 `simplex_admin_id` 后要能就地改（`set_secondary_target`），
    /// 否则敏感内容会一直发往启动快照里的旧 contactId。
    secondary_target: Mutex<Option<TargetId>>,
}

impl RoutingAdapter {
    pub fn new(primary: Arc<dyn BotAdapter>, secondary: Option<Arc<dyn BotAdapter>>) -> Self {
        Self {
            primary,
            secondary,
            secondary_target: Mutex::new(None),
        }
    }

    /// 覆盖 secondary 的发送目标；未设置时沿用调用方传入的 target。
    pub fn with_secondary_target(mut self, target: TargetId) -> Self {
        self.secondary_target = Mutex::new(Some(target));
        self
    }
}

pub(crate) fn is_sensitive(text: &str) -> bool {
    const PROTOCOLS: &[&str] = &[
        "vmess://",
        "vless://",
        "trojan://",
        "ss://",
        "hysteria://",
        "hysteria2://",
        "tuic://",
    ];
    const KEY_FIELDS: &[&str] = &["\"privateKey\"", "\"secretKey\"", "\"password\":"];
    // Trailing colon on "password" reduces false-positives like "invalid password"

    PROTOCOLS.iter().any(|p| text.contains(p)) || KEY_FIELDS.iter().any(|k| text.contains(k))
}

#[async_trait]
impl BotAdapter for RoutingAdapter {
    fn platform(&self) -> Platform {
        self.primary.platform()
    }

    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId> {
        // 判定用**渲染后**文本：按钮 `data` 会被并入正文（`common/markup.rs`），
        // 只判 `content.text` 会让「敏感内容只藏在按钮里」的消息按 primary 发出。
        let routed_text = match &content.markup {
            Some(markup) => render_markup_buttons(content.text.clone(), markup),
            None => content.text.clone(),
        };
        match &self.secondary {
            Some(secondary) if is_sensitive(&routed_text) => {
                // 先取锁并 clone，尽早释放 guard（勿跨 await 持锁）。
                let secondary_target = self
                    .secondary_target
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or(None)
                    .unwrap_or_else(|| target.clone());
                match secondary.send_message(&secondary_target, content).await {
                    Ok(msg) => Ok(msg),
                    Err(e) => {
                        // 静默失败会让敏感内容「两边都收不到」且无从排查（真机已踩）。
                        log::error!(
                            "敏感内容改投 secondary 失败（target={}）: {e}",
                            secondary_target.0
                        );
                        Err(e)
                    }
                }
            }
            _ => self.primary.send_message(target, content).await,
        }
    }

    /// 运行时重钉后刷新敏感内容落点（见 `BotAdapter::set_secondary_target`）。
    fn set_secondary_target(&self, target: TargetId) {
        match self.secondary_target.lock() {
            Ok(mut g) => *g = Some(target),
            Err(e) => log::error!("更新敏感内容落点失败（锁中毒）: {e}"),
        }
    }

    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()> {
        self.primary.edit_message(target, msg_id, content).await
    }

    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()> {
        self.primary.delete_message(target, msg_id).await
    }

    async fn answer_callback(
        &self,
        target: &TargetId,
        callback_id: &str,
        text: Option<String>,
    ) -> Result<()> {
        self.primary
            .answer_callback(target, callback_id, text)
            .await
    }

    async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        self.primary.download_file(file_id).await
    }

    fn capabilities(&self) -> PlatformCapabilities {
        self.primary.capabilities()
    }

    async fn send_file(
        &self,
        target: &TargetId,
        name: &str,
        data: Vec<u8>,
        mime: &str,
    ) -> Result<MessageId> {
        self.primary.send_file(target, name, data, mime).await
    }

    async fn send_image(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.primary.send_image(target, data, mime).await
    }

    async fn send_voice(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.primary.send_voice(target, data, mime).await
    }

    async fn send_typing(&self, target: &TargetId, active: bool) -> Result<()> {
        self.primary.send_typing(target, active).await
    }

    async fn send_reaction(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        emoji: &str,
    ) -> Result<()> {
        self.primary.send_reaction(target, msg_id, emoji).await
    }

    async fn send_message_threaded(
        &self,
        target: &TargetId,
        content: MessageContent,
        thread_root: &str,
    ) -> Result<MessageId> {
        self.primary
            .send_message_threaded(target, content, thread_root)
            .await
    }

    /// TG + SimpleX 形态下，审批动作落在 **secondary**（SimpleX）上，不是 primary（TG）。
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

    /// 安全码属于 SimpleX 连接：必须落到 **secondary**，不是 primary（TG）。
    /// 否则 `--tg-simplex` 下（TOTP 通常来自 TG）永远取不到码。
    async fn contact_security_code(&self, contact_id: i64) -> Result<String> {
        match &self.secondary {
            Some(secondary) => secondary.contact_security_code(contact_id).await,
            None => anyhow::bail!("未配置 SimpleX 次级适配器，无法获取连接安全码"),
        }
    }

    /// 安全通知必须落在 primary（TG），不得因攻击者可控文本被改投 secondary。
    async fn send_message_primary(
        &self,
        target: &TargetId,
        content: MessageContent,
    ) -> Result<MessageId> {
        self.primary.send_message(target, content).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::MockBotAdapter;

    #[tokio::test]
    async fn routing_forwards_contact_approval_to_secondary() {
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
    async fn routing_forwards_contact_rejection_to_secondary() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        let mut secondary = MockBotAdapter::new();
        secondary
            .expect_reject_contact_request()
            .with(mockall::predicate::eq(6))
            .times(1)
            .returning(|_| Ok(()));
        let adapter = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
        adapter.reject_contact_request(6).await.unwrap();
    }

    #[tokio::test]
    async fn routing_contact_approval_without_secondary_errors() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        let adapter = RoutingAdapter::new(Arc::new(primary), None);
        assert!(adapter.accept_contact_request(5).await.is_err());
        assert!(adapter.reject_contact_request(5).await.is_err());
    }

    #[tokio::test]
    async fn set_secondary_target_redirects_sensitive_content() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        // 敏感内容绝不应落到 primary。
        primary.expect_send_message().times(0);
        let mut secondary = MockBotAdapter::new();
        secondary.expect_platform().returning(|| Platform::Simplex);
        secondary
            .expect_send_message()
            .withf(|t, c| t.0 == "9" && c.text.contains("vless://"))
            .times(1)
            .returning(|_, _| Ok(MessageId("1".into())));
        let adapter = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
            .with_secondary_target(TargetId("1".into()));
        // 模拟运行时重钉：落点从 1 改为 9。
        adapter.set_secondary_target(TargetId("9".into()));
        adapter
            .send_message(
                &TargetId("42".into()),
                MessageContent {
                    text: "vless://x".into(),
                    markup: None,
                },
            )
            .await
            .unwrap();
    }

    /// 敏感内容发送失败不得被 RoutingAdapter 吞掉（真机曾因落点无效静默丢弃）。
    #[tokio::test]
    async fn sensitive_send_failure_is_propagated() {
        let mut primary = MockBotAdapter::new();
        primary.expect_platform().returning(|| Platform::Telegram);
        let mut secondary = MockBotAdapter::new();
        secondary.expect_platform().returning(|| Platform::Simplex);
        secondary
            .expect_send_message()
            .times(1)
            .returning(|_, _| anyhow::bail!("no such contact"));
        let adapter = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
            .with_secondary_target(TargetId("1".into()));
        let res = adapter
            .send_message(
                &TargetId("42".into()),
                MessageContent {
                    text: "vless://x".into(),
                    markup: None,
                },
            )
            .await;
        assert!(res.is_err(), "失败必须回传，不能静默成功");
    }

    #[test]
    fn is_sensitive_detects_vmess() {
        assert!(is_sensitive("vmess://abc123"));
    }

    #[test]
    fn is_sensitive_detects_vless() {
        assert!(is_sensitive("vless://abc123"));
    }

    #[test]
    fn is_sensitive_detects_trojan() {
        assert!(is_sensitive("trojan://abc123"));
    }

    #[test]
    fn is_sensitive_detects_ss() {
        assert!(is_sensitive("ss://abc123"));
    }

    #[test]
    fn is_sensitive_detects_hysteria() {
        assert!(is_sensitive("hysteria://abc123"));
    }

    #[test]
    fn is_sensitive_detects_tuic() {
        assert!(is_sensitive("tuic://abc123"));
    }

    #[test]
    fn is_sensitive_detects_private_key() {
        assert!(is_sensitive("{\"privateKey\":\"abc\"}"));
    }

    #[test]
    fn is_sensitive_detects_secret_key() {
        assert!(is_sensitive("{\"secretKey\":\"abc\"}"));
    }

    #[test]
    fn is_sensitive_rejects_normal_text() {
        assert!(!is_sensitive("系统状态：运行中"));
        assert!(!is_sensitive("Hello world"));
    }

    #[test]
    fn is_sensitive_rejects_empty() {
        assert!(!is_sensitive(""));
    }

    #[test]
    fn is_sensitive_rejects_partial_prefix() {
        assert!(!is_sensitive("vmess is a protocol"));
    }

    #[test]
    fn is_sensitive_detects_hysteria2() {
        assert!(is_sensitive("hysteria2://abc123"));
    }

    mod routing_tests {
        use super::*;
        use crate::common::MockBotAdapter;

        #[tokio::test]
        async fn sends_sensitive_to_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_send_message().never();

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Matrix);
            secondary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "vless://abc123".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn sends_normal_to_primary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Matrix);
            secondary.expect_send_message().never();

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "normal system message".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn sends_all_to_primary_when_no_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), None);
            routing
                .send_message(
                    &TargetId("1".to_string()),
                    MessageContent {
                        text: "vless://any-content".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn sensitive_uses_secondary_target_override() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_send_message().never();

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary
                .expect_send_message()
                .times(1)
                .withf(|target, _| target.0 == "999")
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
                .with_secondary_target(TargetId("999".to_string()));
            routing
                .send_message(
                    &TargetId("42".to_string()),
                    MessageContent {
                        text: "vless://abc123".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn normal_message_keeps_original_target() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary
                .expect_send_message()
                .times(1)
                .withf(|target, _| target.0 == "42")
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary.expect_send_message().never();

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
                .with_secondary_target(TargetId("999".to_string()));
            routing
                .send_message(
                    &TargetId("42".to_string()),
                    MessageContent {
                        text: "normal system message".into(),
                        markup: None,
                    },
                )
                .await
                .unwrap();
        }

        /// 安全码属于 SimpleX 连接：RoutingAdapter 必须落到 **secondary**，
        /// 否则 `--tg-simplex` 下（事件来自 TG）永远取不到码。
        #[tokio::test]
        async fn contact_security_code_forwards_to_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_contact_security_code().times(0);

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary
                .expect_contact_security_code()
                .times(1)
                .withf(|id| *id == 3)
                .returning(|_| Ok("52075 05398".to_string()));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)));
            assert_eq!(
                routing.contact_security_code(3).await.unwrap(),
                "52075 05398"
            );
        }

        #[tokio::test]
        async fn contact_security_code_without_secondary_errors() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            let routing = RoutingAdapter::new(Arc::new(primary), None);
            assert!(routing.contact_security_code(3).await.is_err());
        }

        /// C1b 回归：敏感内容只藏在按钮 `data` 里时，只判未渲染的 `content.text` 会漏判
        /// → 敏感内容按 primary(TG) 发出。判定必须用**渲染后**文本。
        #[tokio::test]
        async fn sensitive_only_in_button_data_uses_secondary() {
            let mut primary = MockBotAdapter::new();
            primary.expect_platform().returning(|| Platform::Telegram);
            primary.expect_send_message().never();

            let mut secondary = MockBotAdapter::new();
            secondary.expect_platform().returning(|| Platform::Simplex);
            secondary
                .expect_send_message()
                .times(1)
                .returning(|_, _| Ok(MessageId("1".to_string())));

            let routing = RoutingAdapter::new(Arc::new(primary), Some(Arc::new(secondary)))
                .with_secondary_target(TargetId("999".to_string()));
            routing
                .send_message(
                    &TargetId("42".to_string()),
                    MessageContent {
                        text: "点击复制订阅".into(),
                        markup: Some(crate::common::Markup {
                            buttons: vec![vec![crate::common::InlineButton {
                                text: "复制".into(),
                                data: "vless://abc123".into(),
                            }]],
                        }),
                    },
                )
                .await
                .unwrap();
        }
    }
}
