use thiserror::Error;

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("Failed to apply patch: {0}")]
    PatchFailed(String),
    
    #[error("Document not found: {0}")]
    DocumentNotFound(uuid::Uuid),
    
    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: i64, actual: i64 },
    
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Database error: {0}")]
    DatabaseError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Conflict detected for document {0}")]
    ConflictDetected(uuid::Uuid),
    
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

impl From<serde_json::Error> for SyncError {
    fn from(err: serde_json::Error) -> Self {
        SyncError::SerializationError(err.to_string())
    }
}

impl From<uuid::Error> for SyncError {
    fn from(err: uuid::Error) -> Self {
        SyncError::SerializationError(err.to_string())
    }
}