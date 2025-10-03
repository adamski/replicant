use std::fmt::{Display, Formatter};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;
use tracing::{error};
use tracing::log::warn;
use sync_core::SyncError;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("{0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("{0}")]
    MigrationError(#[from] sqlx::migrate::MigrateError),

    #[error("{0}")]
    ApiError(#[from] ApiError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("argon2 Library Error: {0}")]
    HashingError(argon2::password_hash::Error),

    #[error("Server sync error: {0}")]
    ServerSync(String),

    #[error("Sync error: {0}")]
    SyncError(#[from] SyncError),
}

impl From<argon2::password_hash::Error> for ServerError {
    fn from(error: argon2::password_hash::Error) -> Self {
        ServerError::HashingError(error)
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApiError {
    InternalServerError(String),
    BadRequest(String, String),
    Unauthorized(String),
}


impl ApiError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::InternalServerError(message.into())
    }

    pub fn bad_request(message: impl Into<String>, meta: Option<String>) -> Self {
        Self::BadRequest(message.into(), meta.unwrap_or_else(|| "".to_string()).into())
    }
    
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }    

}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::InternalServerError(message) => write!(f, "Status=500, InternalServerError: {}", message),
            ApiError::BadRequest(message, meta) => write!(f, "Status=400, BadRequest: {}. {}", message, meta),
            ApiError::Unauthorized(message) => write!(f, "Status=403, Unauthorized: {}", message),
        }
    }
}
impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        #[derive(serde::Serialize)]
        struct ErrorResponse {
            message: String,
        }

        let (status, message) = match self {
            ServerError::ApiError(e) => {
                warn!("{}", e);
                match e {
                    ApiError::InternalServerError(message) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, message)
                    }
                    ApiError::BadRequest(message, _) => (StatusCode::BAD_REQUEST, message),
                    ApiError::Unauthorized (message) => (StatusCode::UNAUTHORIZED, message), 
                }
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "Unexpected Error".to_string())
        };

        (status, axum::Json(ErrorResponse {message})).into_response()
    }
}
