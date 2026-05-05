//! Telegram Bot 消息与回调处理模块
//!
//! 包含以下子模块:
//! - [`callback`] - 回调查询分派与处理
//! - [`message`] - 普通消息处理
//! - [`command`] - 命令处理与 TOTP 认证
//! - [`proxy`] - 代理配置生成 UI
//! - [`security`] - 安全相关功能
//! - [`system`] - 系统管理与定时任务 UI

pub mod callback;
pub mod message;
pub mod command;
pub mod proxy;
pub mod security;
pub mod system;

pub use tgbot::core::utils::format_duration_human;
