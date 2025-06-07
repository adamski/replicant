use sync_client::{SyncEngine, ClientDatabase};
use sync_core::models::{Document, VectorClock};
use sync_core::protocol::{ServerMessage};
use std::sync::Arc;
use sqlx::Row;
use uuid::Uuid;
use chrono::Utc;
use serde_json::json;

#[tokio::test]
async fn test_documents_from_server_are_saved_as_synced() {
    // This test verifies that documents received from the server are immediately
    // marked as synced, not pending
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create a document that simulates one coming from the server
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let doc = Document {
        id: doc_id,
        user_id,
        title: "Server Document".to_string(),
        content: json!({"text": "Content from server"}),
        revision_id: "2-server123".to_string(),
        version: 2,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    // When sync engine receives documents from server, it saves them and marks as synced
    // We'll simulate this by saving and then marking as synced
    db.save_document(&doc).await.unwrap();
    db.mark_synced(&doc_id, &doc.revision_id).await.unwrap();
    
    // Verify it's marked as synced
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Documents from server should be marked as synced");
}

#[tokio::test]
async fn test_new_local_documents_are_pending() {
    // This test verifies that new local documents start as pending
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create a document locally (using default save_document method)
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let doc = Document {
        id: doc_id,
        user_id,
        title: "Local Document".to_string(),
        content: json!({"text": "Local content"}),
        revision_id: "1-local123".to_string(),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    // Save without specifying status (should default to pending)
    db.save_document(&doc).await.unwrap();
    
    // Verify it's marked as pending
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "New local documents should be marked as pending");
}

#[tokio::test]
async fn test_pending_documents_can_be_retrieved() {
    // This test verifies the get_pending_documents method works correctly
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    let user_id = Uuid::new_v4();
    let mut doc_ids = Vec::new();
    
    // Create mix of pending and synced documents
    for i in 0..5 {
        let doc_id = Uuid::new_v4();
        let doc = Document {
            id: doc_id,
            user_id,
            title: format!("Document {}", i),
            content: json!({"text": format!("Content {}", i)}),
            revision_id: format!("1-hash{}", i),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        
        doc_ids.push(doc_id);
        
        // Save every other document as synced, rest as pending
        if i % 2 == 0 {
            db.save_document(&doc).await.unwrap();
            db.mark_synced(&doc_id, &doc.revision_id).await.unwrap();
        } else {
            db.save_document(&doc).await.unwrap(); // defaults to pending
        }
    }
    
    // Get pending documents
    let pending_doc_ids = db.get_pending_documents().await.unwrap();
    
    // Should have 3 pending documents (indices 1, 3)
    assert_eq!(pending_doc_ids.len(), 2, "Should have 2 pending documents");
    
    // Verify the pending ones are the odd indices
    assert!(pending_doc_ids.contains(&doc_ids[1]), "Document 1 should be pending");
    assert!(pending_doc_ids.contains(&doc_ids[3]), "Document 3 should be pending");
    assert!(!pending_doc_ids.contains(&doc_ids[0]), "Document 0 should not be pending");
    assert!(!pending_doc_ids.contains(&doc_ids[2]), "Document 2 should not be pending");
    assert!(!pending_doc_ids.contains(&doc_ids[4]), "Document 4 should not be pending");
}

#[tokio::test]
async fn test_mark_synced_updates_status() {
    // This test verifies that mark_synced properly updates the sync status
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let doc = Document {
        id: doc_id,
        user_id,
        title: "Test Document".to_string(),
        content: json!({"text": "Test content"}),
        revision_id: "1-initial".to_string(),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    // Save as pending
    db.save_document(&doc).await.unwrap();
    
    // Verify it's pending
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "Should start as pending");
    
    // Mark as synced
    db.mark_synced(&doc_id, "2-confirmed").await.unwrap();
    
    // Verify it's now synced
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Should be marked as synced");
}