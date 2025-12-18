use replicant_server::auth::AuthState;

#[test]
fn test_api_credentials_generation() {
    let creds = AuthState::generate_api_credentials();

    // API key should start with rpa_ prefix and be 68 characters total
    assert!(creds.api_key.starts_with("rpa_"));
    assert_eq!(creds.api_key.len(), 68);

    // Secret should start with rps_ prefix and be 68 characters total
    assert!(creds.secret.starts_with("rps_"));
    assert_eq!(creds.secret.len(), 68);

    // Generate another set - should be unique
    let creds2 = AuthState::generate_api_credentials();
    assert_ne!(creds.api_key, creds2.api_key);
    assert_ne!(creds.secret, creds2.secret);
}

#[cfg(test)]
mod database_tests {
    use replicant_core::models::Document;
    use replicant_server::database::ServerDatabase;
    use serde_json::json;
    use uuid::Uuid;

    async fn setup_test_db() -> Result<ServerDatabase, Box<dyn std::error::Error>> {
        // Use DATABASE_URL environment variable if set, otherwise skip tests
        let database_url = std::env::var("DATABASE_URL").map_err(|_| {
            "DATABASE_URL environment variable not set. Set it to run database tests."
        })?;

        // Use test app namespace ID
        let app_namespace_id = "com.example.sync-task-list".to_string();

        // Create a fresh connection for the test
        let db = ServerDatabase::new(&database_url, app_namespace_id).await?;

        // Run migrations first
        db.run_migrations().await?;

        // Then clean the database to ensure fresh state
        cleanup_database(&db).await?;

        Ok(db)
    }

    async fn cleanup_database(db: &ServerDatabase) -> Result<(), Box<dyn std::error::Error>> {
        // Delete all data in reverse order of foreign key dependencies
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

        // Reset sequences if needed (PostgreSQL specific)
        sqlx::query("ALTER SEQUENCE IF EXISTS change_events_sequence_seq RESTART WITH 1")
            .execute(&db.pool)
            .await?;

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
        let user_id = db
            .create_user("test@example.com")
            .await
            .expect("Failed to create user");

        // Create a test document
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: json!({
                "title": "Test Document",
                "text": "Hello, World!"
            }),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        // Save document
        db.create_document(&doc)
            .await
            .expect("Failed to create document");

        // Delete document
        db.delete_document(&doc.id, &user_id)
            .await
            .expect("Failed to delete document");

        // Try to retrieve document - it should exist but with deleted_at set
        let loaded_doc = db
            .get_document(&doc.id)
            .await
            .expect("Failed to get document");

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
        let user_id = db
            .create_user("eventtest@example.com")
            .await
            .expect("Failed to create user");

        // Get initial sequence number (should be 0)
        let initial_sequence = db
            .get_latest_sequence(&user_id)
            .await
            .expect("Failed to get initial sequence");

        // Create a test document - this should log a CREATE event
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: json!({"title": "Event Test Document", "text": "Testing events", "version": 1}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc)
            .await
            .expect("Failed to create document");

        // Check that a CREATE event was logged
        let events = db
            .get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");

        assert_eq!(
            events.len(),
            1,
            "Should have exactly 1 event after document creation"
        );
        assert_eq!(
            events[0].event_type,
            replicant_core::protocol::ChangeEventType::Create
        );
        assert_eq!(events[0].document_id, doc.id);
        assert_eq!(events[0].user_id, user_id);
        assert!(
            events[0].forward_patch.is_some(),
            "Create events should have the document as forward_patch"
        );
        assert!(
            events[0].reverse_patch.is_none(),
            "Create events should not have reverse_patch"
        );

        println!(
            "✅ CREATE event properly logged with sequence {}",
            events[0].sequence
        );

        // Update the document - this should log an UPDATE event
        let mut updated_doc = doc.clone();
        updated_doc.content = json!({"text": "Updated content", "version": 2});
        updated_doc.sync_revision = 2;
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
            }),
        ]);

        db.update_document(&updated_doc, Some(&patch))
            .await
            .expect("Failed to update document");

        // Check that an UPDATE event was logged
        let events = db
            .get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");

        assert_eq!(
            events.len(),
            2,
            "Should have exactly 2 events after document update"
        );
        assert_eq!(
            events[1].event_type,
            replicant_core::protocol::ChangeEventType::Update
        );
        assert_eq!(events[1].document_id, doc.id);
        assert!(
            events[1].forward_patch.is_some(),
            "Update events should have forward patch data"
        );
        assert!(
            events[1].reverse_patch.is_some(),
            "Update events should have reverse patch data"
        );

        println!(
            "✅ UPDATE event properly logged with sequence {} and patch data",
            events[1].sequence
        );

        // Delete the document - this should log a DELETE event
        db.delete_document(&doc.id, &user_id)
            .await
            .expect("Failed to delete document");

        // Check that a DELETE event was logged
        let events = db
            .get_changes_since(&user_id, initial_sequence, Some(10))
            .await
            .expect("Failed to get events");

        assert_eq!(
            events.len(),
            3,
            "Should have exactly 3 events after document deletion"
        );
        assert_eq!(
            events[2].event_type,
            replicant_core::protocol::ChangeEventType::Delete
        );
        assert_eq!(events[2].document_id, doc.id);
        assert!(
            events[2].forward_patch.is_none(),
            "Delete events should not have forward patch"
        );
        assert!(
            events[2].reverse_patch.is_some(),
            "Delete events should have reverse patch (full document)"
        );

        println!(
            "✅ DELETE event properly logged with sequence {}",
            events[2].sequence
        );

        // Verify sequence numbers are incrementing
        assert!(
            events[0].sequence < events[1].sequence,
            "Sequence numbers should increment"
        );
        assert!(
            events[1].sequence < events[2].sequence,
            "Sequence numbers should increment"
        );

        // Test get_latest_sequence
        let latest_sequence = db
            .get_latest_sequence(&user_id)
            .await
            .expect("Failed to get latest sequence");
        assert_eq!(
            latest_sequence, events[2].sequence,
            "Latest sequence should match last event"
        );

        println!(
            "✅ Event logging test passed - all events properly recorded with correct sequences"
        );
    }

    #[tokio::test]
    async fn test_conflict_storage_on_create() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_conflict_storage_on_create: {}", e);
                return;
            }
        };

        // Create test user
        let email = format!(
            "conflict_test_{}@example.com",
            &Uuid::new_v4().to_string()[..8]
        );
        let user_id = db.create_user(&email).await.expect("Failed to create user");

        // Create document v1 on "server"
        let doc_id = Uuid::new_v4();
        let server_doc = Document {
            id: doc_id,
            user_id,
            content: json!({"value": "server-content", "source": "server"}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&server_doc)
            .await
            .expect("Failed to create server document");
        println!("✅ Created server version of document");

        // Simulate client creating same document (conflict scenario)
        let client_doc = Document {
            id: doc_id,
            user_id,
            content: json!({"value": "client-content", "source": "client"}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        // Start transaction to log conflict (simulating sync_handler behavior)
        let mut tx = db.pool.begin().await.expect("Failed to begin transaction");

        // Log server version as conflict loser
        let server_content_json = serde_json::to_value(&server_doc.content).unwrap();
        db.log_change_event(
            &mut tx,
            replicant_server::database::ChangeEventParams {
                document_id: &doc_id,
                user_id: &user_id,
                event_type: replicant_core::protocol::ChangeEventType::Create,
                forward_patch: Some(&server_content_json),
                reverse_patch: None,
                applied: false,
            },
        )
        .await
        .expect("Failed to log conflict");

        tx.commit().await.expect("Failed to commit conflict log");
        println!("✅ Logged server version as conflict loser");

        // Update document to client version (winner)
        db.update_document(&client_doc, None)
            .await
            .expect("Failed to update to client version");
        println!("✅ Updated to client version");

        // Verify: Should have both events
        let all_events = db
            .get_changes_since(&user_id, 0, None)
            .await
            .expect("Failed to get changes");
        println!("📊 Total events: {}", all_events.len());
        assert!(
            all_events.len() >= 3,
            "Should have at least 3 events: initial create, conflict, update"
        );

        // Check unapplied changes (conflicts)
        let conflicts = db
            .get_unapplied_changes(&doc_id)
            .await
            .expect("Failed to get unapplied changes");
        println!("📊 Unapplied changes (conflicts): {}", conflicts.len());

        assert_eq!(
            conflicts.len(),
            1,
            "Should have exactly 1 unapplied change (conflict loser)"
        );
        assert!(
            conflicts[0].forward_patch.is_some(),
            "Conflict should preserve server content"
        );

        // Verify the preserved content
        let preserved = &conflicts[0].forward_patch.as_ref().unwrap();
        assert_eq!(
            preserved["source"], "server",
            "Should preserve server's content"
        );

        println!("✅ Conflict storage test passed - server version preserved as unapplied");
    }

    #[tokio::test]
    async fn test_conflict_storage_on_update() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_conflict_storage_on_update: {}", e);
                return;
            }
        };

        // Create test user
        let email = format!(
            "conflict_update_{}@example.com",
            &Uuid::new_v4().to_string()[..8]
        );
        let user_id = db.create_user(&email).await.expect("Failed to create user");

        // Create initial document
        let doc_id = Uuid::new_v4();
        let doc = Document {
            id: doc_id,
            user_id,
            content: json!({"value": 1, "name": "initial"}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc)
            .await
            .expect("Failed to create document");
        println!("✅ Created initial document");

        // Simulate concurrent update scenario
        // Server's state before conflict
        let server_state = json!({"value": 2, "name": "server-update"});

        // Start transaction to log server's state as conflict loser
        let mut tx = db.pool.begin().await.expect("Failed to begin transaction");

        let server_content_json = serde_json::to_value(&server_state).unwrap();
        db.log_change_event(
            &mut tx,
            replicant_server::database::ChangeEventParams {
                document_id: &doc_id,
                user_id: &user_id,
                event_type: replicant_core::protocol::ChangeEventType::Update,
                forward_patch: Some(&server_content_json),
                reverse_patch: None,
                applied: false,
            },
        )
        .await
        .expect("Failed to log conflict");

        tx.commit().await.expect("Failed to commit conflict log");
        println!("✅ Logged server state as conflict loser");

        // Apply client's winning update
        let mut winning_doc = doc.clone();
        winning_doc.content = json!({"value": 3, "name": "client-wins"});
        winning_doc.sync_revision = 2;

        db.update_document(&winning_doc, None)
            .await
            .expect("Failed to apply winning update");
        println!("✅ Applied client's winning update");

        // Verify unapplied changes
        let conflicts = db
            .get_unapplied_changes(&doc_id)
            .await
            .expect("Failed to get conflicts");
        println!("📊 Unapplied changes: {}", conflicts.len());

        assert_eq!(conflicts.len(), 1, "Should have 1 unapplied change");
        assert!(
            conflicts[0].forward_patch.is_some(),
            "Should preserve server's state"
        );

        // Verify preserved content
        let preserved = conflicts[0].forward_patch.as_ref().unwrap();
        assert_eq!(
            preserved["name"], "server-update",
            "Should preserve server's update"
        );
        assert_eq!(preserved["value"], 2, "Should preserve server's value");

        println!("✅ Update conflict storage test passed");
    }

    #[tokio::test]
    async fn test_query_unapplied_changes() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_query_unapplied_changes: {}", e);
                return;
            }
        };

        // Create test user
        let email = format!(
            "query_test_{}@example.com",
            &Uuid::new_v4().to_string()[..8]
        );
        let user_id = db.create_user(&email).await.expect("Failed to create user");

        // Create document with multiple conflicts
        let doc_id = Uuid::new_v4();
        let doc = Document {
            id: doc_id,
            user_id,
            content: json!({"version": 0}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc)
            .await
            .expect("Failed to create document");

        // Create multiple conflict scenarios
        for i in 1..=3 {
            let mut tx = db.pool.begin().await.expect("Failed to begin transaction");

            let conflict_content = json!({"version": i, "conflict": true});
            let conflict_json = serde_json::to_value(&conflict_content).unwrap();

            db.log_change_event(
                &mut tx,
                replicant_server::database::ChangeEventParams {
                    document_id: &doc_id,
                    user_id: &user_id,
                    event_type: replicant_core::protocol::ChangeEventType::Update,
                    forward_patch: Some(&conflict_json),
                    reverse_patch: None,
                    applied: false,
                },
            )
            .await
            .expect("Failed to log conflict");

            tx.commit().await.expect("Failed to commit");
        }

        println!("✅ Created 3 conflict scenarios");

        // Query unapplied changes
        let conflicts = db
            .get_unapplied_changes(&doc_id)
            .await
            .expect("Failed to query conflicts");

        assert_eq!(conflicts.len(), 3, "Should have 3 unapplied changes");

        // Verify ordering (DESC by sequence)
        assert!(
            conflicts[0].sequence > conflicts[1].sequence,
            "Should be ordered DESC"
        );
        assert!(
            conflicts[1].sequence > conflicts[2].sequence,
            "Should be ordered DESC"
        );

        // Verify all are unapplied conflicts
        for (idx, conflict) in conflicts.iter().enumerate() {
            println!("  Conflict {}: seq={}", idx, conflict.sequence);
            assert!(
                conflict.forward_patch.is_some(),
                "All conflicts should have content"
            );
        }

        println!("✅ Query unapplied changes test passed");
    }

    #[tokio::test]
    async fn test_title_extraction() {
        let db = match setup_test_db().await {
            Ok(db) => db,
            Err(e) => {
                println!("⏭️ Skipping test_title_extraction: {}", e);
                return;
            }
        };

        // Create a test user with unique email
        let email = format!(
            "test-title-extraction-{}@example.com",
            &Uuid::new_v4().to_string()[..8]
        );
        let user_id = db.create_user(&email).await.expect("Failed to create user");

        // Test 1: Document with title in content
        let doc_with_title = Document {
            id: Uuid::new_v4(),
            user_id,
            content: json!({
                "title": "My Document",
                "text": "Hello, World!"
            }),
            sync_revision: 1,
            content_hash: None,
            title: None, // Not set, should be extracted
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc_with_title)
            .await
            .expect("Failed to create document with title");

        let retrieved = db
            .get_document(&doc_with_title.id)
            .await
            .expect("Failed to retrieve document");

        assert_eq!(retrieved.title, Some("My Document".to_string()));

        // Test 2: Document without title (should use datetime fallback)
        let doc_without_title = Document {
            id: Uuid::new_v4(),
            user_id,
            content: json!({
                "text": "Hello, World!"
            }),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc_without_title)
            .await
            .expect("Failed to create document without title");

        let retrieved = db
            .get_document(&doc_without_title.id)
            .await
            .expect("Failed to retrieve document");

        // Should have datetime format: YYYY-MM-DD|HH:MM:SS.mmm
        assert!(retrieved.title.is_some());
        let title = retrieved.title.unwrap();
        assert!(title.contains('|'), "Title should contain pipe separator");
        assert!(title.contains('-'), "Title should contain date separator");

        // Test 3: Very long title (should be truncated to 128 chars)
        let long_title = "a".repeat(200);
        let doc_long_title = Document {
            id: Uuid::new_v4(),
            user_id,
            content: json!({
                "title": long_title,
                "text": "Hello, World!"
            }),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.create_document(&doc_long_title)
            .await
            .expect("Failed to create document with long title");

        let retrieved = db
            .get_document(&doc_long_title.id)
            .await
            .expect("Failed to retrieve document");

        assert_eq!(retrieved.title.as_ref().unwrap().len(), 128);
        assert_eq!(retrieved.title, Some("a".repeat(128)));

        // Test 4: Update document with new title
        let updated_doc = Document {
            id: doc_with_title.id,
            user_id,
            content: json!({
                "title": "Updated Title",
                "text": "Updated content"
            }),
            sync_revision: 2,
            content_hash: None,
            title: None,
            created_at: doc_with_title.created_at,
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        db.update_document(&updated_doc, None)
            .await
            .expect("Failed to update document");

        let retrieved = db
            .get_document(&doc_with_title.id)
            .await
            .expect("Failed to retrieve updated document");

        assert_eq!(retrieved.title, Some("Updated Title".to_string()));

        println!("✅ Title extraction test passed");
    }
}
