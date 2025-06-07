use sync_client::{SyncEngine, ClientDatabase};
use sync_core::models::{Document, VectorClock};
use sync_core::protocol::{ClientMessage, ServerMessage};
use std::time::Duration;
use tokio::time::sleep;
use sqlx::Row;
use uuid::Uuid;
use chrono::Utc;
use serde_json::json;

#[tokio::test]
async fn test_sync_engine_leaves_documents_pending_after_creation() {
    // This test demonstrates the BUG: SyncEngine.create_document() creates documents
    // but doesn't mark them as synced even when they should be
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Set up user config for the sync engine
    db.ensure_user_config("ws://localhost:9001", "test_token").await.unwrap();
    
    // Create a sync engine (this will be in "offline" mode since server isn't running)
    // but we can still test the local behavior
    let user_id = db.get_user_id().await.unwrap();
    
    // In offline mode, let's manually create a document to simulate what sync engine does
    let doc = Document {
        id: Uuid::new_v4(),
        user_id,
        title: "Test Doc".to_string(),
        content: json!({"text": "Test content"}),
        revision_id: "1-abc123".to_string(),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    // This simulates what the sync engine does - saves document with pending status
    db.save_document(&doc).await.unwrap();
    
    // Check the sync status - this will be 'pending' due to the bug in queries.rs line 192
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc.id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "BUG: Document is stuck in pending status");
    
    // Even when we manually call mark_synced (simulating server confirmation), 
    // the sync engine never does this automatically
    db.mark_synced(&doc.id, "2-confirmed").await.unwrap();
    
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc.id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "After manual mark_synced, status should be synced");
}

#[tokio::test]
async fn test_sync_engine_has_no_pending_document_recovery() {
    // This test demonstrates the BUG: There's no mechanism to sync pending documents
    // when connection is restored
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    db.ensure_user_config("ws://localhost:9001", "test_token").await.unwrap();
    let user_id = db.get_user_id().await.unwrap();
    
    // Create several "offline" documents that would be stuck in pending
    for i in 1..=3 {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: format!("Offline Doc {}", i),
            content: json!({"text": format!("Content {}", i)}),
            revision_id: "1-abc123".to_string(),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        
        db.save_document(&doc).await.unwrap();
    }
    
    // All documents should be pending
    let pending_count = sqlx::query("SELECT COUNT(*) FROM documents WHERE sync_status = 'pending'")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<i64, _>(0);
    
    assert_eq!(pending_count, 3, "All documents should be pending");
    
    // Now if we had a real sync engine, it should detect these pending documents
    // and sync them when connection is restored. But currently there's no such mechanism.
    
    // We can get the pending documents
    let pending_doc_ids = db.get_pending_documents().await.unwrap();
    assert_eq!(pending_doc_ids.len(), 3, "Should be able to retrieve pending documents");
    
    // But the sync engine has no method to automatically process these pending documents
    // This is the BUG we need to fix
}

#[tokio::test] 
async fn test_mark_synced_requires_revision_id() {
    // This test shows that mark_synced requires a revision_id parameter
    // but the sync engine may not always have the correct one from server responses
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    db.ensure_user_config("ws://localhost:9001", "test_token").await.unwrap();
    let user_id = db.get_user_id().await.unwrap();
    
    let doc = Document {
        id: Uuid::new_v4(),
        user_id,
        title: "Test Doc".to_string(),
        content: json!({"text": "Test content"}),
        revision_id: "1-local".to_string(),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    db.save_document(&doc).await.unwrap();
    
    // To mark as synced, we need the server's revision_id
    // But if the sync engine doesn't properly extract this from server responses,
    // it can't mark documents as synced
    db.mark_synced(&doc.id, "2-server-confirmed").await.unwrap();
    
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc.id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Document should be marked as synced with server revision");
}

#[tokio::test]
async fn test_document_to_params_always_sets_pending() {
    // This test demonstrates the core bug in queries.rs:192
    // document_to_params always sets sync_status to "pending"
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    db.ensure_user_config("ws://localhost:9001", "test_token").await.unwrap();
    let user_id = db.get_user_id().await.unwrap();
    
    // Create a document that should be synced
    let doc = Document {
        id: Uuid::new_v4(),
        user_id,
        title: "Should be synced".to_string(),
        content: json!({"text": "This should be synced"}),
        revision_id: "2-server-confirmed".to_string(),
        version: 2,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    // When we save this document, it should retain its synced status
    // But due to the bug in document_to_params, it will be forced to 'pending'
    db.save_document(&doc).await.unwrap();
    
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc.id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    // This assertion will pass, demonstrating the bug
    assert_eq!(sync_status, "pending", "BUG: document_to_params forces status to pending");
    
    // What it SHOULD be if the bug were fixed:
    // assert_eq!(sync_status, "synced", "Document should retain its synced status");
}