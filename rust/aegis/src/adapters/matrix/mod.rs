//! Matrix adapter — bot powered by `matrix-sdk`.
//!
//! Handles Matrix-specific room events and message formatting
//! via the [`MatrixAdapter`].

pub mod adapter;
pub mod commands;
pub use adapter::MatrixAdapter;
