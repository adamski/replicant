//! HTTP client for the plugin-initiated enrollment flow.
use crate::secret_store::Credentials;

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("enrollment request failed: {0}")]
    Http(String),
    #[error("enrollment response invalid or incomplete")]
    InvalidResponse,
}

const MAX_CRED_LEN: usize = 128;

fn validate(creds: &Credentials) -> Result<(), EnrollError> {
    if creds.user_id.is_nil()
        || !creds.api_key.starts_with("rpa_")
        || creds.api_key.len() > MAX_CRED_LEN
        || !creds.secret.starts_with("rps_")
        || creds.secret.len() > MAX_CRED_LEN
    {
        return Err(EnrollError::InvalidResponse);
    }
    Ok(())
}

pub async fn request(base_url: &str, email: &str) -> Result<(), EnrollError> {
    let resp = reqwest::Client::new()
        .post(format!("{base_url}/api/enroll/request"))
        .json(&serde_json::json!({ "email": email }))
        .send()
        .await
        .map_err(|e| EnrollError::Http(e.to_string()))?;

    if resp.status().as_u16() == 202 {
        Ok(())
    } else {
        Err(EnrollError::Http(format!("status {}", resp.status())))
    }
}

pub async fn claim(base_url: &str, email: &str, token: &str) -> Result<Credentials, EnrollError> {
    let resp = reqwest::Client::new()
        .post(format!("{base_url}/api/enroll/claim"))
        .json(&serde_json::json!({ "email": email, "token": token }))
        .send()
        .await
        .map_err(|e| EnrollError::Http(e.to_string()))?;

    match resp.status().as_u16() {
        200 => {
            // Any undecodable 200 body (missing/unparseable fields, non-JSON)
            // is an invalid enrollment response, not a transport error.
            let creds = resp
                .json::<Credentials>()
                .await
                .map_err(|_| EnrollError::InvalidResponse)?;
            validate(&creds)?;
            Ok(creds)
        }
        401 => Err(EnrollError::InvalidToken),
        s => Err(EnrollError::Http(format!("status {s}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn request_treats_202_as_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/request"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        assert!(request(&server.uri(), "a@b.com").await.is_ok());
    }

    #[tokio::test]
    async fn claim_returns_credentials_on_200() {
        let server = MockServer::start().await;
        let user_id = uuid::Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "rpa_x", "secret": "rps_y", "user_id": user_id
            })))
            .mount(&server)
            .await;

        let creds = claim(&server.uri(), "a@b.com", "TOK").await.unwrap();
        assert_eq!(creds.api_key, "rpa_x");
        assert_eq!(creds.user_id, user_id);
    }

    #[tokio::test]
    async fn claim_rejects_missing_or_nil_user_id() {
        let missing = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"api_key":"rpa_x","secret":"rps_y"})),
            )
            .mount(&missing)
            .await;
        assert!(matches!(
            claim(&missing.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));

        let nil = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "rpa_x",
                "secret": "rps_y",
                "user_id": "00000000-0000-0000-0000-000000000000"
            })))
            .mount(&nil)
            .await;
        assert!(matches!(
            claim(&nil.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));
    }

    #[tokio::test]
    async fn claim_rejects_bad_prefix_or_oversized_fields() {
        let user_id = uuid::Uuid::new_v4();

        let bad_prefix = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "xxx_bad", "secret": "rps_y", "user_id": user_id
            })))
            .mount(&bad_prefix)
            .await;
        assert!(matches!(
            claim(&bad_prefix.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));

        let oversized = MockServer::start().await;
        let oversized_key = format!("rpa_{}", "a".repeat(200));
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": oversized_key, "secret": "rps_y", "user_id": user_id
            })))
            .mount(&oversized)
            .await;
        assert!(matches!(
            claim(&oversized.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));

        let bad_secret_prefix = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "rpa_x", "secret": "xxx_bad", "user_id": user_id
            })))
            .mount(&bad_secret_prefix)
            .await;
        assert!(matches!(
            claim(&bad_secret_prefix.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));

        let oversized_secret = MockServer::start().await;
        let oversized_secret_value = format!("rps_{}", "a".repeat(200));
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "rpa_x", "secret": oversized_secret_value, "user_id": user_id
            })))
            .mount(&oversized_secret)
            .await;
        assert!(matches!(
            claim(&oversized_secret.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));
    }

    #[tokio::test]
    async fn claim_rejects_unparseable_user_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "api_key": "rpa_x", "secret": "rps_y", "user_id": "not-a-uuid"
            })))
            .mount(&server)
            .await;
        assert!(matches!(
            claim(&server.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidResponse)
        ));
    }

    #[tokio::test]
    async fn claim_maps_401_to_invalid_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        assert!(matches!(
            claim(&server.uri(), "a@b.com", "TOK").await,
            Err(EnrollError::InvalidToken)
        ));
    }
}
