//! Application state and business logic flows.
//!
//! Manages auth sessions, self-destruct flows, batch operations,
//! and the shared [`AppState`] accessible across all handlers.

pub mod auth;
pub mod batch_handler;
pub mod destruct_flow;
pub mod state;
