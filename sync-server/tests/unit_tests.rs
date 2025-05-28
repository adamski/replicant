use sync_server::auth::AuthState;

#[test]
fn test_token_generation_format() {
    let token = AuthState::generate_auth_token();
    
    // Should be a valid UUID
    assert_eq!(token.len(), 36);
    assert!(uuid::Uuid::parse_str(&token).is_ok());
}

#[test]
fn test_token_hashing() {
    let token = "test-token";
    let hash1 = AuthState::hash_token(token).unwrap();
    let hash2 = AuthState::hash_token(token).unwrap();
    
    // Same token should produce different hashes (due to salt)
    assert_ne!(hash1, hash2);
    
    // Both hashes should verify correctly
    assert!(AuthState::verify_token_hash(token, &hash1).unwrap());
    assert!(AuthState::verify_token_hash(token, &hash2).unwrap());
}

#[test]
fn test_invalid_hash_verification() {
    let token = "test-token";
    
    // Invalid hash format should return error
    assert!(AuthState::verify_token_hash(token, "invalid-hash").is_err());
    
    // Wrong token should fail verification
    let hash = AuthState::hash_token(token).unwrap();
    assert!(!AuthState::verify_token_hash("wrong-token", &hash).unwrap());
}

#[cfg(test)]
mod database_tests {
    use sync_server::database::ServerDatabase;
    use uuid::Uuid;
    use sync_core::models::{Document, VectorClock};
    use serde_json::json;

    async fn setup_test_db() -> ServerDatabase {
        // Use in-memory database for tests
        let db = ServerDatabase::new("postgres://test_user:test_pass@localhost:5432/test_db")
            .await
            .expect("Failed to create test database");
        
        // Run migrations
        db.run_migrations().await.expect("Failed to run migrations");
        
        db
    }

    #[tokio::test]
    async fn test_document_delete() {
        let db = setup_test_db().await;
        
        // Create a test user
        let user_id = db.create_user("test@example.com", "hashed_token")
            .await
            .expect("Failed to create user");
        
        // Create a test document
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: "Test Document".to_string(),
            content: json!({
                "text": "Hello, World!"
            }),
            revision_id: Document::initial_revision(&json!({
                "text": "Hello, World!"
            })),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        // Save document
        db.create_document(&doc).await.expect("Failed to create document");
        
        // Delete document
        db.delete_document(&doc.id, &user_id).await.expect("Failed to delete document");
        
        // Try to retrieve document - it should exist but with deleted_at set
        let loaded_doc = db.get_document(&doc.id).await.expect("Failed to get document");
        
        // The deleted_at field should be set
        assert!(loaded_doc.deleted_at.is_some());
    }
}