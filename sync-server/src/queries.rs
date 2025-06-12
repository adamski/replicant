use sqlx::{PgPool, postgres::PgRow, Row};
use uuid::Uuid;
use sync_core::models::Document;

/// SQL queries for server database operations
pub struct Queries;

impl Queries {
    // User queries
    pub const CREATE_USER: &'static str = r#"
        INSERT INTO users (email, auth_token_hash)
        VALUES ($1, $2)
        RETURNING id
    "#;
    
    pub const GET_USER_BY_ID: &'static str = r#"
        SELECT id, email, auth_token_hash, created_at, last_seen_at
        FROM users
        WHERE id = $1
    "#;
    
    pub const VERIFY_AUTH_TOKEN: &'static str = r#"
        SELECT COUNT(*) as count
        FROM users
        WHERE id = $1 AND auth_token_hash = $2
    "#;
    
    pub const UPDATE_LAST_SEEN: &'static str = r#"
        UPDATE users
        SET last_seen_at = NOW()
        WHERE id = $1
    "#;
    
    // Document queries
    pub const CREATE_DOCUMENT: &'static str = r#"
        INSERT INTO documents (
            id, user_id, content, revision_id, version,
            vector_clock, created_at, updated_at, deleted_at, checksum, size_bytes
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
    "#;
    
    pub const GET_DOCUMENT: &'static str = r#"
        SELECT id, user_id, content, revision_id, version,
               vector_clock, created_at, updated_at, deleted_at
        FROM documents
        WHERE id = $1
    "#;
    
    pub const UPDATE_DOCUMENT: &'static str = r#"
        UPDATE documents
        SET content = $2, revision_id = $3, version = $4,
            vector_clock = $5, updated_at = $6, deleted_at = $7,
            checksum = $8, size_bytes = $9
        WHERE id = $1
    "#;
    
    pub const GET_USER_DOCUMENTS: &'static str = r#"
        SELECT id, user_id, content, revision_id, version,
               vector_clock, created_at, updated_at, deleted_at
        FROM documents
        WHERE user_id = $1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
    "#;
    
    pub const GET_DOCUMENTS_BY_IDS: &'static str = r#"
        SELECT id, user_id, content, revision_id, version,
               vector_clock, created_at, updated_at, deleted_at
        FROM documents
        WHERE id = ANY($1) AND user_id = $2
    "#;
    
    // Revision queries
    pub const CREATE_REVISION: &'static str = r#"
        INSERT INTO document_revisions (
            document_id, revision_id, content, patch, version, created_by
        ) VALUES ($1, $2, $3, $4, $5, $6)
    "#;
    
    pub const GET_DOCUMENT_REVISIONS: &'static str = r#"
        SELECT id, document_id, revision_id, content, patch, version,
               created_at, created_by
        FROM document_revisions
        WHERE document_id = $1
        ORDER BY created_at DESC
        LIMIT $2
    "#;
    
    // Connection queries
    pub const ADD_ACTIVE_CONNECTION: &'static str = r#"
        INSERT INTO active_connections (user_id, connection_id)
        VALUES ($1, $2)
        ON CONFLICT (user_id) DO UPDATE
        SET connection_id = $2, last_ping_at = NOW()
    "#;
    
    pub const REMOVE_ACTIVE_CONNECTION: &'static str = r#"
        DELETE FROM active_connections WHERE user_id = $1
    "#;
    
    pub const UPDATE_CONNECTION_PING: &'static str = r#"
        UPDATE active_connections
        SET last_ping_at = NOW()
        WHERE user_id = $1
    "#;
    
    pub const GET_ACTIVE_CONNECTIONS: &'static str = r#"
        SELECT user_id, connection_id, connected_at, last_ping_at
        FROM active_connections
        WHERE last_ping_at > NOW() - INTERVAL '5 minutes'
    "#;
}

/// Helper functions for common database operations
pub struct DbHelpers;

impl DbHelpers {
    /// Parse a document from a database row
    pub fn parse_document(row: &PgRow) -> Result<Document, sqlx::Error> {
        Ok(Document {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            content: row.try_get("content")?,
            revision_id: row.try_get("revision_id")?,
            version: row.try_get("version")?,
            vector_clock: serde_json::from_value(row.try_get("vector_clock")?).unwrap_or_default(),
            created_at: row.try_get::<chrono::DateTime<chrono::Local>, _>("created_at")?.with_timezone(&chrono::Utc),
            updated_at: row.try_get::<chrono::DateTime<chrono::Local>, _>("updated_at")?.with_timezone(&chrono::Utc),
            deleted_at: row.try_get::<Option<chrono::DateTime<chrono::Local>>, _>("deleted_at")?
                .map(|dt| dt.with_timezone(&chrono::Utc)),
        })
    }
    
    /// Prepare document values for database insertion
    pub fn document_to_params(doc: &Document) -> (
        Uuid,                    // id
        Uuid,                    // user_id
        serde_json::Value,       // content
        String,                  // revision_id
        i64,                     // version
        serde_json::Value,       // vector_clock
        chrono::DateTime<chrono::Utc>, // created_at
        chrono::DateTime<chrono::Utc>, // updated_at
        Option<chrono::DateTime<chrono::Utc>>, // deleted_at
        String,                  // checksum
        i32,                     // size_bytes
    ) {
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
    pub async fn get_document_stats(pool: &PgPool, user_id: &Uuid) -> Result<DocumentStats, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT 
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE deleted_at IS NULL) as active,
                COUNT(*) FILTER (WHERE deleted_at IS NOT NULL) as deleted,
                COALESCE(SUM(size_bytes), 0) as total_size
            FROM documents
            WHERE user_id = $1
            "#
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
}

#[derive(Debug, Clone)]
pub struct DocumentStats {
    pub total: u64,
    pub active: u64,
    pub deleted: u64,
    pub total_size_bytes: u64,
}