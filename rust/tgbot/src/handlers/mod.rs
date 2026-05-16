pub mod utils;

#[allow(dead_code)]
/// Result of a callback handler execution.
/// Done: handler completed, no further action needed.
/// Redirect: handler wants to re-dispatch with new callback data.
pub enum CallbackOutcome {
    Done,
    Redirect(String),
}
