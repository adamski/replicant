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
use crate::database::ServerDatabase;
use sync_core::SyncResult;

#[derive(Clone)]
pub struct AuthState {
    sessions: Arc<DashMap<Uuid, AuthSession>>,
    db: Arc<ServerDatabase>,
}

#[derive(Clone)]
struct AuthSession {
    #[allow(dead_code)]
    user_id: Uuid,
    token: String,
    #[allow(dead_code)]
    created_at: chrono::DateTime<chrono::Utc>,
}

impl AuthState {
    pub fn new(db: Arc<ServerDatabase>) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            db,
        }
    }
    
    pub fn hash_token(token: &str) -> SyncResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2.hash_password(token.as_bytes(), &salt)?;
        Ok(password_hash.to_string())
    }
    
    pub fn verify_token_hash(token: &str, hash: &str) -> SyncResult<bool> {
        let parsed_hash = PasswordHash::new(hash)?;
        let argon2 = Argon2::default();
        Ok(argon2.verify_password(token.as_bytes(), &parsed_hash).is_ok())
    }
    
    pub async fn verify_token(&self, user_id: &Uuid, token: &str) -> SyncResult<bool> {
        tracing::debug!("Verifying token for user {}: token={}", user_id, token);
        
        // First check if we have an active session
        // But for demo-token, we always need to ensure the user exists in DB
        if token != "demo-token" {
            if let Some(session) = self.sessions.get(user_id) {
                if session.token == token {
                    tracing::debug!("Found active session for user {}", user_id);
                    return Ok(true);
                }
            }
        }
        
        // Special handling for demo token - auto-create user if needed
        if token == "demo-token" {
            tracing::debug!("Demo token detected for user {}", user_id);
            
            // Check if user exists
            let exists = sqlx::query_scalar!(
                "SELECT COUNT(*) FROM users WHERE id = $1",
                user_id
            )
            .fetch_one(&self.db.pool)
            .await?;
            
            tracing::debug!("User {} exists: {}", user_id, exists.unwrap_or(0) > 0);

            if exists.unwrap_or(0) == 0 {
                // Create demo user with this ID
                let demo_hash = Self::hash_token("demo-token")?;
                
                tracing::info!("Creating demo user with ID: {} and email: demo_{}@example.com", user_id, user_id);

                let email = format!("demo_{}@example.com", user_id);
                match sqlx::query!(
                    "INSERT INTO users (id, email, auth_token_hash, created_at) VALUES ($1, $2, $3, NOW())",
                    user_id,
                    email,
                    demo_hash
                )
                .execute(&self.db.pool)
                .await {
                    Ok(_) => {
                        tracing::info!("Successfully created demo user with ID: {}", user_id);
                    }
                    Err(e) => {
                        // Check if this is a duplicate key error (race condition)
                        let error_string = e.to_string();
                        if error_string.contains("duplicate key") || error_string.contains("unique constraint") {
                            tracing::debug!("Demo user {} was created by another client concurrently - this is expected", user_id);
                            // This is fine - another client created the user simultaneously
                            // Continue with session creation
                        } else {
                            tracing::error!("Failed to create demo user with unexpected error: {}", e);
                            return Err(e)?;
                        }
                    }
                }
            } else {
                tracing::debug!("Demo user {} already exists in database", user_id);
            }
            
            // Create session
            self.sessions.insert(*user_id, AuthSession {
                user_id: *user_id,
                token: token.to_string(),
                created_at: chrono::Utc::now(),
            });
            
            return Ok(true);
        }
        
        // Otherwise, verify against database
        let result = sqlx::query!(
            "SELECT auth_token_hash FROM users WHERE id = $1",
            user_id
        )
        .fetch_optional(&self.db.pool)
        .await?;
        
        if let Some(row) = result {
            // Verify the token against the stored hash
            match Self::verify_token_hash(token, &row.auth_token_hash) {
                Ok(true) => {
                    // Create a new session for future requests
                    self.sessions.insert(*user_id, AuthSession {
                        user_id: *user_id,
                        token: token.to_string(),
                        created_at: chrono::Utc::now(),
                    });
                    
                    // Update last seen
                    sqlx::query!(
                        "UPDATE users SET last_seen_at = NOW() WHERE id = $1",
                        user_id
                    )
                    .execute(&self.db.pool)
                    .await?;
                    
                    return Ok(true);
                }
                Ok(false) => return Ok(false),
                Err(_) => return Ok(false), // Invalid hash format
            }
        } else {
            // TODO: Implement proper auto-registration for all authenticated users
            // Currently, only demo-token users are auto-created. In a production system,
            // we should auto-register any user with valid authentication credentials
            // (e.g., JWT from auth provider, API key, etc.) to support true user auto-registration.
            // This would eliminate foreign key constraint violations when new users connect.
            
            tracing::debug!("User {} not found in database, attempting auto-registration", user_id);
            
            // User doesn't exist - auto-register if we have a valid token
            // In a real system, you'd validate the token format or check against an auth provider
            // For now, we'll accept any non-empty token and create the user
            if !token.is_empty() {
                let token_hash = Self::hash_token(token)?;

                // Create user with provided ID
                let email = format!("user_{}@example.com", user_id);
                match sqlx::query!(
                    "INSERT INTO users (id, email, auth_token_hash, created_at) VALUES ($1, $2, $3, NOW())",
                    user_id,
                    email,
                    token_hash
                )
                .execute(&self.db.pool)
                .await {
                    Ok(_) => {
                        tracing::info!("Auto-registered new user with ID: {}", user_id);
                        
                        // Create session
                        self.sessions.insert(*user_id, AuthSession {
                            user_id: *user_id,
                            token: token.to_string(),
                            created_at: chrono::Utc::now(),
                        });
                        
                        return Ok(true);
                    }
                    Err(e) => {
                        tracing::error!("Failed to auto-register user: {}", e);
                        return Ok(false);
                    }
                }
            }
        }
        
        Ok(false)
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