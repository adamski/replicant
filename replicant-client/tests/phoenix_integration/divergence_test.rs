//! Live coverage for the guarded broadcast apply and targeted resync
//! (DEV-1037). A raw `TestClient` drives the server; a real `Client` is the
//! subject under test, so these exercise the production message handler,
//! websocket `get_document`, and local SQLite state together.
//!
//! Gated behind `RUN_INTEGRATION_TESTS`; run via
//! `test/run_phoenix_interop_local.sh`.

use super::{
    serial, server_url, skip_if_no_server, test_api_key, test_api_secret, TestClient, TEST_EMAIL,
};
use replicant_client::{Client, ClientDatabase};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

fn canonical_user_id() -> Uuid {
    std::env::var("REPLICANT_TEST_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
        .expect("REPLICANT_TEST_USER_ID is required for divergence tests")
}

fn temp_db_path(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "databases/divergence_{}_{}_{}.sqlite3",
        tag,
        std::process::id(),
        nanos
    )
}

async fn connect_subject(db_url: &str) -> Client {
    let client = Client::with_event_dispatcher(
        db_url,
        &server_url(),
        TEST_EMAIL,
        &test_api_key(),
        &test_api_secret(),
        Some(canonical_user_id()),
        None,
    )
    .await
    .expect("subject client should connect");

    for _ in 0..50 {
        if client.is_connected() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(client.is_connected(), "subject client should be connected");
    client
}

/// Poll the subject's local database until it reaches the expected revision.
async fn wait_for_revision(db_url: &str, id: Uuid, revision: i64) -> Option<(Value, i64)> {
    let db = ClientDatabase::new(db_url).await.unwrap();
    for _ in 0..100 {
        if let Ok(doc) = db.get_document(&id).await {
            if doc.sync_revision == revision {
                return Some((doc.content, doc.sync_revision));
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

fn server_content(reply: &Value) -> Value {
    reply.get("content").cloned().expect("reply has content")
}

fn server_revision(reply: &Value) -> i64 {
    reply
        .get("sync_revision")
        .and_then(|v| v.as_i64())
        .expect("reply has sync_revision")
}

fn server_hash(reply: &Value) -> String {
    reply
        .get("content_hash")
        .and_then(|v| v.as_str())
        .expect("reply has content_hash")
        .to_string()
}

#[tokio::test]
#[serial]
async fn contiguous_broadcast_is_applied_and_verified_against_the_server_hash() {
    if skip_if_no_server() {
        return;
    }

    std::fs::create_dir_all("databases").ok();
    let db_file = temp_db_path("apply");
    let db_url = format!("sqlite:{}?mode=rwc", db_file);
    let subject = connect_subject(&db_url).await;

    let driver = TestClient::connect(TEST_EMAIL).await.unwrap();
    let doc_id = Uuid::new_v4();
    let created = driver
        .create_document_with_id(doc_id, json!({"title": "base", "n": 1}))
        .await
        .unwrap();

    assert!(
        wait_for_revision(&db_url, doc_id, server_revision(&created))
            .await
            .is_some(),
        "subject should receive the created document"
    );

    // One contiguous update: the guard must apply it, not resync.
    driver
        .update_document(
            doc_id,
            json!([{"op": "replace", "path": "/n", "value": 2}]),
            &server_hash(&created),
        )
        .await
        .unwrap();

    let after = driver.get_document(doc_id).await.unwrap();
    let (content, revision) = wait_for_revision(&db_url, doc_id, server_revision(&after))
        .await
        .expect("subject should converge on the updated revision");

    assert_eq!(content, server_content(&after), "content must be identical");
    assert_eq!(revision, server_revision(&after));

    drop(subject);
    std::fs::remove_file(&db_file).ok();
}

#[tokio::test]
#[serial]
async fn missed_broadcast_resyncs_instead_of_frankenpatching() {
    if skip_if_no_server() {
        return;
    }

    std::fs::create_dir_all("databases").ok();
    let db_file = temp_db_path("resync");
    let db_url = format!("sqlite:{}?mode=rwc", db_file);
    let subject = connect_subject(&db_url).await;

    let driver = TestClient::connect(TEST_EMAIL).await.unwrap();
    let doc_id = Uuid::new_v4();
    driver
        .create_document_with_id(doc_id, json!({"title": "base", "n": 1}))
        .await
        .unwrap();
    // The create reply omits `content`; read the full document back so the
    // rewind below can restore the exact pre-u1 state.
    let created = driver.get_document(doc_id).await.unwrap();
    assert!(
        wait_for_revision(&db_url, doc_id, server_revision(&created))
            .await
            .is_some(),
        "subject should receive the created document"
    );

    // u1: the subject applies this normally.
    driver
        .update_document(
            doc_id,
            json!([{"op": "replace", "path": "/n", "value": 2}]),
            &server_hash(&created),
        )
        .await
        .unwrap();
    let after_u1 = driver.get_document(doc_id).await.unwrap();
    assert!(
        wait_for_revision(&db_url, doc_id, server_revision(&after_u1))
            .await
            .is_some(),
        "subject should apply u1"
    );

    // Simulate u1's broadcast never arriving: rewind the subject's local row to
    // the pre-u1 content and revision. u2's broadcast will then be one ahead of
    // where the subject thinks it is.
    {
        let db = ClientDatabase::new(&db_url).await.unwrap();
        sqlx::query("UPDATE documents SET content = ?, sync_revision = ? WHERE id = ?")
            .bind(server_content(&created).to_string())
            .bind(server_revision(&created))
            .bind(doc_id.to_string())
            .execute(&db.pool)
            .await
            .unwrap();
    }

    // u2: arrives with a revision gap, so the subject must resync.
    driver
        .update_document(
            doc_id,
            json!([{"op": "replace", "path": "/title", "value": "after u2"}]),
            &server_hash(&after_u1),
        )
        .await
        .unwrap();

    let after_u2 = driver.get_document(doc_id).await.unwrap();
    let (content, revision) = wait_for_revision(&db_url, doc_id, server_revision(&after_u2))
        .await
        .expect("subject should resync to the server revision");

    assert_eq!(
        content,
        server_content(&after_u2),
        "resync must produce content identical to the server, not a frankenpatch"
    );
    assert_eq!(revision, server_revision(&after_u2));

    drop(subject);
    std::fs::remove_file(&db_file).ok();
}
