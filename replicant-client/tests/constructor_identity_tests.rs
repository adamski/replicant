//! Constructor-time identity adoption: the canonical user id (from stored
//! credentials) is adopted during `Client` construction, before any WebSocket
//! work and before the handle is returned — so adoption can never race live
//! document creation.

use replicant_client::Client;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Unreachable server: connect fails fast, keeping tests offline. Only used
/// by tests that don't need to prove "zero connection attempts" (a dead
/// address fails identically whether or not the sync gate ran).
const DEAD_SERVER: &str = "http://127.0.0.1:1";

fn temp_db_url() -> String {
    let path = std::env::temp_dir().join(format!("replicant-ctor-test-{}.sqlite3", Uuid::new_v4()));
    format!("sqlite://{}?mode=rwc", path.display())
}

/// Binds a local listener and counts accepted connections, so tests can
/// assert "no connection attempt was made" instead of relying on a dead
/// address (which fails identically whether or not the gate ran). Returns
/// the reachable server URL and the shared counter.
async fn counting_listener() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local listener");
    let addr = listener.local_addr().expect("local addr");
    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = Arc::clone(&count);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                    drop(socket);
                }
                Err(_) => break,
            }
        }
    });
    (format!("http://{}", addr), count)
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

/// Empty credentials must yield a fully usable local-only client: no
/// connection attempt against the sync server, and full CRUD over the local
/// DB. The 10s timeout is a suite-hang safety net only — it does not prove
/// the client dropped cleanly (there is no `Drop` impl on `Client` and any
/// spawned task is detached, so a hung reconnect task would not fail this
/// wrapper). What actually falsifies a regressed sync gate is the counting
/// listener below: if `sync_enabled` were wrongly true, the constructor's own
/// connect attempt would register on the counter within milliseconds.
#[tokio::test]
async fn open_without_credentials_is_local_only_and_usable() {
    let (server_url, connections) = counting_listener().await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let db_url = temp_db_url();
        let client = Client::new(&db_url, &server_url, "offline@test.com", "", "", None)
            .await
            .unwrap();

        assert!(!client.is_connected(), "no credentials must mean offline");

        let doc = client
            .create_document(serde_json::json!({"title": "offline-only"}))
            .await
            .unwrap();

        let docs = client.get_all_documents().await.unwrap();
        assert!(docs.iter().any(|d| d.id == doc.id));

        client
            .update_document(doc.id, serde_json::json!({"title": "offline-only-edited"}))
            .await
            .unwrap();
        let docs = client.get_all_documents().await.unwrap();
        let updated = docs.iter().find(|d| d.id == doc.id).unwrap();
        assert_eq!(updated.content["title"], "offline-only-edited");

        client.delete_document(doc.id).await.unwrap();
        let docs = client.get_all_documents().await.unwrap();
        assert!(!docs.iter().any(|d| d.id == doc.id));

        assert!(
            !client.is_connected(),
            "still offline after local CRUD activity"
        );

        drop(client);
    })
    .await;

    assert!(
        result.is_ok(),
        "test did not complete within the 10s suite-hang safety timeout"
    );

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "no connection attempt must be made against the sync server without credentials"
    );
}

/// Reviewer-flagged gap (Task-3 decision row 5): credentials are present but
/// the identity has never been adopted (canonical_user_id = None, never
/// adopted before). Sync must stay disabled — no connection attempt at all —
/// even though api_key/api_secret are non-empty. Uses a real local listener
/// (not the dead-server address) so a wrongly-attempted connection would
/// actually register instead of merely failing to connect either way.
#[tokio::test]
async fn open_with_credentials_but_unadopted_identity_stays_local() {
    let (server_url, connections) = counting_listener().await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let db_url = temp_db_url();
        let client = Client::new(
            &db_url,
            &server_url,
            "unadopted@test.com",
            "some-api-key",
            "some-api-secret",
            None,
        )
        .await
        .unwrap();

        assert!(
            !client.is_connected(),
            "unadopted identity must not connect even with credentials present"
        );

        let doc = client
            .create_document(serde_json::json!({"title": "unadopted-but-local"}))
            .await
            .unwrap();
        let docs = client.get_all_documents().await.unwrap();
        assert!(docs.iter().any(|d| d.id == doc.id));

        // Give a hypothetical reconnect loop time to spin and misbehave; it
        // must not, since sync_enabled requires an adopted identity.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(
            !client.is_connected(),
            "reconnection loop must not have connected/spammed while identity is unadopted"
        );

        drop(client);
    })
    .await;

    assert!(
        result.is_ok(),
        "test did not complete within the 10s suite-hang safety timeout"
    );

    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "no connection attempt must be made against the sync server while identity is unadopted"
    );
}
