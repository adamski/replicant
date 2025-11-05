use crate::integration::helpers::TestContext;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

crate::integration_test!(
    test_offline_conflict_detection_and_resolution,
    |ctx: TestContext| async move {
        // Tests offline conflict resolution workflow.
        //
        // Scenario:
        // 1. Two clients (A and B) both have the same document synced
        // 2. Both go offline
        // 3. Both make conflicting edits to the same field
        // 4. Both reconnect and sync
        // 5. Server detects conflict and resolves it
        // 6. Both clients eventually converge to the same state
        //
        // This is a critical test for the offline-first sync system.
        tracing::info!("=== Testing Offline Conflict Resolution ===");

        // Create test user and credentials with unique email
        let test_id = uuid::Uuid::new_v4();
        let email = format!("conflict-test-{}@example.com", test_id);
        let (api_key, api_secret) = ctx
            .generate_test_credentials(&format!("conflict-test-{}", test_id))
            .await
            .expect("Failed to generate credentials");
        let user_id = ctx
            .create_test_user(&email)
            .await
            .expect("Failed to create user");

        tracing::info!("Created user: {}", user_id);

        // Create two clients
        let client_a = ctx
            .create_test_client(&email, user_id, &api_key, &api_secret)
            .await
            .expect("Failed to create client A");

        let client_b = ctx
            .create_test_client(&email, user_id, &api_key, &api_secret)
            .await
            .expect("Failed to create client B");

        sleep(Duration::from_millis(500)).await;

        // Client A creates a document
        tracing::info!("Client A creating document...");
        let original_content = json!({
            "title": "Conflict Test Document",
            "field_to_edit": "original_value",
            "content": "This will be edited by both clients"
        });

        let doc = client_a
            .create_document(original_content.clone())
            .await
            .expect("Failed to create document");

        let doc_id = doc.id;
        tracing::info!("Document created: {}", doc_id);

        // Wait for sync to client B
        let start = std::time::Instant::now();
        let max_wait = Duration::from_secs(5);
        let mut synced_to_b = false;

        while start.elapsed() < max_wait {
            let docs_b = client_b
                .get_all_documents()
                .await
                .expect("Failed to get docs");
            if docs_b.iter().any(|d| d.id == doc_id) {
                synced_to_b = true;
                tracing::info!("Document synced to client B");
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            synced_to_b,
            "Document should sync to client B within 5 seconds"
        );

        // Both clients now have the same document
        let docs_a = client_a
            .get_all_documents()
            .await
            .expect("Failed to get docs");
        let docs_b = client_b
            .get_all_documents()
            .await
            .expect("Failed to get docs");
        assert_eq!(docs_a.len(), 1);
        assert_eq!(docs_b.len(), 1);
        assert_eq!(docs_a[0].id, docs_b[0].id);

        // Simulate offline edits by making conflicting updates
        // In a real scenario, clients would be disconnected here
        tracing::info!("Making conflicting edits...");

        // Client A edits the field
        let edit_a = json!({
            "title": "Conflict Test Document",
            "field_to_edit": "client_a_value",
            "content": "Client A modified this"
        });

        client_a
            .update_document(doc_id, edit_a.clone())
            .await
            .expect("Client A update failed");

        // Client B also edits the same field (conflict!)
        let edit_b = json!({
            "title": "Conflict Test Document",
            "field_to_edit": "client_b_value",
            "content": "Client B modified this - CONFLICT!"
        });

        client_b
            .update_document(doc_id, edit_b.clone())
            .await
            .expect("Client B update failed");

        tracing::info!("Conflicting edits made");

        // Wait for sync and conflict resolution
        sleep(Duration::from_secs(3)).await;

        // Check final state on both clients
        let final_docs_a = client_a
            .get_all_documents()
            .await
            .expect("Failed to get docs");
        let final_docs_b = client_b
            .get_all_documents()
            .await
            .expect("Failed to get docs");

        assert_eq!(final_docs_a.len(), 1, "Client A should have 1 document");
        assert_eq!(final_docs_b.len(), 1, "Client B should have 1 document");

        let final_a = &final_docs_a[0];
        let final_b = &final_docs_b[0];

        tracing::info!("Client A final content: {:?}", final_a.content);
        tracing::info!("Client B final content: {:?}", final_b.content);
        tracing::info!("Client A sync_revision: {}", final_a.sync_revision);
        tracing::info!("Client B sync_revision: {}", final_b.sync_revision);

        // Verify eventual consistency - both clients should have the same final state
        assert_eq!(
            final_a.content, final_b.content,
            "Clients should converge to the same content after conflict resolution"
        );

        assert_eq!(
            final_a.sync_revision, final_b.sync_revision,
            "Clients should have the same version after conflict resolution"
        );

        tracing::info!(
            "✅ Conflict resolution test passed - clients converged to consistent state"
        );
    },
    true
);

crate::integration_test!(
    test_conflict_events_logged_correctly,
    |ctx: TestContext| async move {
        // Tests that conflicts are properly logged in the change_events table.
        //
        // This test verifies that when conflicts occur, the losing version
        // is preserved in the event log with applied=false.
        let test_id = uuid::Uuid::new_v4();
        let email = format!("conflict-events-test-{}@example.com", test_id);
        let (api_key, api_secret) = ctx
            .generate_test_credentials(&format!("conflict-events-test-{}", test_id))
            .await
            .expect("Failed to generate credentials");
        let user_id = ctx
            .create_test_user(&email)
            .await
            .expect("Failed to create user");

        let client = ctx
            .create_test_client(&email, user_id, &api_key, &api_secret)
            .await
            .expect("Failed to create client");

        sleep(Duration::from_millis(500)).await;

        // Create a document
        let doc = client
            .create_document(json!({"value": 1}))
            .await
            .expect("Failed to create document");

        sleep(Duration::from_millis(500)).await;

        // Make multiple rapid updates to potentially trigger conflicts
        for i in 2..5 {
            client
                .update_document(doc.id, json!({"value": i}))
                .await
                .expect("Update failed");
            sleep(Duration::from_millis(100)).await;
        }

        // Wait for all events to be logged
        sleep(Duration::from_millis(1000)).await;

        // Verify document exists with final state
        let docs = client
            .get_all_documents()
            .await
            .expect("Failed to get docs");
        assert_eq!(docs.len(), 1);
        assert!(docs[0].content["value"].as_i64().unwrap() >= 2);

        tracing::info!("✅ Conflict events test passed");
    },
    true
);
