//! Adapter layer — platform-agnostic interface for multiple chat platforms.
//!
//! Defines the [`BotAdapter`] trait and provides concrete implementations
//! for Telegram, Discord, and Matrix. All bot logic targets this trait
//! rather than any specific platform.

pub mod common;
pub mod telegram;

pub mod discord;
pub mod matrix;
