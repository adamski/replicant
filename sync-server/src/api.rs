use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;
use crate::{AppState, auth::AuthState};

#[derive(Deserialize)]
pub struct RegisterRequest {
    email: String,
    #[allow(dead_code)]
    password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    user_id: Uuid,
    auth_token: String,
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    // Generate auth token
    let auth_token = AuthState::generate_auth_token();
    
    // Hash the token for storage
    let token_hash = AuthState::hash_token(&auth_token)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Create user
    let user_id = state.db.create_user(&req.email, &token_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Create session
    state.auth.create_session(user_id, auth_token.clone());
    
    Ok(Json(RegisterResponse {
        user_id,
        auth_token,
    }))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    user_id: Uuid,
    auth_token: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    session_id: Uuid,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    // Verify token
    let valid = state.auth.verify_token(&req.user_id, &req.auth_token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }
    
    // Create session
    let session_id = state.auth.create_session(req.user_id, req.auth_token);
    
    Ok(Json(LoginResponse { session_id }))
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
    // Verify auth
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
    // Verify auth
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