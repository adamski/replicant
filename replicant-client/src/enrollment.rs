//! HTTP client for the plugin-initiated enrollment flow.
use crate::secret_store::Credentials;

#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("enrollment request failed: {0}")]
    Http(String),
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
        200 => resp
            .json::<Credentials>()
            .await
            .map_err(|e| EnrollError::Http(e.to_string())),
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
        Mock::given(method("POST"))
            .and(path("/api/enroll/claim"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"api_key":"rpa_x","secret":"rps_y"})),
            )
            .mount(&server)
            .await;

        let creds = claim(&server.uri(), "a@b.com", "TOK").await.unwrap();
        assert_eq!(creds.api_key, "rpa_x");
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
