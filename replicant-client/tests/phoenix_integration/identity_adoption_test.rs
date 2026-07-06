//! End-to-end identity adoption against a live server.
//!
//! Pre-seeds a provisional (random v4) identity plus a document owned by it,
//! then constructs the real `Client` pointed at the deployed server. Connecting
//! runs the join-reply → adoption path, which must re-stamp the document to the
//! server's canonical id, flip `identity_adopted`, and update the in-memory id.
//!
//! Gated behind `RUN_INTEGRATION_TESTS` (see `skip_if_no_server`); needs
//! `SYNC_SERVER_URL`, `REPLICANT_API_KEY`, `REPLICANT_API_SECRET`.

use super::{serial, server_url, skip_if_no_server, test_api_key, test_api_secret, TEST_EMAIL};
use replicant_client::{Client, ClientDatabase};
use sqlx::Row;
use uuid::Uuid;

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
async fn adoption_restamps_document_and_flips_flag_against_live_server() {
    if skip_if_no_server() {
        eprintln!("skipping identity adoption test: RUN_INTEGRATION_TESTS not set");
        return;
    }

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

        sqlx::query("INSERT INTO documents (id, user_id, content) VALUES (?1, ?2, ?3)")
            .bind(doc_id.to_string())
            .bind(provisional.to_string())
            .bind("{}")
            .execute(&db.pool)
            .await
            .unwrap();
    }

    // 2. Construct the real client on the same db, pointed at the live server.
    //    Adoption runs synchronously inside construction, on first join reply.
    let client = Client::with_event_dispatcher(
        &db_url,
        &server_url(),
        TEST_EMAIL,
        &test_api_key(),
        &test_api_secret(),
        None,
    )
    .await
    .expect("client should connect to the live server");

    // 3. The in-memory id switched to the server's canonical id.
    let canonical = client.user_id();
    assert_ne!(
        canonical, provisional,
        "client should have adopted the server's canonical id"
    );

    // 4. The DB reflects the adoption and the pre-existing doc was re-stamped.
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
