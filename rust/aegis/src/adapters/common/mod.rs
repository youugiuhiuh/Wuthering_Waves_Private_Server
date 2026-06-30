//! Shared adapter types and the platform-agnostic [`BotAdapter`] trait.
//!
//! Includes [`RoutingAdapter`] which transparently routes sensitive messages
//! (e.g. proxy configs containing keys) to a secondary adapter.

pub mod r#trait;
pub use r#trait::*;
pub mod routing;
pub use routing::RoutingAdapter;
