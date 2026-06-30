//! Telegram adapter — bot powered by `teloxide`.
//!
//! Handles Telegram-specific callback queries, inline keyboards,
//! and message formatting via the [`TelegramAdapter`].

pub mod adapter;
pub use adapter::TelegramAdapter;
