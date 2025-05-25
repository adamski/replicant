use uuid::Uuid;
use argon2::{
    password_hash::{
        rand_core::OsRng,
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString
    },
    Argon2
};
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    sessions: Arc<DashMap<Uuid, AuthSession>>,
}

#[derive(Clone)]
struct AuthSession {
    user_id: Uuid,
    token: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }
    
    pub fn hash_token(token: &str) -> Result<String, argon2::password_hash::Error> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2.hash_password(token.as_bytes(), &salt)?;
        Ok(password_hash.to_string())
    }
    
    pub fn verify_token_hash(token: &str, hash: &str) -> Result<bool, argon2::password_hash::Error> {
        let parsed_hash = PasswordHash::new(hash)?;
        let argon2 = Argon2::default();
        Ok(argon2.verify_password(token.as_bytes(), &parsed_hash).is_ok())
    }
    
    pub async fn verify_token(&self, user_id: &Uuid, token: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // In production, verify against database
        // For now, simple in-memory check
        if let Some(session) = self.sessions.get(user_id) {
            Ok(session.token == token)
        } else {
            Ok(false)
        }
    }
    
    pub fn create_session(&self, user_id: Uuid, token: String) -> Uuid {
        let session_id = Uuid::new_v4();
        self.sessions.insert(session_id, AuthSession {
            user_id,
            token,
            created_at: chrono::Utc::now(),
        });
        session_id
    }
    
    pub fn remove_session(&self, session_id: &Uuid) {
        self.sessions.remove(session_id);
    }
    
    pub fn generate_auth_token() -> String {
        Uuid::new_v4().to_string()
    }
}