use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;
use crate::AppState;

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
) -> Result<Json<RegisterResponse>, StatusCode> {
    // Hash password
    let password_hash = crate::auth::AuthState::hash_password(&req.password)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Create user
    let user_id = state.db.create_user(&req.email, &password_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(RegisterResponse { user_id }))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    // Verify email/password
    let user_id = state.auth.verify_user_password(&req.email, &req.password)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    
    // Check if user already has API keys
    let existing_keys = sqlx::query_scalar::<_, String>(
        "SELECT key_hash FROM api_keys WHERE user_id = $1 AND is_active = true LIMIT 1"
    )
    .bind(&user_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let api_key = if let Some(_) = existing_keys {
        // User already has API keys - don't create new one for login
        return Err(StatusCode::BAD_REQUEST); // Should use existing API key
    } else {
        // Create new API key for this login
        state.auth.create_api_key(&user_id, "Desktop Client API Key")
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    
    Ok(Json(LoginResponse { user_id, api_key }))
}

pub async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<CreateApiKeyResponse>, StatusCode> {
    // Create API key
    let api_key = state.auth.create_api_key(&req.user_id, &req.name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
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
) -> Result<impl IntoResponse, StatusCode> {
    // Verify API key
    let valid = state.auth.verify_token(&auth.user_id, &auth.auth_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }
    
    // Get documents
    let documents = state.db.get_user_documents(&auth.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(documents))
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(document_id): Path<Uuid>,
    Json(auth): Json<AuthHeader>,
) -> Result<impl IntoResponse, StatusCode> {
    // Verify API key
    let valid = state.auth.verify_token(&auth.user_id, &auth.auth_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }
    
    // Get document
    let document = state.db.get_document(&document_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    
    // Verify ownership
    if document.user_id != auth.user_id {
        return Err(StatusCode::FORBIDDEN);
    }
    
    Ok(Json(document))
}