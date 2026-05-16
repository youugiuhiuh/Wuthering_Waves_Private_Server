pub mod kcp;
pub mod schedule;
pub mod utils;
pub mod xray_config;

#[allow(dead_code)]
pub enum CallbackOutcome {
    Done,
    Redirect(String),
}
