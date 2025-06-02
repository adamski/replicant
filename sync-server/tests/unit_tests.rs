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

    async fn setup_test_db() -> Result<ServerDatabase, Box<dyn std::error::Error>> {
        // Use DATABASE_URL environment variable if set, otherwise skip tests
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL environment variable not set. Set it to run database tests.")?;
        
        // Create a fresh connection for the test
        let db = ServerDatabase::new(&database_url).await?;
        
        // Run migrations first
        db.run_migrations().await?;
        
        // Then clean the database to ensure fresh state
        cleanup_database(&db).await?;
        
        Ok(db)
    }
    
    async fn cleanup_database(db: &ServerDatabase) -> Result<(), Box<dyn std::error::Error>> {
        // Delete all data in reverse order of foreign key dependencies
        sqlx::query("DELETE FROM change_events").execute(&db.pool).await?;
        sqlx::query("DELETE FROM document_revisions").execute(&db.pool).await?;
        sqlx::query("DELETE FROM active_connections").execute(&db.pool).await?;
        sqlx::query("DELETE FROM documents").execute(&db.pool).await?;
        sqlx::query("DELETE FROM users").execute(&db.pool).await?;
        
        // Reset sequences if needed (PostgreSQL specific)
        sqlx::query("ALTER SEQUENCE IF EXISTS change_events_sequence_seq RESTART WITH 1")
            .execute(&db.pool).await?;
        
        Ok(())
    }

    #[tokio::test]
    async fn test_document_delete() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_document_delete: {}", e);
                return;
            }
        };
        
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
        db.delete_document(&doc.id, &user_id, "1-delete").await.expect("Failed to delete document");
        
        // Try to retrieve document - it should exist but with deleted_at set
        let loaded_doc = db.get_document(&doc.id).await.expect("Failed to get document");
        
        // The deleted_at field should be set
        assert!(loaded_doc.deleted_at.is_some());
        
        println!("✅ test_document_delete passed");
    }

    #[tokio::test]
    async fn test_event_logging() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_event_logging: {}", e);
                return;
            }
        };
        
        // Create a test user
        let user_id = db.create_user("eventtest@example.com", "hashed_token")
            .await
            .expect("Failed to create user");
        
        // Get initial sequence number (should be 0)
        let initial_sequence = db.get_latest_sequence(&user_id).await.expect("Failed to get initial sequence");
        
        // Create a test document - this should log a CREATE event
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: "Event Test Document".to_string(),
            content: json!({"text": "Testing events", "version": 1}),
            revision_id: Document::initial_revision(&json!({"text": "Testing events", "version": 1})),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        db.create_document(&doc).await.expect("Failed to create document");
        
        // Check that a CREATE event was logged
        let events = db.get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");
        
        assert_eq!(events.len(), 1, "Should have exactly 1 event after document creation");
        assert_eq!(events[0].event_type, sync_core::protocol::ChangeEventType::Create);
        assert_eq!(events[0].document_id, doc.id);
        assert_eq!(events[0].user_id, user_id);
        assert_eq!(events[0].revision_id, doc.revision_id);
        assert!(events[0].forward_patch.is_some(), "Create events should have the document as forward_patch");
        assert!(events[0].reverse_patch.is_none(), "Create events should not have reverse_patch");
        
        println!("✅ CREATE event properly logged with sequence {}", events[0].sequence);
        
        // Update the document - this should log an UPDATE event
        let mut updated_doc = doc.clone();
        updated_doc.content = json!({"text": "Updated content", "version": 2});
        updated_doc.revision_id = updated_doc.next_revision(&updated_doc.content);
        updated_doc.version = 2;
        updated_doc.updated_at = chrono::Utc::now();
        
        // Create a simple patch for testing
        let patch = json_patch::Patch(vec![
            json_patch::PatchOperation::Replace(json_patch::ReplaceOperation {
                path: "/text".to_string(),
                value: json!("Updated content"),
            }),
            json_patch::PatchOperation::Replace(json_patch::ReplaceOperation {
                path: "/version".to_string(), 
                value: json!(2),
            })
        ]);
        
        db.update_document(&updated_doc, Some(&patch)).await.expect("Failed to update document");
        
        // Check that an UPDATE event was logged
        let events = db.get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");
            
        assert_eq!(events.len(), 2, "Should have exactly 2 events after document update");
        assert_eq!(events[1].event_type, sync_core::protocol::ChangeEventType::Update);
        assert_eq!(events[1].document_id, doc.id);
        assert_eq!(events[1].revision_id, updated_doc.revision_id);
        assert!(events[1].forward_patch.is_some(), "Update events should have forward patch data");
        assert!(events[1].reverse_patch.is_some(), "Update events should have reverse patch data");
        
        println!("✅ UPDATE event properly logged with sequence {} and patch data", events[1].sequence);
        
        // Delete the document - this should log a DELETE event  
        db.delete_document(&doc.id, &user_id, "2-delete").await.expect("Failed to delete document");
        
        // Check that a DELETE event was logged
        let events = db.get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");
            
        assert_eq!(events.len(), 3, "Should have exactly 3 events after document deletion");
        assert_eq!(events[2].event_type, sync_core::protocol::ChangeEventType::Delete);
        assert_eq!(events[2].document_id, doc.id);
        assert_eq!(events[2].revision_id, "2-delete");
        assert!(events[2].forward_patch.is_none(), "Delete events should not have forward patch");
        assert!(events[2].reverse_patch.is_some(), "Delete events should have reverse patch (full document)");
        
        println!("✅ DELETE event properly logged with sequence {}", events[2].sequence);
        
        // Verify sequence numbers are incrementing
        assert!(events[0].sequence < events[1].sequence, "Sequence numbers should increment");
        assert!(events[1].sequence < events[2].sequence, "Sequence numbers should increment");
        
        // Test get_latest_sequence
        let latest_sequence = db.get_latest_sequence(&user_id).await.expect("Failed to get latest sequence");
        assert_eq!(latest_sequence, events[2].sequence, "Latest sequence should match last event");
        
        println!("✅ Event logging test passed - all events properly recorded with correct sequences");
    }
}