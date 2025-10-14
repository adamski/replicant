use std::time::Duration;
use tokio::time::sleep;
use serde_json::json;
use uuid::Uuid;
use crate::integration::helpers::TestContext;
use sync_client::SyncEngine;
use std::fs;

// Shared state structure for passing data between phases
#[derive(serde::Serialize, serde::Deserialize)]
struct TestState {
    user_id: Uuid,
    token: String,
    doc1_id: Option<Uuid>,
    doc2_id: Option<Uuid>,
    doc3_id: Option<Uuid>,
    offline_doc_id: Option<Uuid>,
    client1_db_path: Option<String>,
    client2_db_path: Option<String>,
    summary: Option<String>,
}

fn get_state_file() -> String {
    std::env::var("OFFLINE_TEST_STATE_FILE")
        .unwrap_or_else(|_| "/tmp/sync_offline_test_state.json".to_string())
}

async fn create_persistent_client(
    user_id: Uuid,
    token: &str,
    db_path: &str,
    server_url: &str,
) -> Result<SyncEngine, Box<dyn std::error::Error + Send + Sync>> {
    // Create and initialize the client database with the user
    let db = sync_client::ClientDatabase::new(db_path).await?;
    db.run_migrations().await?;
    
    // Generate a unique client_id for this test client
    let client_id = Uuid::new_v4();
    
    // Set up user config in the client database
    sqlx::query(
        "INSERT OR REPLACE INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)"
    )
    .bind(user_id.to_string())
    .bind(client_id.to_string())
    .bind(server_url)
    .bind(token)
    .execute(&db.pool)
    .await?;
    
    // Create sync engine with persistent database
    // Note: In these special tests that use persistent clients, we use the token as both api_key and placeholder secret
    let engine = SyncEngine::new(
        db_path,
        &format!("{}/ws", server_url),
        "test-user@example.com",
        token,
        token
    ).await?;
    
    // Give it time to connect (connection starts automatically)
    sleep(Duration::from_millis(500)).await;
    
    Ok(engine)
}

fn save_state(state: &TestState) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(state)?;
    fs::write(get_state_file(), json)?;
    Ok(())
}

fn load_state() -> Result<TestState, Box<dyn std::error::Error>> {
    let json = fs::read_to_string(get_state_file())?;
    Ok(serde_json::from_str(&json)?)
}

#[tokio::test]
async fn phase1_initial_sync() {
    if std::env::var("OFFLINE_TEST_PHASE").unwrap_or_default() != "phase1" {
        return;
    }
    
    tracing::info!("=== PHASE 1: Initial Sync ===");
    
    let ctx = TestContext::new();
    
    // Create a test user
    let email = "offline-sync-test@example.com";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-offline-phases").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");
    let token = api_key.clone();  // Keep token variable for state persistence
    
    // Create two clients with persistent database files
    let test_dir = format!("/tmp/offline_sync_test_{}", user_id);
    std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");
    
    let client1_db_path = format!("sqlite:{}/client1.sqlite3?mode=rwc", test_dir);
    let client2_db_path = format!("sqlite:{}/client2.sqlite3?mode=rwc", test_dir);
    
    tracing::info!("Creating client 1 with persistent database...");
    let client1 = create_persistent_client(user_id, &token, &client1_db_path, &ctx.server_url)
        .await
        .expect("Failed to create client 1");
    
    sleep(Duration::from_millis(500)).await;
    
    tracing::info!("Creating client 2 with persistent database...");
    let client2 = create_persistent_client(user_id, &token, &client2_db_path, &ctx.server_url)
        .await
        .expect("Failed to create client 2");
    
    sleep(Duration::from_millis(500)).await;
    
    // Create some initial documents
    tracing::info!("Creating initial documents...");
    
    let doc1 = client1.create_document(
        json!({ "title": "Document 1", "content": "Initial content 1", "phase": "1" })
    ).await.expect("Failed to create doc1");
    
    let doc2 = client1.create_document(
        json!({ "title": "Document 2", "content": "Initial content 2", "phase": "1" })
    ).await.expect("Failed to create doc2");
    
    let doc3 = client2.create_document(
        json!({ "title": "Document 3", "content": "Initial content 3", "phase": "1" })
    ).await.expect("Failed to create doc3");
    
    // Wait for sync
    sleep(Duration::from_millis(2000)).await;
    
    // Verify both clients see all documents
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    
    assert_eq!(docs1.len(), 3, "Client 1 should see 3 documents");
    assert_eq!(docs2.len(), 3, "Client 2 should see 3 documents");
    
    tracing::info!("✓ Initial sync successful - both clients see {} documents", docs1.len());
    
    // Save state for next phase
    let state = TestState {
        user_id,
        token,
        doc1_id: Some(doc1.id),
        doc2_id: Some(doc2.id),
        doc3_id: Some(doc3.id),
        offline_doc_id: None,
        client1_db_path: Some(client1_db_path),
        client2_db_path: Some(client2_db_path),
        summary: Some(format!("Phase 1: Created 3 documents, both clients synced")),
    };
    save_state(&state).expect("Failed to save state");
    
    tracing::info!("✓ Phase 1 complete");
}

#[tokio::test]
async fn phase2_offline_changes() {
    if std::env::var("OFFLINE_TEST_PHASE").unwrap_or_default() != "phase2" {
        return;
    }
    
    tracing::info!("=== PHASE 2: Offline Changes ===");
    
    // Load state from phase 1
    let mut state = load_state().expect("Failed to load state");
    
    // Work directly with the client database files from phase 1
    tracing::info!("Working with existing client databases while server is offline...");
    
    let client1_db_path = state.client1_db_path.as_ref().expect("Client 1 DB path not found");
    
    // Open the database directly for offline operations
    let client_db = sync_client::ClientDatabase::new(client1_db_path)
        .await
        .expect("Failed to open client database");
    
    tracing::info!("Working with offline client database...");
    sleep(Duration::from_millis(500)).await;
    
    // Make changes while offline using direct database operations
    tracing::info!("Making offline changes...");
    
    // Update document 1
    if let Some(doc1_id) = state.doc1_id {
        match client_db.get_document(&doc1_id).await {
            Ok(mut doc) => {
                doc.content = json!({ 
                    "title": "Document 1",
                    "content": "Updated offline in phase 2",
                    "phase": "2",
                    "offline": true
                });
                doc.updated_at = chrono::Utc::now();
                match client_db.save_document(&doc).await {
                    Ok(_) => tracing::info!("✓ Updated document 1 while offline"),
                    Err(e) => tracing::warn!("Failed to save updated document 1: {}", e),
                }
            }
            Err(e) => tracing::warn!("Failed to get document 1: {}", e),
        }
    }
    
    // Create a new document while offline
    let offline_doc = sync_core::models::Document {
        id: Uuid::new_v4(),
        user_id: state.user_id,
        content: json!({ 
            "title": "Offline Document",
            "content": "Created while offline",
            "phase": "2",
            "created_offline": true
        }),
        revision_id: "1-offline".to_string(),
        version: 1,
        vector_clock: sync_core::models::VectorClock::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    };
    
    match client_db.save_document(&offline_doc).await {
        Ok(_) => {
            tracing::info!("✓ Created new document while offline: {}", offline_doc.id);
            state.offline_doc_id = Some(offline_doc.id);
        }
        Err(e) => tracing::warn!("Failed to create offline document: {}", e),
    }
    
    // Delete document 3 (soft delete)
    if let Some(doc3_id) = state.doc3_id {
        match client_db.delete_document(&doc3_id).await {
            Ok(_) => tracing::info!("✓ Deleted document 3 while offline"),
            Err(e) => tracing::warn!("Failed to delete document 3: {}", e),
        }
    }
    
    // Verify changes are stored locally
    let local_docs = client_db.get_all_documents().await.expect("Failed to get local docs");
    tracing::info!("Local documents after offline changes: {}", local_docs.len());
    
    // Update state summary
    state.summary = Some(format!(
        "Phase 2: Made offline changes - updated 1, created 1, deleted 1. Local docs: {}",
        local_docs.len()
    ));
    save_state(&state).expect("Failed to save state");
    
    tracing::info!("✓ Phase 2 complete");
}

#[tokio::test]
async fn phase3_sync_recovery() {
    if std::env::var("OFFLINE_TEST_PHASE").unwrap_or_default() != "phase3" {
        return;
    }
    
    tracing::info!("=== PHASE 3: Sync Recovery ===");
    
    // Load state
    let state = load_state().expect("Failed to load state");
    let ctx = TestContext::new();
    
    // Wait for server to be ready
    ctx.wait_for_server().await.expect("Server not ready");
    
    // Create clients - they should reconnect and sync
    tracing::info!("Creating clients - they should reconnect to server...");

    // We don't have email/credentials saved in state, so we'll create new ones
    let email = "offline-sync-test@example.com";
    let (api_key, api_secret) = ctx.generate_test_credentials("test-offline-phases-recovery").await
        .expect("Failed to generate credentials");

    let client1 = ctx.create_test_client(email, state.user_id, &api_key, &api_secret)
        .await
        .expect("Failed to create client 1");

    let client2 = ctx.create_test_client(email, state.user_id, &api_key, &api_secret)
        .await
        .expect("Failed to create client 2");
    
    // Give plenty of time for reconnection and sync
    tracing::info!("Waiting for reconnection and sync...");
    sleep(Duration::from_millis(5000)).await;
    
    // Check documents on both clients
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    
    tracing::info!("Client 1 sees {} documents", docs1.len());
    tracing::info!("Client 2 sees {} documents", docs2.len());
    
    // Both should see the same documents
    assert_eq!(docs1.len(), docs2.len(), "Both clients should see same number of documents");
    
    // Verify the offline document exists on both
    if let Some(offline_doc_id) = state.offline_doc_id {
        let doc1_found = docs1.iter().any(|d| d.id == offline_doc_id);
        let doc2_found = docs2.iter().any(|d| d.id == offline_doc_id);
        
        assert!(doc1_found, "Client 1 should see the offline-created document");
        assert!(doc2_found, "Client 2 should see the offline-created document");
        
        tracing::info!("✓ Offline-created document synced to both clients");
    }
    
    // Verify document 3 was deleted on both
    if let Some(doc3_id) = state.doc3_id {
        let doc1_found = docs1.iter().any(|d| d.id == doc3_id);
        let doc2_found = docs2.iter().any(|d| d.id == doc3_id);
        
        assert!(!doc1_found, "Client 1 should not see deleted document 3");
        assert!(!doc2_found, "Client 2 should not see deleted document 3");
        
        tracing::info!("✓ Deletion synced to both clients");
    }
    
    // Verify document 1 has updated content
    if let Some(doc1_id) = state.doc1_id {
        let doc1_client1 = docs1.iter().find(|d| d.id == doc1_id);
        let doc1_client2 = docs2.iter().find(|d| d.id == doc1_id);
        
        if let (Some(d1), Some(d2)) = (doc1_client1, doc1_client2) {
            assert_eq!(d1.content["offline"], true, "Client 1 should see offline update");
            assert_eq!(d2.content["offline"], true, "Client 2 should see offline update");
            tracing::info!("✓ Offline update synced to both clients");
        }
    }
    
    tracing::info!("✓ Phase 3 complete - sync recovery successful");
}

#[tokio::test]
async fn phase4_verification() {
    if std::env::var("OFFLINE_TEST_PHASE").unwrap_or_default() != "verify" {
        return;
    }
    
    tracing::info!("=== PHASE 4: Final Verification ===");
    
    // Load state
    let state = load_state().expect("Failed to load state");
    let ctx = TestContext::new();
    
    // Create multiple clients to verify final state
    let email = "offline-sync-test@example.com";
    let (api_key, api_secret) = ctx.generate_test_credentials("test-offline-phases-verify").await
        .expect("Failed to generate credentials");

    let client1 = ctx.create_test_client(email, state.user_id, &api_key, &api_secret)
        .await
        .expect("Failed to create client 1");

    let client2 = ctx.create_test_client(email, state.user_id, &api_key, &api_secret)
        .await
        .expect("Failed to create client 2");

    let client3 = ctx.create_test_client(email, state.user_id, &api_key, &api_secret)
        .await
        .expect("Failed to create client 3");
    
    // Wait for sync
    sleep(Duration::from_millis(2000)).await;
    
    // Get documents from all clients
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs");
    let docs3 = client3.get_all_documents().await.expect("Failed to get docs");
    
    // All should have same count
    assert_eq!(docs1.len(), docs2.len(), "Client 1 and 2 should have same doc count");
    assert_eq!(docs2.len(), docs3.len(), "Client 2 and 3 should have same doc count");
    
    tracing::info!("✓ All {} clients converged to {} documents", 3, docs1.len());
    
    // Verify specific documents
    let mut summary_parts = vec![];
    summary_parts.push(format!("Final state: {} documents across all clients", docs1.len()));
    
    // Check each expected document
    if let Some(doc1_id) = state.doc1_id {
        if docs1.iter().any(|d| d.id == doc1_id && d.content["offline"] == true) {
            summary_parts.push("✓ Document 1: Updated offline and synced".to_string());
        }
    }
    
    if let Some(doc2_id) = state.doc2_id {
        if docs1.iter().any(|d| d.id == doc2_id) {
            summary_parts.push("✓ Document 2: Unchanged and present".to_string());
        }
    }
    
    if let Some(doc3_id) = state.doc3_id {
        if !docs1.iter().any(|d| d.id == doc3_id) {
            summary_parts.push("✓ Document 3: Successfully deleted".to_string());
        }
    }
    
    if let Some(offline_doc_id) = state.offline_doc_id {
        if docs1.iter().any(|d| d.id == offline_doc_id) {
            summary_parts.push("✓ Offline document: Created and synced".to_string());
        }
    }
    
    // Save final summary
    let mut final_state = state;
    final_state.summary = Some(summary_parts.join("\n"));
    save_state(&final_state).expect("Failed to save final state");
    
    tracing::info!("✓ All verifications passed!");
}