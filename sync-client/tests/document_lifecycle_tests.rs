//! # Document Lifecycle Tests
//!
//! Tests for the core document lifecycle: creation, retrieval, and deletion.
//! These tests verify that documents transition through sync states correctly.
//!
//! Tests cover:
//! - New local documents start as pending
//! - Pending documents can be queried
//! - Delete operations mark documents as pending until synced

mod common;

use common::*;
use uuid::Uuid;

/// Verifies that newly created local documents start with "pending" sync status.
/// This is critical for offline-first functionality - documents created while
/// offline must be tracked for later sync.
#[tokio::test]
async fn test_new_local_documents_are_pending() {
    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let doc = make_document(user_id, "Local Document", "Local content", 1);

    // Save without specifying status (should default to pending)
    db.save_document(&doc).await.unwrap();

    let sync_status = get_sync_status(&db, doc.id).await;
    assert_eq!(
        sync_status, "pending",
        "New local documents should be marked as pending"
    );
}

/// Verifies that the get_pending_documents query correctly filters and retrieves
/// only documents with pending sync status.
#[tokio::test]
async fn test_pending_documents_can_be_retrieved() {
    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let mut doc_ids = Vec::new();

    // Create mix of pending and synced documents
    for i in 0..5 {
        let doc = make_document(
            user_id,
            &format!("Document {}", i),
            &format!("Content {}", i),
            1,
        );

        doc_ids.push(doc.id);

        // Save every other document as synced, rest as pending
        if i % 2 == 0 {
            db.save_document(&doc).await.unwrap();
            db.mark_synced(&doc.id).await.unwrap();
        } else {
            db.save_document(&doc).await.unwrap(); // defaults to pending
        }
    }

    // Get pending documents
    let pending_docs = db.get_pending_documents().await.unwrap();

    // Should have 2 pending documents (indices 1, 3)
    assert_eq!(pending_docs.len(), 2, "Should have 2 pending documents");

    // Verify the pending ones are the odd indices
    assert!(
        pending_docs.iter().any(|p| p.id == doc_ids[1]),
        "Document 1 should be pending"
    );
    assert!(
        pending_docs.iter().any(|p| p.id == doc_ids[3]),
        "Document 3 should be pending"
    );
    assert!(
        !pending_docs.iter().any(|p| p.id == doc_ids[0]),
        "Document 0 should not be pending"
    );
    assert!(
        !pending_docs.iter().any(|p| p.id == doc_ids[2]),
        "Document 2 should not be pending"
    );
    assert!(
        !pending_docs.iter().any(|p| p.id == doc_ids[4]),
        "Document 4 should not be pending"
    );
}

/// Verifies the complete deletion workflow:
/// 1. Document starts synced
/// 2. Delete operation marks it pending
/// 3. After server confirmation, it's marked synced again
///
/// This ensures deletes are tracked for sync just like creates and updates.
#[tokio::test]
async fn test_delete_marks_document_as_pending_then_synced() {
    let db = setup_test_db().await;
    let user_id = Uuid::new_v4();
    let doc = make_document(user_id, "Test Document", "Test content", 1);

    // Create and sync a document
    db.save_document(&doc).await.unwrap();
    db.mark_synced(&doc.id, &doc.revision_id).await.unwrap();

    // Verify it's synced
    let sync_status = get_sync_status(&db, doc.id).await;
    assert_eq!(sync_status, "synced", "Document should start as synced");

    // Delete the document
    db.delete_document(&doc.id).await.unwrap();

    // Verify deletion marks it as pending
    let sync_status = get_sync_status(&db, doc.id).await;
    assert_eq!(
        sync_status, "pending",
        "Deleted document should be marked as pending sync"
    );

    // Simulate server confirmation
    db.mark_synced(&doc.id, "2-deleted").await.unwrap();

    // Verify it's now synced
    let sync_status = get_sync_status(&db, doc.id).await;
    assert_eq!(
        sync_status, "synced",
        "Deleted document should be marked as synced after confirmation"
    );
}
