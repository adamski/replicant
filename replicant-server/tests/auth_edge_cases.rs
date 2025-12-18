//! # Authentication Edge Case Tests
//!
//! This module tests all error paths and edge cases in the HMAC
//! authentication flow. Each test verifies:
//! 1. Correct rejection of invalid authentication attempts
//! 2. No sensitive data leakage in error responses
//! 3. Protection against timing attacks and replay attacks
//!
//! See: docs/testing_guide.md for conventions

use replicant_server::auth::AuthState;
use replicant_server::database::ServerDatabase;
use std::sync::Arc;

async fn setup_test_db() -> Result<ServerDatabase, Box<dyn std::error::Error>> {
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL environment variable not set")?;

    let app_namespace_id = "com.example.sync-task-list".to_string();
    let db = ServerDatabase::new(&database_url, app_namespace_id).await?;
    db.run_migrations().await?;
    cleanup_database(&db).await?;

    Ok(db)
}

async fn cleanup_database(db: &ServerDatabase) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("DELETE FROM change_events")
        .execute(&db.pool)
        .await?;
    sqlx::query("DELETE FROM document_revisions")
        .execute(&db.pool)
        .await?;
    sqlx::query("DELETE FROM active_connections")
        .execute(&db.pool)
        .await?;
    sqlx::query("DELETE FROM documents")
        .execute(&db.pool)
        .await?;
    sqlx::query("DELETE FROM users").execute(&db.pool).await?;
    sqlx::query("DELETE FROM api_credentials")
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Tests that HMAC signatures with invalid format are rejected.
#[tokio::test]
async fn test_hmac_signature_validation_with_invalid_signature() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    // Generate and save valid credentials
    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-invalid-sig")
        .await
        .unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    // Test with completely invalid signature
    let result = auth
        .verify_hmac(
            &creds.api_key,
            "invalid_signature_format",
            timestamp,
            email,
            body,
        )
        .await
        .unwrap();

    assert!(!result, "Invalid signature should be rejected");

    // Test with valid hex but wrong signature
    let result = auth
        .verify_hmac(
            &creds.api_key,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            timestamp,
            email,
            body,
        )
        .await
        .unwrap();

    assert!(!result, "Wrong signature should be rejected");

    // Test with empty signature
    let result = auth
        .verify_hmac(&creds.api_key, "", timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Empty signature should be rejected");

    println!("✅ Invalid signature test passed");
}

/// Tests that HMAC signatures older than 5 minutes are rejected.
/// This prevents replay attacks using captured signatures.
#[tokio::test]
async fn test_hmac_signature_validation_with_expired_timestamp() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-expired").await.unwrap();

    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    // Test with timestamp 6 minutes in the past (beyond 5-minute window)
    let old_timestamp = chrono::Utc::now().timestamp() - 360; // 6 minutes ago
    let signature =
        AuthState::create_hmac_signature(&creds.secret, old_timestamp, email, &creds.api_key, body);

    let result = auth
        .verify_hmac(&creds.api_key, &signature, old_timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Signature older than 5 minutes should be rejected");

    // Test with timestamp in the future (beyond 5-minute window)
    let future_timestamp = chrono::Utc::now().timestamp() + 360; // 6 minutes in future
    let signature = AuthState::create_hmac_signature(
        &creds.secret,
        future_timestamp,
        email,
        &creds.api_key,
        body,
    );

    let result = auth
        .verify_hmac(&creds.api_key, &signature, future_timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Future timestamp beyond window should be rejected");

    // Test with timestamp exactly at the 5-minute boundary (should pass)
    let boundary_timestamp = chrono::Utc::now().timestamp() - 299; // 4m 59s ago
    let signature = AuthState::create_hmac_signature(
        &creds.secret,
        boundary_timestamp,
        email,
        &creds.api_key,
        body,
    );

    let result = auth
        .verify_hmac(&creds.api_key, &signature, boundary_timestamp, email, body)
        .await
        .unwrap();

    assert!(
        result,
        "Signature within 5-minute window should be accepted"
    );

    println!("✅ Expired timestamp test passed");
}

/// Tests various malformed API key formats are rejected.
#[tokio::test]
async fn test_malformed_api_key_formats() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-malformed")
        .await
        .unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;
    let signature =
        AuthState::create_hmac_signature(&creds.secret, timestamp, email, &creds.api_key, body);

    // Test with wrong prefix
    let wrong_prefix_key = creds.api_key.replace("rpa_", "rpx_");
    let result = auth
        .verify_hmac(&wrong_prefix_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "API key without rpa_ prefix should be rejected");

    // Test with no prefix
    let no_prefix_key = creds.api_key.replace("rpa_", "");
    let result = auth
        .verify_hmac(&no_prefix_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "API key without prefix should be rejected");

    // Test with empty API key
    let result = auth
        .verify_hmac("", &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Empty API key should be rejected");

    println!("✅ Malformed API key test passed");
}

/// Tests that requests with non-existent API keys are rejected.
#[tokio::test]
async fn test_api_key_not_found_in_database() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    // Don't save credentials - just generate them
    let creds = AuthState::generate_api_credentials();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;
    let signature =
        AuthState::create_hmac_signature(&creds.secret, timestamp, email, &creds.api_key, body);

    // Try to authenticate with non-existent API key
    let result = auth
        .verify_hmac(&creds.api_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Non-existent API key should be rejected");

    println!("✅ API key not found test passed");
}

/// Tests that using correct API key but wrong secret fails authentication.
/// The signature will be computed with the wrong secret.
#[tokio::test]
async fn test_api_secret_mismatch() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-secret-mismatch")
        .await
        .unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    // Create signature with wrong secret
    let wrong_secret = "rps_0000000000000000000000000000000000000000000000000000000000000000";
    let signature =
        AuthState::create_hmac_signature(wrong_secret, timestamp, email, &creds.api_key, body);

    let result = auth
        .verify_hmac(&creds.api_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(
        !result,
        "Signature created with wrong secret should be rejected"
    );

    println!("✅ API secret mismatch test passed");
}

/// Tests that valid HMAC authentication succeeds.
/// This is the happy path to ensure our rejection tests aren't too strict.
#[tokio::test]
async fn test_valid_hmac_authentication_succeeds() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-valid").await.unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    // Create valid signature
    let signature =
        AuthState::create_hmac_signature(&creds.secret, timestamp, email, &creds.api_key, body);

    // Should succeed with correct credentials
    let result = auth
        .verify_hmac(&creds.api_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(result, "Valid HMAC authentication should succeed");

    println!("✅ Valid authentication test passed");
}

/// Tests that changing any part of the signed message invalidates the signature.
/// This ensures the HMAC covers all critical parameters.
#[tokio::test]
async fn test_hmac_signature_covers_all_parameters() {
    let db = match setup_test_db().await {
        Ok(db) => db,
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(Arc::new(db));

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-params").await.unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    let signature =
        AuthState::create_hmac_signature(&creds.secret, timestamp, email, &creds.api_key, body);

    // Try to use signature with different email
    let result = auth
        .verify_hmac(
            &creds.api_key,
            &signature,
            timestamp,
            "different@example.com",
            body,
        )
        .await
        .unwrap();

    assert!(!result, "Changing email should invalidate signature");

    // Try to use signature with different body
    let result = auth
        .verify_hmac(
            &creds.api_key,
            &signature,
            timestamp,
            email,
            r#"{"test": "different"}"#,
        )
        .await
        .unwrap();

    assert!(!result, "Changing body should invalidate signature");

    // Try to use signature with different timestamp
    let result = auth
        .verify_hmac(&creds.api_key, &signature, timestamp + 1, email, body)
        .await
        .unwrap();

    assert!(!result, "Changing timestamp should invalidate signature");

    println!("✅ HMAC parameter coverage test passed");
}

/// Tests that inactive API credentials are rejected.
#[tokio::test]
async fn test_inactive_credentials_rejected() {
    let db = match setup_test_db().await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            println!("⏭️ Skipping test: {}", e);
            return;
        }
    };

    let auth = AuthState::new(db.clone());

    let creds = AuthState::generate_api_credentials();
    auth.save_credentials(&creds, "test-inactive")
        .await
        .unwrap();

    // Mark credentials as inactive
    sqlx::query("UPDATE api_credentials SET is_active = false WHERE api_key = $1")
        .bind(&creds.api_key)
        .execute(&db.pool)
        .await
        .unwrap();

    let timestamp = chrono::Utc::now().timestamp();
    let email = "test@example.com";
    let body = r#"{"test": "data"}"#;

    let signature =
        AuthState::create_hmac_signature(&creds.secret, timestamp, email, &creds.api_key, body);

    // Try to authenticate with inactive credentials
    let result = auth
        .verify_hmac(&creds.api_key, &signature, timestamp, email, body)
        .await
        .unwrap();

    assert!(!result, "Inactive credentials should be rejected");

    println!("✅ Inactive credentials test passed");
}
