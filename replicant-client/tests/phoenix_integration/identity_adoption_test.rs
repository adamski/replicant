//! End-to-end claim-time identity adoption against a live server.
//!
//! Pre-seeds a provisional (random v4) identity plus a document owned by it,
//! then constructs the real `Client` with the canonical user id (as delivered
//! by enrollment claim; in CI it is seeded and exported alongside the API
//! credentials). Construction must adopt BEFORE connecting: re-stamp the
//! document, flip `identity_adopted`, switch the in-memory id — and then join
//! `sync:user:<canonical>` successfully, passing the join-reply drift check.
//!
//! Gated behind `RUN_INTEGRATION_TESTS` (see `skip_if_no_server`); needs
//! `SYNC_SERVER_URL`, `REPLICANT_API_KEY`, `REPLICANT_API_SECRET`, and
//! `REPLICANT_TEST_USER_ID` (the canonical id bound to those credentials).

use super::{serial, server_url, skip_if_no_server, test_api_key, test_api_secret, TEST_EMAIL};
use replicant_client::{Client, ClientDatabase};
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

fn canonical_user_id_from_env() -> Option<Uuid> {
    std::env::var("REPLICANT_TEST_USER_ID")
        .ok()
        .and_then(|s| Uuid::parse_str(&s).ok())
}

fn temp_db_path() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "databases/identity_adopt_{}_{}.sqlite3",
        std::process::id(),
        nanos
    )
}

#[tokio::test]
#[serial]
async fn claim_time_adoption_restamps_and_connects_to_canonical_topic() {
    if skip_if_no_server() {
        eprintln!("skipping identity adoption test: RUN_INTEGRATION_TESTS not set");
        return;
    }
    let Some(canonical) = canonical_user_id_from_env() else {
        panic!("RUN_INTEGRATION_TESTS is set but REPLICANT_TEST_USER_ID is missing/invalid");
    };

    std::fs::create_dir_all("databases").ok();
    let db_file = temp_db_path();
    let db_url = format!("sqlite:{}?mode=rwc", db_file);

    // 1. Pre-seed the offline state: a provisional random id + a document under it.
    let provisional;
    let doc_id = Uuid::new_v4();
    {
        let db = ClientDatabase::new(&db_url).await.unwrap();
        db.run_migrations().await.unwrap();
        db.ensure_user_config(&server_url()).await.unwrap();

        provisional = db.get_user_id().await.unwrap();
        assert_eq!(
            provisional.get_version(),
            Some(uuid::Version::Random),
            "seeded id should be a random v4 provisional id"
        );
        assert_ne!(provisional, canonical);

        sqlx::query("INSERT INTO documents (id, user_id, content) VALUES (?1, ?2, ?3)")
            .bind(doc_id.to_string())
            .bind(provisional.to_string())
            .bind("{}")
            .execute(&db.pool)
            .await
            .unwrap();
    }

    // 2. Construct the real client with the canonical id (claim-time adoption).
    //    Adoption runs inside construction, before any WebSocket work.
    let client = Client::with_event_dispatcher(
        &db_url,
        &server_url(),
        TEST_EMAIL,
        &test_api_key(),
        &test_api_secret(),
        Some(canonical),
        None,
    )
    .await
    .expect("client should adopt and connect to the live server");

    // 3. The in-memory id switched to the canonical id.
    assert_eq!(client.user_id(), canonical);

    // 4. The join to sync:user:<canonical> succeeded (drift check passed).
    let mut connected = client.is_connected();
    for _ in 0..50 {
        if connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        connected = client.is_connected();
    }
    assert!(connected, "client should join sync:user:<canonical>");

    // 5. The DB reflects the adoption and the pre-existing doc was re-stamped.
    let db = ClientDatabase::new(&db_url).await.unwrap();
    let adopted: i64 = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .try_get("identity_adopted")
        .unwrap();
    assert_eq!(adopted, 1, "identity_adopted should be flipped");
    assert_eq!(db.get_user_id().await.unwrap(), canonical);

    let doc_owner: String = sqlx::query("SELECT user_id FROM documents WHERE id = ?1")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .try_get("user_id")
        .unwrap();
    assert_eq!(
        doc_owner,
        canonical.to_string(),
        "the pre-existing document should be re-stamped to the canonical id"
    );

    drop(client);
    std::fs::remove_file(&db_file).ok();
}
