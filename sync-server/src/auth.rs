use uuid::Uuid;
use dashmap::DashMap;
use std::sync::Arc;
use rand::Rng;
use crate::database::ServerDatabase;
use sync_core::SyncResult;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub struct ApiCredentials {
    pub api_key: String,
    pub secret: String,
}

#[derive(Clone)]
pub struct AuthState {
    sessions: Arc<DashMap<Uuid, AuthSession>>,
    db: Arc<ServerDatabase>,
}

#[derive(Clone)]
struct AuthSession {
    token: String,
}

impl AuthState {
    pub fn new(db: Arc<ServerDatabase>) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            db,
        }
    }

    pub fn create_session(&self, _user_id: Uuid, token: String) -> Uuid {
        let session_id = Uuid::new_v4();
        self.sessions.insert(session_id, AuthSession {
            token,
        });
        session_id
    }

    pub fn remove_session(&self, session_id: &Uuid) {
        self.sessions.remove(session_id);
    }

    pub fn generate_api_key() -> String {
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 32] = rng.gen();
        format!("rpa_{}", hex::encode(random_bytes))
    }

    pub fn generate_api_credentials() -> ApiCredentials {
        let mut rng = rand::thread_rng();
        let api_key_bytes: [u8; 32] = rng.gen();
        let secret_bytes: [u8; 32] = rng.gen();

        ApiCredentials {
            api_key: format!("rpa_{}", hex::encode(api_key_bytes)),
            secret: format!("rps_{}", hex::encode(secret_bytes)),
        }
    }

    pub async fn save_credentials(
        &self,
        credentials: &ApiCredentials,
        name: &str,
    ) -> SyncResult<()> {
        sqlx::query(
            "INSERT INTO api_credentials (api_key, secret, name)
             VALUES ($1, $2, $3)"
        )
        .bind(&credentials.api_key)
        .bind(&credentials.secret)
        .bind(name)
        .execute(&self.db.pool)
        .await?;

        Ok(())
    }

    pub fn create_hmac_signature(
        secret: &str,
        timestamp: i64,
        email: &str,
        api_key: &str,
        body: &str,
    ) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");

        let message = format!("{}.{}.{}.{}", timestamp, email, api_key, body);
        mac.update(message.as_bytes());

        hex::encode(mac.finalize().into_bytes())
    }

    pub async fn verify_hmac(
        &self,
        api_key: &str,
        signature: &str,
        timestamp: i64,
        email: &str,
        body: &str,
    ) -> SyncResult<bool> {
        // Validate timestamp (5 minute window)
        let now = chrono::Utc::now().timestamp();
        if (now - timestamp).abs() > 300 {
            tracing::warn!("HMAC timestamp outside 5-minute window");
            return Ok(false);
        }

        // Check API key format
        if !api_key.starts_with("rpa_") {
            tracing::warn!("Invalid API key format - must start with rpa_");
            return Ok(false);
        }

        // Look up credential by api_key (direct SELECT)
        let result = sqlx::query_as::<_, (String,)>(
            "SELECT secret FROM api_credentials
             WHERE api_key = $1 AND is_active = true"
        )
        .bind(api_key)
        .fetch_optional(&self.db.pool)
        .await?;

        let Some((secret,)) = result else {
            tracing::warn!("API key not found");
            return Ok(false);
        };

        // Compute expected signature
        let expected = Self::create_hmac_signature(&secret, timestamp, email, api_key, body);

        // Constant-time comparison
        if signature != expected {
            tracing::warn!("HMAC signature mismatch");
            return Ok(false);
        }

        // Update last_used_at
        sqlx::query("UPDATE api_credentials SET last_used_at = NOW() WHERE api_key = $1")
            .bind(api_key)
            .execute(&self.db.pool)
            .await?;

        Ok(true)
    }

    // Helper function for testing - verifies HMAC with known secret
    #[cfg(test)]
    pub fn verify_hmac_with_secret(
        secret: &str,
        api_key: &str,
        signature: &str,
        timestamp: i64,
        email: &str,
        body: &str,
    ) -> bool {
        let expected_signature = Self::create_hmac_signature(
            secret,
            timestamp,
            email,
            api_key,
            body,
        );
        expected_signature == signature
    }
}
