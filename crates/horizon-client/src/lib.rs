//! Gameplay and startup, above the reusable client library.
#![recursion_limit = "256"]
#![forbid(unsafe_code)]
extern crate self as horizon_client;
pub use horizon_client_lib::*;
pub mod entry;
pub mod game;
pub mod mod_api;
