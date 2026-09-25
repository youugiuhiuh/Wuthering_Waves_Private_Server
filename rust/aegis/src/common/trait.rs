use crate::core::i18n::Lang;

use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct TargetId(pub String);

#[derive(Debug, Clone)]
pub struct MessageId(pub String);

#[derive(Debug, Clone)]
pub struct MessageContent {
    pub text: String,
    pub markup: Option<Markup>,
}

#[derive(Debug, Clone)]
pub struct Markup {
    pub buttons: Vec<Vec<InlineButton>>,
}

#[derive(Debug, Clone)]
pub struct InlineButton {
    pub text: String,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Telegram,
    Matrix,
    /// 不可改动：内部平台标识符。对外显示名是 `Aegis SimpleX Bot`（见
    /// `crate::main::simplex::simplex_bot_display_name`），与此处无关。
    /// 改名会波及 dispatch / routing 的全部分支匹配，属破坏性变更。
    Simplex,
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformCapabilities {
    pub can_edit_message: bool,
    pub can_delete_message: bool,
    pub has_inline_keyboard: bool,
    pub has_slash_commands: bool,
    pub has_file_transfer: bool,
    pub can_send_file: bool,
    pub can_send_image: bool,
    pub can_send_voice: bool,
    pub can_send_typing: bool,
    pub can_send_reaction: bool,
    pub can_thread: bool,
    pub has_e2ee: bool,
}

impl PlatformCapabilities {
    pub const TELEGRAM: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: true,
        has_slash_commands: true,
        has_file_transfer: true,
        can_send_file: true,
        can_send_image: true,
        can_send_voice: true,
        can_send_typing: true,
        can_send_reaction: true,
        can_thread: true,
        has_e2ee: false,
    };

    /// 必须与 `MatrixAdapter::capabilities()` 保持一致（Matrix 确实实现了
    /// `typing_notice` / `Annotation` / `Thread`）；此常量曾与 adapter 不一致。
    pub const MATRIX: Self = Self {
        can_edit_message: true,
        can_delete_message: true,
        has_inline_keyboard: false,
        has_slash_commands: false,
        has_file_transfer: true,
        can_send_file: true,
        can_send_image: true,
        can_send_voice: false,
        can_send_typing: true,
        can_send_reaction: true,
        can_thread: true,
        has_e2ee: true,
    };

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
}

#[mockall::automock]
#[async_trait]
pub trait BotAdapter: Send + Sync {
    fn platform(&self) -> Platform;
    async fn send_message(&self, target: &TargetId, content: MessageContent) -> Result<MessageId>;
    async fn edit_message(
        &self,
        target: &TargetId,
        msg_id: &MessageId,
        content: MessageContent,
    ) -> Result<()>;
    async fn delete_message(&self, target: &TargetId, msg_id: &MessageId) -> Result<()>;

    async fn answer_callback(
        &self,
        _target: &TargetId,
        _callback_id: &str,
        _text: Option<String>,
    ) -> Result<()> {
        Ok(())
    }

    async fn download_file(&self, _file_id: &str) -> Result<Vec<u8>> {
        anyhow::bail!("platform does not support file download")
    }

    /// Apply OS-level locale/system settings for a chosen language.
    /// Default no-op; only the Telegram adapter performs system operations.
    async fn set_system_locale(&self, _lang: Lang) -> Result<()> {
        Ok(())
    }

    fn capabilities(&self) -> PlatformCapabilities;

    /// Send a file as attachment. Returns the message ID.
    async fn send_file(
        &self,
        _target: &TargetId,
        _name: &str,
        _data: Vec<u8>,
        _mime: &str,
    ) -> Result<MessageId> {
        anyhow::bail!("platform does not support file sending")
    }

    /// Send an image. Delegates to send_file by default.
    async fn send_image(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.send_file(target, "image", data, mime).await
    }

    /// Send a voice recording. Delegates to send_file by default.
    async fn send_voice(&self, target: &TargetId, data: Vec<u8>, mime: &str) -> Result<MessageId> {
        self.send_file(target, "voice", data, mime).await
    }

    /// Set typing indicator. Default no-op.
    async fn send_typing(&self, _target: &TargetId, _active: bool) -> Result<()> {
        Ok(())
    }

    /// React to a message with an emoji. Default no-op.
    async fn send_reaction(
        &self,
        _target: &TargetId,
        _msg_id: &MessageId,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Send a message in a thread. Default: falls back to send_message.
    async fn send_message_threaded(
        &self,
        target: &TargetId,
        content: MessageContent,
        _thread_root: &str,
    ) -> Result<MessageId> {
        self.send_message(target, content).await
    }

    /// 批准一个 SimpleX 联系人请求（TG 带外审批 / 本地 CLI 用）。
    /// 默认不支持：只有 SimpleX 适配器实现；RoutingAdapter 转发到 secondary。
    async fn accept_contact_request(&self, _contact_request_id: i64) -> Result<()> {
        anyhow::bail!("当前平台不支持联系人审批")
    }

    /// 拒绝一个 SimpleX 联系人请求（对请求方静默）。
    async fn reject_contact_request(&self, _contact_request_id: i64) -> Result<()> {
        anyhow::bail!("当前平台不支持联系人审批")
    }

    /// 取 SimpleX 连接安全码（管理员在自己客户端「验证安全码」比对，防 MITM）。
    /// 默认不支持：只有 SimpleX 适配器实现。失败不得影响调用方的 TOTP 结果。
    async fn contact_security_code(&self, _contact_id: i64) -> Result<String> {
        anyhow::bail!("当前平台不支持获取连接安全码")
    }

    /// 强制经 primary 发送，绕开 `RoutingAdapter` 的敏感内容分流。
    ///
    /// 用于**安全通知**：其文本包含攻击者可控内容（如 SimpleX 显示名），若走
    /// `send_message`，一个名为 `vless://x` 的陌生人就能把审批提示改投到 secondary
    /// （SimpleX），使 TG 端收不到提示、按钮失效。默认等价于 `send_message`。
    async fn send_message_primary(
        &self,
        target: &TargetId,
        content: MessageContent,
    ) -> Result<MessageId> {
        self.send_message(target, content).await
    }

    /// 更新敏感内容落点（仅 `RoutingAdapter` 有实际实现；其余平台默认空操作）。
    ///
    /// 运行时重钉 `simplex_admin_id` 后必须调用：`RoutingAdapter` 的落点不是
    /// `AppState` 的镜像，否则敏感内容会一直发往启动快照里的旧 contactId。
    fn set_secondary_target(&self, _target: TargetId) {}
}

// MockBotAdapter is auto-generated by mockall above.
// answer_callback, download_file, send_file, send_image, send_voice,
// send_typing, send_reaction, send_message_threaded, accept_contact_request,
// reject_contact_request have default implementations so mockall does NOT require
// setting expectations for them.

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
        assert_ne!(Platform::Simplex, Platform::Matrix);
    }
}
