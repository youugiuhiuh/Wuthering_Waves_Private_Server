extern crate self as aegis;

rust_i18n::i18n!("src/resources/i18n");

pub mod app;
pub mod bootstrap;
pub mod common;
pub mod core;
pub mod gateways;
pub mod shared;
pub(crate) mod utils;
