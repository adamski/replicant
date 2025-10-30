use chrono::Utc;
use serde_json::Value;
use sync_client::{ClientDatabase, SyncEngine};
use sync_core::models::{Document, VectorClock};
use uuid::Uuid;

/// Creates a new in-memory test sqlite database and runs migrations.
pub async fn setup_test_db() -> ClientDatabase {
    let db_url = "sqlite::memory:";
    let db = ClientDatabase::new(db_url).await.unwrap();
    db.run_migrations().await.unwrap();
    db
}

/// Creates a sample document for a given user.
pub fn make_document(user_id: Uuid, title: &str, text: &str, version: i64) -> Document {
    let content = serde_json::json!({
        "title": title,
        "text": text
    });

    Document {
        id: Uuid::new_v4(),
        user_id,
        content: content.clone(),
        revision_id: if version == 1 {
            Document::initial_revision(&content)
        } else {
            format!("{}-server", version)
        },
        version,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    }
}

/// Helper to get the sync_status for a document.
pub async fn get_sync_status(db: &ClientDatabase, doc_id: Uuid) -> String {
    use sqlx::Row;
    sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
        .bind(doc_id.to_string())
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get::<String, _>(0)
}

/// Creates a test DB with `n` pending documents and returns their IDs.
pub async fn seed_pending_docs(db: &ClientDatabase, user_id: Uuid, n: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for i in 0..n {
        let doc = make_document(user_id, &format!("Doc {}", i + 1), "Pending content", 1);
        db.save_document(&doc).await.unwrap();
        ids.push(doc.id);
    }
    ids
}

/// Marks all documents in `ids` as synced with given revision id suffix.
pub async fn mark_all_synced(db: &ClientDatabase, ids: &[Uuid]) {
    for (i, id) in ids.iter().enumerate() {
        db.mark_synced(id, &format!("2-synced-{}", i))
            .await
            .unwrap();
    }
}
