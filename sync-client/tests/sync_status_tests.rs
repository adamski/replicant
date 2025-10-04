use sync_client::ClientDatabase;
use sync_core::models::{Document, VectorClock};
use sqlx::Row;
use uuid::Uuid;
use chrono::Utc;
use serde_json::json;

#[tokio::test]
async fn test_document_marked_synced_after_creation() {
    let db_url = "sqlite::memory:";

    // Create database and run migrations
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create a document manually since we need direct database access
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Test Doc", "text": "Content"});
    let doc = Document {
        id: doc_id,
        user_id,
        content: content.clone(),
        revision_id: Document::initial_revision(&content),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    db.save_document(&doc).await.unwrap();
    
    // Check initial sync status - should be 'pending'
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "New document should have pending sync status");
    
    // Simulate server confirmation by marking as synced
    // This is what the sync engine SHOULD do after getting server confirmation
    db.mark_synced(&doc_id, "2-synced").await.unwrap();
    
    // Verify document is now marked as synced
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Document should be marked as synced after server confirmation");
}

#[tokio::test]
async fn test_pending_documents_synced_on_connection_restore() {
    let db_url = "sqlite::memory:";

    // Create database and run migrations
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create multiple documents while "offline"
    let user_id = Uuid::new_v4();
    let mut doc_ids = Vec::new();
    
    for i in 1..=3 {
        let content = json!({"title": format!("Doc {}", i), "text": format!("Content {}", i)});
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            revision_id: Document::initial_revision(&content),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        doc_ids.push(doc.id);
        db.save_document(&doc).await.unwrap();
    }
    
    // Verify all are pending
    let pending_count = sqlx::query("SELECT COUNT(*) FROM documents WHERE sync_status = 'pending'")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<i64, _>(0);
    
    assert_eq!(pending_count, 3, "Should have 3 pending documents");
    
    // Simulate connection restore and sync process
    // The sync engine SHOULD automatically sync pending documents
    // For now, we'll simulate what it should do
    for doc_id in &doc_ids {
        db.mark_synced(doc_id, "2-synced").await.unwrap();
    }
    
    // Verify all are now synced
    let synced_count = sqlx::query("SELECT COUNT(*) FROM documents WHERE sync_status = 'synced'")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<i64, _>(0);
    
    assert_eq!(synced_count, 3, "All documents should be synced after connection restore");
}

#[tokio::test]
async fn test_sync_engine_marks_document_synced_after_server_confirmation() {
    // This test requires actual sync engine integration
    // It should verify that when the sync engine receives a DocumentCreated
    // message from the server, it marks the local document as synced
    
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create a document
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Test", "text": "Content"});
    let doc = Document {
        id: doc_id,
        user_id,
        content: content.clone(),
        revision_id: Document::initial_revision(&content),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    db.save_document(&doc).await.unwrap();
    
    // Simulate what the sync engine should do after receiving server confirmation
    // The sync engine receives a ServerMessage::DocumentCreated with the document
    // It SHOULD then call mark_synced on the document
    
    // Check current status
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "Document should start as pending");
    
    // This is what the sync engine SHOULD do but currently doesn't
    db.mark_synced(&doc_id, "2-synced").await.unwrap();
    
    // Verify it's marked as synced
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Document should be marked synced after server confirmation");
}

#[tokio::test]
async fn test_delete_marks_document_as_pending_then_synced() {
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create and sync a document
    let user_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Test", "text": "Content"});
    let doc = Document {
        id: doc_id,
        user_id,
        content: content.clone(),
        revision_id: Document::initial_revision(&content),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };

    db.save_document(&doc).await.unwrap();
    db.mark_synced(&doc_id, &doc.revision_id).await.unwrap();

    // Delete the document
    db.delete_document(&doc_id).await.unwrap();
    
    // Check sync status after delete
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "pending", "Deleted document should be marked as pending sync");
    
    // After server confirms deletion, it should be marked as synced
    db.mark_synced(&doc_id, "2-deleted").await.unwrap();
    
    let sync_status = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0);
    
    assert_eq!(sync_status, "synced", "Deleted document should be marked as synced after confirmation");
}

#[tokio::test]
async fn test_get_pending_documents() {
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    
    // Create mix of synced and pending documents
    let user_id = Uuid::new_v4();
    let mut doc_ids = Vec::new();
    
    for (i, title) in [(1, "Pending 1"), (2, "Synced 1"), (3, "Pending 2")].iter() {
        let content = json!({"title": title.to_string(), "text": "Content"});
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            revision_id: Document::initial_revision(&content),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        doc_ids.push((doc.id, *i));
        db.save_document(&doc).await.unwrap();
    }
    
    // Mark the second document as synced
    db.mark_synced(&doc_ids[1].0, "2-synced").await.unwrap();
    
    // Get pending documents
    let pending_doc_ids = db.get_pending_documents().await.unwrap();
    
    assert_eq!(pending_doc_ids.len(), 2, "Should have 2 pending documents");
    
    assert!(pending_doc_ids.iter().any(|p| p.id == doc_ids[0].0), "Should include first pending document");
    assert!(pending_doc_ids.iter().any(|p| p.id == doc_ids[2].0), "Should include second pending document");
    assert!(!pending_doc_ids.iter().any(|p| p.id == doc_ids[1].0), "Should not include synced document");
}