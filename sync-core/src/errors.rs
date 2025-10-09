use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::fmt::{Display, Formatter};
use chrono::ParseError;
use thiserror::Error;
use tracing::log::warn;
use crate::protocol;
use tokio::sync::mpsc::error::SendError;
#[derive(Error, Debug)]
#[non_exhaustive]
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
    DatabaseError(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("UUID parsing error: {0}")]
    UuidParse(#[from] uuid::Error),

    #[error("Conflict detected for document {0}")]
    ConflictDetected(uuid::Uuid),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Date parsing error: {0}")]
    DateParse(#[from] ParseError),

    #[error("Migration error: {0}")]
    MigrationError(#[from] sqlx::migrate::MigrateError),

    #[error("Client error: {0}")]
    Client(#[from] ClientError),

    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ServerError {
    #[error("{0}")]
    ApiError(#[from] ApiError),

    #[error("argon2 Library Error: {0}")]
    HashingError(argon2::password_hash::Error),

    #[error("Server sync error: {0}")]
    ServerSync(String),

    #[error("Server Channel send failed: {0}")]
    SendError(#[from] SendError<protocol::ServerMessage>)
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ClientError {
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Connection lost")]
    ConnectionLost,

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Channel send failed: {0}")]
    SendError(String),

    #[error("Failed to acquire {0} lock")]
    LockError(String),

    #[error("Thread safety violation: process_events() must be called on the registration thread")]
    ThreadSafetyViolation,

    #[error("No callbacks registered yet")]
    NoCallbacksRegistered,

    #[error("Internal channel closed")]
    ChannelClosed,
}





impl From<argon2::password_hash::Error> for SyncError {
    fn from(error: argon2::password_hash::Error) -> Self {
        SyncError::Server(ServerError::HashingError(error))
    }
}



#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApiError {
    InternalServerError(String),
    BadRequest(String, Option<String>),
    Unauthorized(String),
}

impl ApiError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::InternalServerError(message.into())
    }

    pub fn bad_request(message: impl Into<String>, meta: Option<String>) -> Self {
        Self::BadRequest(
            message.into(),
            meta.unwrap_or_else(|| "".to_string()).into(),
        )
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::InternalServerError(message) => {
                write!(f, "Status=500, InternalServerError: {}", message)
            }
            ApiError::BadRequest(message, meta) => {
                write!(f, "Status=400, BadRequest: {}. {}", message, meta.clone().unwrap_or_default())
            }
            ApiError::Unauthorized(message) => write!(f, "Status=403, Unauthorized: {}", message),
        }
    }
}
impl IntoResponse for SyncError {
    fn into_response(self) -> Response {
        #[derive(serde::Serialize)]
        struct ErrorResponse {
            message: String,
        }

        let (status, message) = match self {
            SyncError::Server(ServerError::ApiError(e)) => {
                warn!("{}", e);
                match e {
                    ApiError::InternalServerError(message) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, message)
                    }
                    ApiError::BadRequest(message, _) => (StatusCode::BAD_REQUEST, message),
                    ApiError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
                }
            }
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unexpected Error".to_string(),
            ),
        };

        (status, axum::Json(ErrorResponse { message })).into_response()
    }
}

impl From<ApiError> for SyncError {
    fn from(value: ApiError) -> Self {
        SyncError::Server(value.into())
    }
}

impl From<SendError<protocol::ServerMessage>> for SyncError {
    fn from(value: SendError<protocol::ServerMessage>) -> Self {
        SyncError::Server(value.into())
    }
}