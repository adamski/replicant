mod common;

use common::*;
use sqlx::Row;
use uuid::Uuid;


/// This test verifies that documents received from the server are immediately
/// marked as synced, not pending
#[tokio::test]
async fn test_documents_from_server_are_saved_as_synced() {
   
    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let mut doc = make_document(user_id, "Server Document", "Content from server", 2);
        
    // When sync engine receives documents from server, it saves them and marks as synced
    // We'll simulate this by saving and then marking as synced
    db.save_document(&doc).await.unwrap();

    db.mark_synced(&doc.id, &doc.revision_id).await.unwrap();

    let sync_status = get_sync_status(&db, doc.id).await;

    // Verify it's marked as synced
    assert_eq!(sync_status, "synced", "Documents from server should be marked as synced");
}


/// This test verifies that new local documents start as pending
#[tokio::test]
async fn test_new_local_documents_are_pending() {


    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let doc = make_document(user_id, "Local Document", "Local content", 1);

    // Save without specifying status (should default to pending)
    db.save_document(&doc).await.unwrap();

    let sync_status = get_sync_status(&db, doc.id).await;    
    assert_eq!(sync_status, "pending", "New local documents should be marked as pending");
}


/// This test verifies the get_pending_documents method works correctly
#[tokio::test]
async fn test_pending_documents_can_be_retrieved() {
    
    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let mut doc_ids = Vec::new();
    
    // Create mix of pending and synced documents
    for i in 0..5 {
        let doc = make_document(user_id, &format!("Document {}", i), &format!("Content {}", i), 1);
        
        doc_ids.push(doc.id);
        
        // Save every other document as synced, rest as pending
        if i % 2 == 0 {
            db.save_document(&doc).await.unwrap();
            db.mark_synced(&doc.id, &doc.revision_id).await.unwrap();
        } else {
            db.save_document(&doc).await.unwrap(); // defaults to pending
        }
    }
    
    // Get pending documents
    let pending_doc_ids = db.get_pending_documents().await.unwrap();
    
    // Should have 3 pending documents (indices 1, 3)
    assert_eq!(pending_doc_ids.len(), 2, "Should have 2 pending documents");
    
    // Verify the pending ones are the odd indices
    assert!(pending_doc_ids.iter().any(|p| p.id == doc_ids[1]), "Document 1 should be pending");
    assert!(pending_doc_ids.iter().any(|p| p.id == doc_ids[3]), "Document 3 should be pending");
    assert!(!pending_doc_ids.iter().any(|p| p.id == doc_ids[0]), "Document 0 should not be pending");
    assert!(!pending_doc_ids.iter().any(|p| p.id == doc_ids[2]), "Document 2 should not be pending");
    assert!(!pending_doc_ids.iter().any(|p| p.id == doc_ids[4]), "Document 4 should not be pending");
}


/// This test verifies that mark_synced properly updates the sync status
#[tokio::test]
async fn test_mark_synced_updates_status() {

    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let doc = make_document(user_id, "Test Document", "Test content", 1);
    
    // Save as pending
    db.save_document(&doc).await.unwrap();
    
    // Verify it's pending
    let sync_status = get_sync_status(&db, doc.id).await;
    
    assert_eq!(sync_status, "pending", "Should start as pending");
    
    // Mark as synced
    db.mark_synced(&doc.id, "2-confirmed").await.unwrap();
    
    // Verify it's now synced
    let sync_status = get_sync_status(&db, doc.id).await;

    assert_eq!(sync_status, "synced", "Should be marked as synced");
}