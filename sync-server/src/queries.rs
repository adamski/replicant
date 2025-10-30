use sqlx::{postgres::PgRow, PgPool, Row};
use sync_core::{models::Document, SyncResult};
use uuid::Uuid;

/// Type alias for document parameters tuple
pub type DocumentParams = (
    Uuid,                                  // id
    Uuid,                                  // user_id
    serde_json::Value,                     // content
    String,                                // revision_id
    i64,                                   // version
    serde_json::Value,                     // vector_clock
    chrono::DateTime<chrono::Utc>,         // created_at
    chrono::DateTime<chrono::Utc>,         // updated_at
    Option<chrono::DateTime<chrono::Utc>>, // deleted_at
    String,                                // checksum
    i32,                                   // size_bytes
);

/// Parse a document from a database row
pub fn parse_document(row: &PgRow) -> SyncResult<Document> {
    Ok(Document {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        content: row.try_get("content")?,
        revision_id: row.try_get("revision_id")?,
        version: row.try_get("version")?,
        vector_clock: serde_json::from_value(row.try_get("vector_clock")?).unwrap_or_default(),
        created_at: row
            .try_get::<chrono::DateTime<chrono::Local>, _>("created_at")?
            .with_timezone(&chrono::Utc),
        updated_at: row
            .try_get::<chrono::DateTime<chrono::Local>, _>("updated_at")?
            .with_timezone(&chrono::Utc),
        deleted_at: row
            .try_get::<Option<chrono::DateTime<chrono::Local>>, _>("deleted_at")?
            .map(|dt| dt.with_timezone(&chrono::Utc)),
    })
}

/// Prepare document values for database insertion
pub fn document_to_params(doc: &Document) -> DocumentParams {
    let content_str = doc.content.to_string();
    let checksum = sync_core::patches::calculate_checksum(&doc.content);
    let size_bytes = content_str.len() as i32;

    (
        doc.id,
        doc.user_id,
        doc.content.clone(),
        doc.revision_id.clone(),
        doc.version,
        serde_json::to_value(&doc.vector_clock).unwrap_or(serde_json::json!({})),
        doc.created_at,
        doc.updated_at,
        doc.deleted_at,
        checksum,
        size_bytes,
    )
}

/// Calculate document statistics
pub async fn get_document_stats(pool: &PgPool, user_id: &Uuid) -> SyncResult<DocumentStats> {
    let row = sqlx::query(
        r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE deleted_at IS NULL) as active,
                COUNT(*) FILTER (WHERE deleted_at IS NOT NULL) as deleted,
                COALESCE(SUM(size_bytes), 0) as total_size
            FROM documents
            WHERE user_id = $1
            "#,
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(DocumentStats {
        total: row.try_get::<i64, _>("total")? as u64,
        active: row.try_get::<i64, _>("active")? as u64,
        deleted: row.try_get::<i64, _>("deleted")? as u64,
        total_size_bytes: row.try_get::<i64, _>("total_size")? as u64,
    })
}

#[derive(Debug, Clone)]
pub struct DocumentStats {
    pub total: u64,
    pub active: u64,
    pub deleted: u64,
    pub total_size_bytes: u64,
}
