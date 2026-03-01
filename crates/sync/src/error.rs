//! Types d'erreurs du crate `sync`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Encode error: {0}")]
    Encode(String),

    #[error("Decrypt error")]
    Decrypt,

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Timeout")]
    Timeout,

    #[error("mDNS error: {0}")]
    Mdns(String),

    #[error("Sync channel closed")]
    ChannelClosed,

    #[error("Core error: {0}")]
    Core(ai_core::error::CoreError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Bincode error: {0}")]
    Bincode(String),
}

impl From<ai_core::error::CoreError> for SyncError {
    fn from(e: ai_core::error::CoreError) -> Self {
        SyncError::Core(e)
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;
