//! Conflict handling tests: hash mismatch, duplicate IDs

use super::{connect_subject, serial, skip_if_no_server, temp_db_path, TestClient, TEST_EMAIL};
use replicant_client::ClientDatabase;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn test_update_with_wrong_hash_fails() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Create a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Hash Test", "version": 1});
    client
        .create_document_with_id(doc_id, content)
        .await
        .unwrap();

    // Try to update with wrong content hash
    let patch = json!([{"op": "replace", "path": "/title", "value": "Should Fail"}]);
    let result = client
        .update_document(doc_id, patch, "wrong_hash_value")
        .await;

    // Should fail with hash_mismatch
    assert!(result.is_err(), "Update with wrong hash should fail");
    let error = result.unwrap_err();
    assert!(
        error.contains("hash_mismatch") || error.contains("error"),
        "Error should indicate hash mismatch: {}",
        error
    );
}

#[tokio::test]
#[serial]
async fn test_stale_hash_returns_current_state() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Create a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Original", "data": "initial"});
    let create_result = client
        .create_document_with_id(doc_id, content)
        .await
        .unwrap();
    let first_hash = create_result
        .get("content_hash")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Update the document to get a new hash
    let patch1 = json!([{"op": "replace", "path": "/title", "value": "First Update"}]);
    client
        .update_document(doc_id, patch1, &first_hash)
        .await
        .unwrap();

    // Try to update with the stale (first) hash
    let patch2 = json!([{"op": "replace", "path": "/title", "value": "Should Fail"}]);
    let result = client.update_document(doc_id, patch2, &first_hash).await;

    // Should fail because hash is stale
    assert!(result.is_err(), "Update with stale hash should fail");
}

#[tokio::test]
#[serial]
async fn test_duplicate_document_id_returns_conflict() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Create a document with a specific ID
    let doc_id = Uuid::new_v4();
    let content1 = json!({"title": "First Document"});
    let result1 = client.create_document_with_id(doc_id, content1).await;
    assert!(result1.is_ok(), "First create should succeed");

    // Try to create another document with the same ID
    let content2 = json!({"title": "Duplicate Document"});
    let result2 = client.create_document_with_id(doc_id, content2).await;

    // Should fail with conflict
    assert!(result2.is_err(), "Duplicate ID should fail");
    let error = result2.unwrap_err();
    assert!(
        error.contains("conflict") || error.contains("error"),
        "Error should indicate conflict: {}",
        error
    );
}

#[tokio::test]
#[serial]
async fn test_update_nonexistent_document_fails() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Try to update a document that doesn't exist
    let nonexistent_id = Uuid::new_v4();
    let patch = json!([{"op": "replace", "path": "/title", "value": "Won't Work"}]);
    let result = client
        .update_document(nonexistent_id, patch, "any_hash")
        .await;

    assert!(
        result.is_err(),
        "Update of nonexistent document should fail"
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("not_found") || error.contains("error"),
        "Error should indicate not found: {}",
        error
    );
}

#[tokio::test]
#[serial]
async fn test_delete_nonexistent_document_fails() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Try to delete a document that doesn't exist
    let nonexistent_id = Uuid::new_v4();
    let result = client.delete_document(nonexistent_id).await;

    assert!(
        result.is_err(),
        "Delete of nonexistent document should fail"
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("not_found") || error.contains("error"),
        "Error should indicate not found: {}",
        error
    );
}

#[tokio::test]
#[serial]
async fn test_concurrent_updates_one_wins() {
    if skip_if_no_server() {
        return;
    }

    // Two clients as the same user
    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Create a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Concurrent Test", "counter": 0});
    let create_result = client_a
        .create_document_with_id(doc_id, content)
        .await
        .unwrap();
    let content_hash = create_result
        .get("content_hash")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Both clients try to update with the same hash (simulating concurrent edits)
    let patch_a = json!([{"op": "replace", "path": "/title", "value": "Client A Wins"}]);
    let patch_b = json!([{"op": "replace", "path": "/title", "value": "Client B Wins"}]);

    // Client A's update should succeed
    let result_a = client_a
        .update_document(doc_id, patch_a, &content_hash)
        .await;
    assert!(result_a.is_ok(), "Client A's update should succeed");

    // Client B's update should fail (stale hash)
    let result_b = client_b
        .update_document(doc_id, patch_b, &content_hash)
        .await;
    assert!(
        result_b.is_err(),
        "Client B's update should fail with stale hash"
    );
}

// ---------------------------------------------------------------------------
// Rebase-and-resend on hash_mismatch (DEV-1037, Task 5)
// ---------------------------------------------------------------------------

/// Poll a client's local database until its content matches `expected`.
async fn wait_for_content(db_url: &str, id: Uuid, expected: &Value) -> Option<Value> {
    let db = ClientDatabase::new(db_url).await.unwrap();
    for _ in 0..150 {
        if let Ok(doc) = db.get_document(&id).await {
            if &doc.content == expected {
                return Some(doc.content);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    ClientDatabase::new(db_url)
        .await
        .unwrap()
        .get_document(&id)
        .await
        .ok()
        .map(|d| d.content)
}

async fn local_revision(db_url: &str, id: Uuid) -> Option<i64> {
    ClientDatabase::new(db_url)
        .await
        .unwrap()
        .get_document(&id)
        .await
        .ok()
        .map(|d| d.sync_revision)
}

/// Two clients edit different fields of the same document from the same base.
/// The loser's upload is rejected with `hash_mismatch`; it must rebase its
/// queued patch onto the winner's content and resend, so both clients AND the
/// server end up holding both edits.
#[tokio::test]
#[serial]
async fn concurrent_edits_to_different_fields_converge_on_both() {
    if skip_if_no_server() {
        return;
    }

    std::fs::create_dir_all("databases").ok();
    let winner_db_file = temp_db_path("rebase_winner");
    let loser_db_file = temp_db_path("rebase_loser");
    let winner_db = format!("sqlite:{}?mode=rwc", winner_db_file);
    let loser_db = format!("sqlite:{}?mode=rwc", loser_db_file);

    let winner = connect_subject(&winner_db).await;
    let loser = connect_subject(&loser_db).await;

    let doc_id = Uuid::new_v4();
    let base = json!({"title": "base", "referenceFrequency": 440.0});
    winner
        .create_document_with_id(doc_id, base.clone())
        .await
        .unwrap();
    assert_eq!(
        wait_for_content(&loser_db, doc_id, &base).await.as_ref(),
        Some(&base),
        "the loser should receive the created document"
    );
    let base_revision = local_revision(&loser_db, doc_id).await.unwrap();

    // The winner edits `title`.
    let won = json!({"title": "winner", "referenceFrequency": 440.0});
    winner.update_document(doc_id, won.clone()).await.unwrap();
    assert_eq!(
        wait_for_content(&winner_db, doc_id, &won).await.as_ref(),
        Some(&won)
    );

    // Simulate the loser never having seen that update: rewind its row to the
    // shared base. Its next edit is therefore built on a stale hash, exactly as
    // a genuinely concurrent edit would be.
    {
        let db = ClientDatabase::new(&loser_db).await.unwrap();
        sqlx::query("UPDATE documents SET content = ?, sync_revision = ? WHERE id = ?")
            .bind(base.to_string())
            .bind(base_revision)
            .bind(doc_id.to_string())
            .execute(&db.pool)
            .await
            .unwrap();
    }

    // The loser edits `referenceFrequency` from the stale base.
    let lost = json!({"title": "base", "referenceFrequency": 441.0});
    loser.update_document(doc_id, lost).await.unwrap();

    let merged = json!({"title": "winner", "referenceFrequency": 441.0});

    assert_eq!(
        wait_for_content(&loser_db, doc_id, &merged).await.as_ref(),
        Some(&merged),
        "the loser must rebase onto the winner's content, not drop its edit"
    );
    assert_eq!(
        wait_for_content(&winner_db, doc_id, &merged).await.as_ref(),
        Some(&merged),
        "the winner must converge on the rebased content"
    );

    let driver = TestClient::connect(TEST_EMAIL).await.unwrap();
    let server_doc = driver.get_document(doc_id).await.unwrap();
    assert_eq!(
        server_doc.get("content").cloned(),
        Some(merged),
        "the server must hold both edits too"
    );

    drop(winner);
    drop(loser);
    std::fs::remove_file(&winner_db_file).ok();
    std::fs::remove_file(&loser_db_file).ok();
}
