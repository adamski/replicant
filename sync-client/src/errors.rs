use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("UUID parsing error: {0}")]
    UuidParse(#[from] uuid::Error),
    
    #[error("Sync error: {0}")]
    Sync(#[from] sync_core::SyncError),
    
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    #[error("Connection lost")]
    ConnectionLost,
    
    #[error("Invalid state: {0}")]
    InvalidState(String),
    
    #[error("Date parsing error: {0}")]
    DateParse(#[from] chrono::ParseError),
    
    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("Failed to acquire lock: {0}")]
    LockError(String),

    #[error("Channel send failed")]
    SendError(#[from] std::sync::mpsc::SendError<crate::events::QueuedEvent>),

    #[error("Thread safety violation: process_events() must be called on the registration thread")]
    ThreadSafetyViolation,

    #[error("No callbacks registered yet")]
    NoCallbacksRegistered,

    #[error("Internal channel closed")]
    ChannelClosed,
}