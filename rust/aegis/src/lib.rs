extern crate self as aegis;

rust_i18n::i18n!("src/resources/i18n");

pub mod adapters;
pub mod app;
pub mod bootstrap;
pub mod core;
pub mod shared;
pub(crate) mod utils;
