use sqlx::{SqlitePool, Row, sqlite::SqliteRow};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use sync_core::models::Document;
use crate::errors::ClientError;

/// SQL queries for client database operations
pub struct Queries;

impl Queries {
    /// Create the client database schema
    pub const SCHEMA: &'static str = r#"
        CREATE TABLE IF NOT EXISTS user_config (
            user_id TEXT PRIMARY KEY,
            server_url TEXT NOT NULL,
            last_sync_at TIMESTAMP,
            auth_token TEXT
        );
        
        CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            title TEXT NOT NULL,
            content JSON NOT NULL,
            revision_id TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            vector_clock JSON,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            deleted_at TIMESTAMP,
            local_changes JSON,
            sync_status TEXT DEFAULT 'pending',
            last_synced_revision TEXT,
            CHECK (sync_status IN ('synced', 'pending', 'conflict'))
        );
        
        CREATE TABLE IF NOT EXISTS sync_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            document_id TEXT NOT NULL,
            operation_type TEXT NOT NULL,
            patch JSON,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            retry_count INTEGER DEFAULT 0,
            FOREIGN KEY (document_id) REFERENCES documents(id),
            CHECK (operation_type IN ('create', 'update', 'delete'))
        );
        
        CREATE INDEX IF NOT EXISTS idx_documents_user_id ON documents(user_id);
        CREATE INDEX IF NOT EXISTS idx_documents_sync_status ON documents(sync_status);
        CREATE INDEX IF NOT EXISTS idx_sync_queue_created_at ON sync_queue(created_at);
    "#;

    // User config queries
    pub const GET_USER_ID: &'static str = "SELECT user_id FROM user_config LIMIT 1";
    
    pub const INSERT_USER_CONFIG: &'static str = 
        "INSERT INTO user_config (user_id, server_url, auth_token) VALUES (?1, ?2, ?3)";
    
    pub const UPDATE_LAST_SYNC: &'static str = 
        "UPDATE user_config SET last_sync_at = ?1 WHERE user_id = ?2";

    // Document queries
    pub const GET_DOCUMENT: &'static str = r#"
        SELECT id, user_id, title, content, revision_id, version,
               vector_clock, created_at, updated_at, deleted_at
        FROM documents
        WHERE id = ?1
    "#;
    
    pub const UPSERT_DOCUMENT: &'static str = r#"
        INSERT INTO documents (
            id, user_id, title, content, revision_id, version,
            vector_clock, created_at, updated_at, deleted_at, sync_status
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            content = excluded.content,
            revision_id = excluded.revision_id,
            version = excluded.version,
            vector_clock = excluded.vector_clock,
            updated_at = excluded.updated_at,
            deleted_at = excluded.deleted_at,
            sync_status = excluded.sync_status
    "#;
    
    pub const LIST_USER_DOCUMENTS: &'static str = r#"
        SELECT id, title, sync_status, updated_at 
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
    "#;
    
    pub const GET_PENDING_DOCUMENTS: &'static str = r#"
        SELECT id FROM documents
        WHERE sync_status = 'pending'
        ORDER BY updated_at ASC
    "#;
    
    pub const MARK_DOCUMENT_SYNCED: &'static str = r#"
        UPDATE documents
        SET sync_status = 'synced',
            last_synced_revision = ?2
        WHERE id = ?1
    "#;
    
    pub const UPDATE_SYNC_STATUS: &'static str = 
        "UPDATE documents SET sync_status = ?2 WHERE id = ?1";
    
    pub const COUNT_BY_SYNC_STATUS: &'static str = 
        "SELECT COUNT(*) as count FROM documents WHERE sync_status = ?1";

    // Sync queue queries
    pub const INSERT_SYNC_QUEUE: &'static str = r#"
        INSERT INTO sync_queue (document_id, operation_type, patch)
        VALUES (?1, ?2, ?3)
    "#;
    
    pub const GET_SYNC_QUEUE: &'static str = r#"
        SELECT id, document_id, operation_type, patch, retry_count
        FROM sync_queue
        ORDER BY created_at ASC
        LIMIT 100
    "#;
    
    pub const DELETE_FROM_QUEUE: &'static str = 
        "DELETE FROM sync_queue WHERE id = ?1";
    
    pub const INCREMENT_RETRY_COUNT: &'static str = 
        "UPDATE sync_queue SET retry_count = retry_count + 1 WHERE id = ?1";
}

/// Helper functions for common database operations
pub struct DbHelpers;

impl DbHelpers {
    /// Initialize the database schema
    pub async fn init_schema(pool: &SqlitePool) -> Result<(), ClientError> {
        sqlx::query(Queries::SCHEMA)
            .execute(pool)
            .await?;
        Ok(())
    }
    
    /// Parse a document from a database row
    pub fn parse_document(row: &SqliteRow) -> Result<Document, ClientError> {
        let id: String = row.get("id");
        let user_id: String = row.get("user_id");
        let title: String = row.get("title");
        let content: String = row.get("content");
        let revision_id: String = row.get("revision_id");
        let version: i64 = row.get("version");
        let vector_clock: Option<String> = row.get("vector_clock");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");
        let deleted_at: Option<String> = row.get("deleted_at");
        
        Ok(Document {
            id: Uuid::parse_str(&id)?,
            user_id: Uuid::parse_str(&user_id)?,
            title,
            content: serde_json::from_str(&content)?,
            revision_id: Uuid::parse_str(&revision_id)?,
            version,
            vector_clock: serde_json::from_str(&vector_clock.unwrap_or_else(|| "{}".to_string()))?,
            created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
            deleted_at: deleted_at.map(|dt| DateTime::parse_from_rfc3339(&dt).ok())
                .flatten()
                .map(|dt| dt.with_timezone(&Utc)),
        })
    }
    
    /// Prepare document values for database insertion
    pub fn document_to_params(doc: &Document) -> Result<(
        String, String, String, String, String, i64, String, String, String, Option<String>, String
    ), ClientError> {
        Ok((
            doc.id.to_string(),
            doc.user_id.to_string(),
            doc.title.clone(),
            serde_json::to_string(&doc.content)?,
            doc.revision_id.to_string(),
            doc.version,
            serde_json::to_string(&doc.vector_clock)?,
            doc.created_at.to_rfc3339(),
            doc.updated_at.to_rfc3339(),
            doc.deleted_at.map(|dt| dt.to_rfc3339()),
            "pending".to_string(),
        ))
    }
    
    /// Get count of documents by sync status
    pub async fn count_by_status(
        pool: &SqlitePool,
        status: &str,
    ) -> Result<i64, ClientError> {
        let row = sqlx::query(Queries::COUNT_BY_SYNC_STATUS)
            .bind(status)
            .fetch_one(pool)
            .await?;
        
        Ok(row.try_get("count")?)
    }
}