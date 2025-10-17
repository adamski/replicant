use std::sync::Arc;
use rand::Rng;
use crate::database::ServerDatabase;
use sync_core::SyncResult;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub struct ApiCredentials {
    pub api_key: String,
    pub secret: String,
}

#[derive(Clone)]
pub struct AuthState {
    db: Arc<ServerDatabase>,
}

impl AuthState {
    pub fn new(db: Arc<ServerDatabase>) -> Self {
        Self {
            db,
        }
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
        sqlx::query!(
            "INSERT INTO api_credentials (api_key, secret, name)
             VALUES ($1, $2, $3)",
            credentials.api_key,
            credentials.secret,
            name
        )
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

        // Look up credential by api_key
        let secret: Option<String> = sqlx::query_scalar!(
            "SELECT secret FROM api_credentials
             WHERE api_key = $1 AND is_active = true",
            api_key
        )
        .fetch_optional(&self.db.pool)
        .await?;

        let Some(secret) = secret else {
            tracing::warn!("API key not found");
            return Ok(false);
        };

        // Compute expected signature
        let expected = Self::create_hmac_signature(&secret, timestamp, email, api_key, body);

        // Constant-time comparison to prevent timing attacks
        if !bool::from(signature.as_bytes().ct_eq(expected.as_bytes())) {
            tracing::warn!("HMAC signature mismatch");
            return Ok(false);
        }

        // Update last_used_at
        sqlx::query!(
            "UPDATE api_credentials SET last_used_at = NOW() WHERE api_key = $1",
            api_key
        )
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
