//! Runtime entry point orchestration.
//!
//! Bootstraps adapters, config, CLI, and the Matrix listener,
//! then starts the tokio runtime.

pub mod adapter;
pub mod cli;
pub mod config;
pub mod matrix;
pub mod runtime;
