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
use rand::Rng;
use crate::database::ServerDatabase;
use sync_core::SyncResult;

#[derive(Clone)]
pub struct AuthState {
    sessions: Arc<DashMap<Uuid, AuthSession>>,
    db: Arc<ServerDatabase>,
}

#[derive(Clone)]
struct AuthSession {
    user_id: Uuid,
    token: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl AuthState {
    pub fn new(db: Arc<ServerDatabase>) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            db,
        }
    }

    pub fn hash_password(password: &str) -> SyncResult<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2.hash_password(password.as_bytes(), &salt)?;
        Ok(password_hash.to_string())
    }

    pub fn verify_password(password: &str, hash: &str) -> SyncResult<bool> {
        let parsed_hash = PasswordHash::new(hash)?;
        let argon2 = Argon2::default();
        Ok(argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok())
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

    pub async fn verify_token(&self, user_id: &Uuid, api_key: &str) -> SyncResult<bool> {
        tracing::debug!("Verifying API key for user {}: key={}", user_id, &api_key[..std::cmp::min(10, api_key.len())]);

        // Must be an API key
        if !api_key.starts_with("sk_") {
            tracing::warn!("Invalid token format - must start with sk_");
            return Ok(false);
        }

        // Check if we have an active session for this API key
        if let Some(session) = self.sessions.get(user_id) {
            if session.token == api_key {
                tracing::debug!("Found active session for user {}", user_id);
                return Ok(true);
            }
        }

        // Verify API key and get user_id
        match self.verify_api_key(api_key).await? {
            Some(api_user_id) => {
                if api_user_id == *user_id {
                    // Create session
                    self.sessions.insert(*user_id, AuthSession {
                        user_id: *user_id,
                        token: api_key.to_string(),
                        created_at: chrono::Utc::now(),
                    });

                    // Update user last seen
                    sqlx::query(
                        "UPDATE users SET last_seen_at = NOW() WHERE id = $1"
                    )
                    .bind(user_id)
                    .execute(&self.db.pool)
                    .await?;

                    return Ok(true);
                } else {
                    tracing::warn!("API key user_id {} does not match provided user_id {}", api_user_id, user_id);
                    return Ok(false);
                }
            }
            None => {
                tracing::warn!("Invalid API key provided");
                return Ok(false);
            }
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

    pub fn generate_api_key() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 32] = rng.gen();
        format!("sk_{}", hex::encode(random_bytes))
    }


    pub async fn create_api_key(&self, user_id: &Uuid, name: &str) -> SyncResult<String> {
        let api_key = Self::generate_api_key();
        let key_hash = Self::hash_token(&api_key)?;

        sqlx::query(
            "INSERT INTO api_keys (user_id, key_hash, name) VALUES ($1, $2, $3)"
        )
        .bind(user_id)
        .bind(&key_hash)
        .bind(name)
        .execute(&self.db.pool)
        .await?;

        Ok(api_key)
    }

    pub async fn verify_api_key(&self, api_key: &str) -> SyncResult<Option<Uuid>> {
        // Get all active API keys and verify one by one
        let api_keys = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT user_id, key_hash FROM api_keys WHERE is_active = true"
        )
        .fetch_all(&self.db.pool)
        .await?;

        for (user_id, key_hash) in api_keys {
            if Self::verify_token_hash(api_key, &key_hash).unwrap_or(false) {
                return Ok(Some(user_id));
            }
        }

        Ok(None)
    }

    pub async fn verify_user_password(&self, email: &str, password: &str) -> SyncResult<Option<Uuid>> {
        match self.db.verify_user_password(email).await? {
            Some((user_id, password_hash)) => {
                if Self::verify_password(password, &password_hash).unwrap_or(false) {
                    return Ok(Some(user_id));
                }
            }
            None => return Ok(None)
        }
        Ok(None)
    }
}
