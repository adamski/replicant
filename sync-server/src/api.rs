use crate::AppState;
use sync_core::{SyncResult, errors::ApiError};
use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateUserRequest {
    email: String,
}

#[derive(Serialize)]
pub struct CreateUserResponse {
    user_id: Uuid,
}

pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUserRequest>,
) -> SyncResult<Json<CreateUserResponse>> {
    // Create user with email only
    let user_id = state.db.create_user(&req.email)
        .await
        .map_err(|e| {
            tracing::error!(%e, "Failed to create user");

            // Check if it's a unique constraint violation (email already exists)
            if let sync_core::SyncError::DatabaseError(sqlx::Error::Database(ref db_err)) = e {
                // PostgreSQL unique violation code is 23505
                if db_err.code().as_deref() == Some("23505") {
                    return ApiError::conflict(
                        "User with this email already exists",
                        Some(format!("email: {}", req.email))
                    );
                }
            }

            // All other database errors are server errors (connection issues, etc.)
            ApiError::service_unavailable("Database temporarily unavailable")
        })?;

    Ok(Json(CreateUserResponse { user_id }))
}

// TODO: These REST endpoints need to be updated to use HMAC authentication
// Commented out temporarily to focus on WebSocket authentication

/*
pub async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApiKeyRequest>,
) -> SyncResult<Json<CreateApiKeyResponse>> {
    // Create API key
    let api_key = state.auth.create_api_key(&req.user_id, &req.name)
        .await
        .map_err(|e| {
            tracing::error!(%e, "Failed to create API key");
            ApiError::internal("Failed to create API key")
        })?;

    Ok(Json(CreateApiKeyResponse { api_key }))
}

#[derive(Deserialize)]
pub struct AuthHeader {
    user_id: Uuid,
    auth_token: String,
}

pub async fn list_documents(
    State(state): State<Arc<AppState>>,
    Json(auth): Json<AuthHeader>,
) -> SyncResult<impl IntoResponse> {
    // Verify API key
    let valid = state
        .auth
        .verify_token(&auth.user_id, &auth.token)
        .await
        .map_err(|_| ApiError::internal("Invalid Token"))?;

    if !valid {
        return Err(ApiError::unauthorized("Unauthorized Token"))?;
    }

    // Get documents
    let documents = state
        .db
        .get_user_documents(&auth.user_id)
        .await
        .map_err(|e| {
            tracing::error!(%e, "Failed to retrieve documents");
            ApiError::bad_request(
                "Invalid credentials",
                Some(format!(
                    "user_id: {}, auth_token: {}",
                    auth.user_id, auth.auth_token
                )),
            )
        })?;

    Ok(Json(documents))
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(document_id): Path<Uuid>,
    Json(auth): Json<AuthHeader>,
) -> SyncResult<impl IntoResponse> {
    // Verify API key
    let valid = state
        .auth
        .verify_token(&auth.user_id, &auth.auth_token)
        .await
        .map_err(|_| ApiError::internal("Invalid Token"))?;

    if !valid {
        return Err(ApiError::unauthorized("Unauthorized Token"))?;
    }

    // Get document
    let document = state.db.get_document(&document_id).await.map_err(|e| {
        tracing::error!(%e, "Failed to retrieve documents");
        ApiError::bad_request(
            "Invalid credentials",
            Some(format!(
                "user_id: {}, auth_token: {}, document: {}",
                auth.user_id, auth.auth_token, document_id
            )),
        )
    })?;

    // Verify ownership
    if document.user_id != auth.user_id {
        return Err(ApiError::unauthorized(""))?;
    }

    Ok(Json(document))
}
*/
