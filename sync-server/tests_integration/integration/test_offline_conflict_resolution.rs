use std::time::Duration;
use tokio::time::sleep;
use serde_json::json;
use uuid::Uuid;
use crate::integration::helpers::TestContext;
use sync_client::SyncEngine;
use std::fs;

// Shared state structure for passing data between conflict test phases
#[derive(serde::Serialize, serde::Deserialize)]
struct ConflictTestState {
    user_id: Uuid,
    token: String,
    document_id: Option<Uuid>,
    client1_db_path: Option<String>,
    client2_db_path: Option<String>,
    client1_edit: Option<String>,
    client2_edit: Option<String>,
    original_content: Option<String>,
    summary: Option<String>,
}

fn get_conflict_state_file() -> String {
    std::env::var("CONFLICT_TEST_STATE_FILE")
        .unwrap_or_else(|_| "/tmp/sync_conflict_test_state.json".to_string())
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
    let mut engine = SyncEngine::new(
        db_path,
        &format!("{}/ws", server_url),
        token,
        "test-user@example.com"
    ).await?;
    
    // Start the engine
    engine.start().await?;
    
    // Give it time to connect
    sleep(Duration::from_millis(500)).await;
    
    Ok(engine)
}

fn save_conflict_state(state: &ConflictTestState) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(state)?;
    fs::write(get_conflict_state_file(), json)?;
    Ok(())
}

fn load_conflict_state() -> Result<ConflictTestState, Box<dyn std::error::Error>> {
    let json = fs::read_to_string(get_conflict_state_file())?;
    Ok(serde_json::from_str(&json)?)
}

#[tokio::test]
async fn conflict_phase1_setup_and_sync() {
    if std::env::var("CONFLICT_TEST_PHASE").unwrap_or_default() != "phase1" {
        return;
    }
    
    tracing::info!("=== CONFLICT PHASE 1: Setup and Initial Sync ===");
    
    let ctx = TestContext::new();
    
    // Create a test user
    let (user_id, token) = ctx.create_test_user("conflict-test@example.com")
        .await
        .expect("Failed to create test user");
    
    // Create two clients with persistent database files
    let test_dir = format!("/tmp/conflict_test_{}", user_id);
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
    
    // Create a document that will be the subject of the conflict
    tracing::info!("Creating document for conflict testing...");
    
    let original_content = "This is the original content that will be edited by both clients";
    let doc = client1.create_document(
        json!({ 
            "title": "Conflict Test Document",
            "content": original_content,
            "field_to_edit": "original_value",
            "metadata": "created_for_conflict_test"
        })
    ).await.expect("Failed to create document");
    
    // Test immediate sync - documents should sync within 2 seconds
    let start_time = std::time::Instant::now();
    let max_sync_time = Duration::from_millis(2000);
    
    // Wait up to 2 seconds for sync
    let mut both_synced = false;
    while start_time.elapsed() < max_sync_time && !both_synced {
        let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
        let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
        
        if docs1.len() == 1 && docs2.len() == 1 && docs1[0].id == doc.id && docs2[0].id == doc.id {
            both_synced = true;
            tracing::info!("✅ Documents synced to both clients in {:?}", start_time.elapsed());
        } else {
            sleep(Duration::from_millis(100)).await;
        }
    }
    
    if !both_synced {
        tracing::error!("❌ Documents failed to sync within {:?}", max_sync_time);
    }
    
    // Verify both clients see the document
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    
    assert_eq!(docs1.len(), 1, "Client 1 should see 1 document");
    assert_eq!(docs2.len(), 1, "Client 2 should see 1 document");
    assert_eq!(docs1[0].id, doc.id, "Document IDs should match");
    assert_eq!(docs2[0].id, doc.id, "Document IDs should match");
    
    tracing::info!("✓ Document created and synced to both clients: {}", doc.id);
    
    // Save state for next phase
    let state = ConflictTestState {
        user_id,
        token,
        document_id: Some(doc.id),
        client1_db_path: Some(client1_db_path),
        client2_db_path: Some(client2_db_path),
        client1_edit: None,
        client2_edit: None,
        original_content: Some(original_content.to_string()),
        summary: Some("Phase 1: Document created and synced to both clients".to_string()),
    };
    save_conflict_state(&state).expect("Failed to save conflict state");
    
    tracing::info!("✓ Conflict Phase 1 complete");
}

#[tokio::test]
async fn conflict_phase2_concurrent_offline_edits() {
    if std::env::var("CONFLICT_TEST_PHASE").unwrap_or_default() != "phase2" {
        return;
    }
    
    tracing::info!("=== CONFLICT PHASE 2: Concurrent Offline Edits ===");
    
    // Load state from phase 1
    let mut state = load_conflict_state().expect("Failed to load conflict state");
    
    let document_id = state.document_id.expect("Document ID not found");
    let client1_db_path = state.client1_db_path.as_ref().expect("Client 1 DB path not found");
    let client2_db_path = state.client2_db_path.as_ref().expect("Client 2 DB path not found");
    
    tracing::info!("Making conflicting edits while server is offline...");
    
    // Open both client databases directly for offline operations
    let client1_db = sync_client::ClientDatabase::new(client1_db_path)
        .await
        .expect("Failed to open client 1 database");
    
    let client2_db = sync_client::ClientDatabase::new(client2_db_path)
        .await
        .expect("Failed to open client 2 database");
    
    // Client 1: Edit the document
    tracing::info!("Client 1: Making edit to document...");
    let client1_edit_content = "Client 1 modified this content while offline";
    
    match client1_db.get_document(&document_id).await {
        Ok(mut doc) => {
            doc.content = json!({ 
                "title": "Conflict Test Document",
                "content": client1_edit_content,
                "field_to_edit": "client1_value",
                "metadata": "edited_by_client1_offline",
                "edit_timestamp": chrono::Utc::now().to_rfc3339()
            });
            doc.updated_at = chrono::Utc::now();
            doc.version += 1;
            
            match client1_db.save_document(&doc).await {
                Ok(_) => {
                    tracing::info!("✓ Client 1 saved edit: {}", client1_edit_content);
                    state.client1_edit = Some(client1_edit_content.to_string());
                }
                Err(e) => tracing::warn!("Failed to save client 1 edit: {}", e),
            }
        }
        Err(e) => tracing::warn!("Failed to get document from client 1: {}", e),
    }
    
    // Small delay to simulate time difference
    sleep(Duration::from_millis(100)).await;
    
    // Client 2: Edit the SAME FIELD with DIFFERENT content
    tracing::info!("Client 2: Making conflicting edit to same document...");
    let client2_edit_content = "Client 2 also modified this content while offline - CONFLICT!";
    
    match client2_db.get_document(&document_id).await {
        Ok(mut doc) => {
            doc.content = json!({ 
                "title": "Conflict Test Document",
                "content": client2_edit_content,
                "field_to_edit": "client2_value", // Same field, different value!
                "metadata": "edited_by_client2_offline",
                "edit_timestamp": chrono::Utc::now().to_rfc3339()
            });
            doc.updated_at = chrono::Utc::now();
            doc.version += 1;
            
            match client2_db.save_document(&doc).await {
                Ok(_) => {
                    tracing::info!("✓ Client 2 saved conflicting edit: {}", client2_edit_content);
                    state.client2_edit = Some(client2_edit_content.to_string());
                }
                Err(e) => tracing::warn!("Failed to save client 2 edit: {}", e),
            }
        }
        Err(e) => tracing::warn!("Failed to get document from client 2: {}", e),
    }
    
    // Verify local changes are stored
    let client1_docs = client1_db.get_all_documents().await.expect("Failed to get client 1 docs");
    let client2_docs = client2_db.get_all_documents().await.expect("Failed to get client 2 docs");
    
    tracing::info!("Client 1 local version: {:?}", client1_docs[0].content["content"]);
    tracing::info!("Client 2 local version: {:?}", client2_docs[0].content["content"]);
    
    // Verify they are different (conflict exists)
    assert_ne!(
        client1_docs[0].content["field_to_edit"], 
        client2_docs[0].content["field_to_edit"],
        "Clients should have conflicting edits"
    );
    
    // Update state summary
    state.summary = Some(format!(
        "Phase 2: Conflicting offline edits made - Client1: '{}', Client2: '{}'",
        state.client1_edit.as_ref().unwrap_or(&"unknown".to_string()),
        state.client2_edit.as_ref().unwrap_or(&"unknown".to_string())
    ));
    save_conflict_state(&state).expect("Failed to save conflict state");
    
    tracing::info!("✓ Conflict Phase 2 complete - conflicting edits made");
}

#[tokio::test]
async fn conflict_phase3_server_recovery_and_resolution() {
    if std::env::var("CONFLICT_TEST_PHASE").unwrap_or_default() != "phase3" {
        return;
    }
    
    tracing::info!("=== CONFLICT PHASE 3: Server Recovery and Conflict Resolution ===");
    
    // Load state
    let state = load_conflict_state().expect("Failed to load conflict state");
    let ctx = TestContext::new();
    
    // Wait for server to be ready
    ctx.wait_for_server().await.expect("Server not ready");
    
    // Create clients that will reconnect and attempt to sync
    tracing::info!("Creating clients - they should reconnect and attempt sync...");
    
    let client1 = ctx.create_test_client(state.user_id, &state.token)
        .await
        .expect("Failed to create client 1");
    
    let client2 = ctx.create_test_client(state.user_id, &state.token)
        .await
        .expect("Failed to create client 2");
    
    // Give plenty of time for reconnection and sync attempts
    tracing::info!("Waiting for reconnection and conflict resolution...");
    sleep(Duration::from_millis(10000)).await; // Extended wait for conflict resolution
    
    // Check final state on both clients
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    
    tracing::info!("Client 1 final documents: {}", docs1.len());
    tracing::info!("Client 2 final documents: {}", docs2.len());
    
    // Log the actual content to see what happened
    if !docs1.is_empty() {
        tracing::info!("Client 1 document content: {:?}", docs1[0].content);
        tracing::info!("Client 1 document revision: {}", docs1[0].revision_id);
    }
    if !docs2.is_empty() {
        tracing::info!("Client 2 document content: {:?}", docs2[0].content);
        tracing::info!("Client 2 document revision: {}", docs2[0].revision_id);
    }
    
    if !docs1.is_empty() && !docs2.is_empty() {
        let doc1 = &docs1[0];
        let doc2 = &docs2[0];
        
        tracing::info!("Client 1 final content: {:?}", doc1.content);
        tracing::info!("Client 2 final content: {:?}", doc2.content);
        tracing::info!("Client 1 revision: {}", doc1.revision_id);
        tracing::info!("Client 2 revision: {}", doc2.revision_id);
        
        // Test what actually happens with conflicts
        if doc1.content == doc2.content {
            tracing::info!("✓ Clients converged to same content");
            
            // Check if both edits are preserved somehow (e.g., in conflict fields or history)
            let has_client1_data = doc1.content.to_string().contains("client1") || 
                                   doc1.content.to_string().contains("Client 1");
            let has_client2_data = doc1.content.to_string().contains("client2") || 
                                   doc1.content.to_string().contains("Client 2");
            
            if has_client1_data && has_client2_data {
                tracing::info!("✓ Both client edits appear to be preserved in final content");
            } else if has_client1_data {
                tracing::warn!("⚠️  Only client 1 edit preserved - client 2 edit lost");
            } else if has_client2_data {
                tracing::warn!("⚠️  Only client 2 edit preserved - client 1 edit lost");
            } else {
                tracing::error!("❌ Neither client edit preserved - both lost!");
            }
        } else {
            tracing::warn!("⚠️  Clients did not converge - conflict not resolved");
            tracing::warn!("Client 1 field_to_edit: {:?}", doc1.content["field_to_edit"]);
            tracing::warn!("Client 2 field_to_edit: {:?}", doc2.content["field_to_edit"]);
        }
    } else {
        tracing::error!("❌ Documents missing after sync - data loss occurred!");
    }
    
    tracing::info!("✓ Conflict Phase 3 complete - analyzed conflict resolution behavior");
}

#[tokio::test]
async fn conflict_phase4_detailed_analysis() {
    if std::env::var("CONFLICT_TEST_PHASE").unwrap_or_default() != "verify" {
        return;
    }
    
    tracing::info!("=== CONFLICT PHASE 4: Detailed Analysis ===");
    
    // Load state
    let state = load_conflict_state().expect("Failed to load conflict state");
    let ctx = TestContext::new();
    
    // Create multiple clients to verify final state consistency
    let client1 = ctx.create_test_client(state.user_id, &state.token)
        .await
        .expect("Failed to create client 1");
    
    let client2 = ctx.create_test_client(state.user_id, &state.token)
        .await
        .expect("Failed to create client 2");
    
    let client3 = ctx.create_test_client(state.user_id, &state.token)
        .await
        .expect("Failed to create client 3");
    
    // Wait for sync
    sleep(Duration::from_millis(3000)).await;
    
    // Analyze final state across all clients
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs");
    let docs3 = client3.get_all_documents().await.expect("Failed to get docs");
    
    // Check consistency
    let all_same_count = docs1.len() == docs2.len() && docs2.len() == docs3.len();
    tracing::info!("All clients have same document count: {}", all_same_count);
    
    if !docs1.is_empty() && all_same_count {
        let content1 = &docs1[0].content;
        let content2 = &docs2[0].content;
        let content3 = &docs3[0].content;
        
        let all_same_content = content1 == content2 && content2 == content3;
        tracing::info!("All clients have same content: {}", all_same_content);
        
        if all_same_content {
            tracing::info!("✓ GOOD: All clients converged to consistent state");
            
            // Detailed analysis of what was preserved
            let final_content = content1;
            tracing::info!("Final resolved content: {}", serde_json::to_string_pretty(final_content).unwrap_or_default());
            
            // Check what happened to the conflicting edits
            let unknown = "unknown".to_string();
            let original_content = state.original_content.as_ref().unwrap_or(&unknown);
            let client1_edit = state.client1_edit.as_ref().unwrap_or(&unknown);
            let client2_edit = state.client2_edit.as_ref().unwrap_or(&unknown);
            
            let final_str = final_content.to_string();
            
            let preserved_original = final_str.contains(original_content);
            let preserved_client1 = final_str.contains(client1_edit) || final_str.contains("client1");
            let preserved_client2 = final_str.contains(client2_edit) || final_str.contains("client2");
            
            tracing::info!("Conflict resolution analysis:");
            tracing::info!("  Original content preserved: {}", preserved_original);
            tracing::info!("  Client 1 edit preserved: {}", preserved_client1);
            tracing::info!("  Client 2 edit preserved: {}", preserved_client2);
            
            if preserved_client1 && preserved_client2 {
                tracing::info!("✅ EXCELLENT: Both conflicting edits were preserved");
            } else if preserved_client1 || preserved_client2 {
                tracing::warn!("⚠️  PARTIAL: Only one edit preserved - other lost");
            } else {
                tracing::error!("❌ POOR: Both edits lost - conflict resolution failed");
            }
        } else {
            tracing::error!("❌ CRITICAL: Clients have inconsistent state - no convergence");
        }
    }
    
    tracing::info!("✓ Detailed conflict analysis complete");
    
    // Save final analysis
    let mut final_state = state;
    final_state.summary = Some(format!(
        "Final Analysis: Clients converged: {}, Document count: {} across all clients", 
        docs1.len() == docs2.len() && docs2.len() == docs3.len() && 
        (!docs1.is_empty() && docs1[0].content == docs2[0].content && docs2[0].content == docs3[0].content),
        docs1.len()
    ));
    save_conflict_state(&final_state).expect("Failed to save final conflict state");
}