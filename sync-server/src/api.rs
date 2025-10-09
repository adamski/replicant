use crate::{auth::AuthState, AppState};
use sync_core::{SyncResult, errors::ApiError};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct RegisterRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    user_id: Uuid,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    user_id: Uuid,
    api_key: String,
}

#[derive(Deserialize)]
pub struct CreateApiKeyRequest {
    user_id: Uuid,
    name: String,
}

#[derive(Serialize)]
pub struct CreateApiKeyResponse {
    api_key: String,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> SyncResult<Json<RegisterResponse>> {
    // Hash password
    let password_hash = AuthState::hash_password(&req.password)
        .map_err(|_| ApiError::internal("Failed to hash password"))?;

    // Create user
    let user_id = state.db.create_user(&req.email, &password_hash)
        .await
        .map_err(|e| {
            tracing::error!(%e, "Failed to create user");
            ApiError::bad_request(
                "Failed to create user",
                Some(format!("email: {}", req.email)),
            )
        })?;

    Ok(Json(RegisterResponse { user_id }))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> SyncResult<Json<LoginResponse>> {
    // Verify email/password
    let user_id = state.auth.verify_user_password(&req.email, &req.password)
        .await
        .map_err(|_| ApiError::internal("Authentication failed"))?
        .ok_or_else(|| ApiError::unauthorized("Invalid credentials"))?;

    // Check if user already has API keys
    let existing_keys = sqlx::query_scalar::<_, String>(
        "SELECT key_hash FROM api_keys WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(&user_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|e| {
        tracing::error!(%e, "Failed to check existing API keys");
        ApiError::internal("Database error")
    })?;

    let api_key = if existing_keys.is_some() {
        // User already has API keys - don't create new one for login
        return Err(ApiError::bad_request("User already has API keys", None))?;
    } else {
        // Create new API key for this login
        state.auth.create_api_key(&user_id, "Desktop Client API Key")
            .await
            .map_err(|e| {
                tracing::error!(%e, "Failed to create API key");
                ApiError::internal("Failed to create API key")
            })?
    };

    Ok(Json(LoginResponse { user_id, api_key }))
}

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
        .verify_token(&auth.user_id, &auth.auth_token)
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
