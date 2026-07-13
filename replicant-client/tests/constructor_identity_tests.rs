//! Constructor-time identity adoption: the canonical user id (from stored
//! credentials) is adopted during `Client` construction, before any WebSocket
//! work and before the handle is returned — so adoption can never race live
//! document creation.

use replicant_client::Client;
use uuid::Uuid;

/// Unreachable server: connect fails fast, keeping tests offline.
const DEAD_SERVER: &str = "http://127.0.0.1:1";

fn temp_db_url() -> String {
    let path = std::env::temp_dir().join(format!("replicant-ctor-test-{}.sqlite3", Uuid::new_v4()));
    format!("sqlite://{}?mode=rwc", path.display())
}

async fn open_offline(db_url: &str, canonical: Option<Uuid>) -> replicant_core::SyncResult<Client> {
    Client::new(db_url, DEAD_SERVER, "ctor@test.com", "", "", canonical).await
}

#[tokio::test]
async fn constructor_adopts_canonical_id_before_returning() {
    let db_url = temp_db_url();

    // First run: offline, no canonical id → provisional identity.
    let client = open_offline(&db_url, None).await.unwrap();
    let provisional = client.user_id();
    let doc = client
        .create_document(serde_json::json!({"title": "offline tuning"}))
        .await
        .unwrap();
    assert_eq!(doc.user_id, Some(provisional));
    drop(client);

    // Second run: credentials arrived with a canonical id → constructor adopts.
    let canonical = Uuid::new_v4();
    let client = open_offline(&db_url, Some(canonical)).await.unwrap();
    assert_eq!(client.user_id(), canonical);

    let docs = client.get_all_documents().await.unwrap();
    let restamped = docs.iter().find(|d| d.id == doc.id).unwrap();
    assert_eq!(
        restamped.user_id,
        Some(canonical),
        "offline-created document must be re-stamped to the canonical id"
    );
}

#[tokio::test]
async fn constructor_rejects_account_switch() {
    let db_url = temp_db_url();
    let canonical = Uuid::new_v4();

    let client = open_offline(&db_url, Some(canonical)).await.unwrap();
    assert_eq!(client.user_id(), canonical);
    drop(client);

    // A different canonical id on an already-adopted install is an account
    // switch — refuse rather than re-stamp another account's documents.
    let other_account = Uuid::new_v4();
    let result = open_offline(&db_url, Some(other_account)).await;
    assert!(result.is_err(), "account switch must be rejected");
}

#[tokio::test]
async fn constructor_same_id_after_adoption_is_noop() {
    let db_url = temp_db_url();
    let canonical = Uuid::new_v4();

    let client = open_offline(&db_url, Some(canonical)).await.unwrap();
    drop(client);

    // Same canonical id again (e.g. credential rotation): proceed normally.
    let client = open_offline(&db_url, Some(canonical)).await.unwrap();
    assert_eq!(client.user_id(), canonical);
}
