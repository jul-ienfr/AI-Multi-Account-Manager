//! Crate `core` — types partagés et utilitaires pour AI Manager v3.

pub mod accounts;
pub mod capture;
pub mod config;
pub mod credentials;
pub mod error;
pub mod event_log;
pub mod models;
pub mod oauth;
pub mod profiles;
pub mod quota;
pub mod routing;
pub mod stats;
pub mod switch_controller;
pub mod types;
pub mod validator;
pub mod webhook;

pub use error::{CoreError, Result};
pub use credentials::GoogleOAuthSlot;
