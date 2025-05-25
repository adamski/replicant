use sqlx::{SqlitePool, sqlite::SqlitePoolOptions, Row};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use sync_core::models::Document;
use crate::errors::ClientError;
use json_patch;

pub struct ClientDatabase {
    pub(crate) pool: SqlitePool,
}

impl ClientDatabase {
    pub async fn new(database_url: &str) -> Result<Self, ClientError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn run_migrations(&self) -> Result<(), ClientError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
    
    pub async fn get_user_id(&self) -> Result<Uuid, ClientError> {
        let row = sqlx::query("SELECT user_id FROM user_config LIMIT 1")
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.get("user_id");
        Ok(Uuid::parse_str(&user_id)?)
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, ClientError> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, title, content, revision_id, version,
                   vector_clock, created_at, updated_at, deleted_at
            FROM documents
            WHERE id = ?1
            "#,
        )
        .bind(id.to_string())
        .fetch_one(&self.pool)
        .await?;
        
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
    
    pub async fn save_document(&self, doc: &Document) -> Result<(), ClientError> {
        let content_json = serde_json::to_string(&doc.content)?;
        let vector_clock_json = serde_json::to_string(&doc.vector_clock)?;
        
        sqlx::query(
            r#"
            INSERT INTO documents (
                id, user_id, title, content, revision_id, version,
                vector_clock, created_at, updated_at, deleted_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                content = excluded.content,
                revision_id = excluded.revision_id,
                version = excluded.version,
                vector_clock = excluded.vector_clock,
                updated_at = excluded.updated_at,
                deleted_at = excluded.deleted_at
            "#,
        )
        .bind(doc.id.to_string())
        .bind(doc.user_id.to_string())
        .bind(&doc.title)
        .bind(content_json)
        .bind(doc.revision_id.to_string())
        .bind(doc.version as i64)
        .bind(vector_clock_json)
        .bind(doc.created_at.to_rfc3339())
        .bind(doc.updated_at.to_rfc3339())
        .bind(doc.deleted_at.map(|dt| dt.to_rfc3339()))
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn get_pending_documents(&self) -> Result<Vec<Uuid>, ClientError> {
        let rows = sqlx::query(
            r#"
            SELECT id FROM documents
            WHERE sync_status = 'pending'
            ORDER BY updated_at ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        
        rows.into_iter()
            .map(|row| {
                let id: String = row.get("id");
                Uuid::parse_str(&id)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    
    pub async fn mark_synced(&self, document_id: &Uuid, revision_id: &Uuid) -> Result<(), ClientError> {
        sqlx::query(
            r#"
            UPDATE documents
            SET sync_status = 'synced',
                last_synced_revision = ?2
            WHERE id = ?1
            "#,
        )
        .bind(document_id.to_string())
        .bind(revision_id.to_string())
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn queue_sync_operation(
        &self,
        document_id: &Uuid,
        operation_type: &str,
        patch: Option<&json_patch::Patch>,
    ) -> Result<(), ClientError> {
        let patch_json = patch.map(|p| serde_json::to_string(p)).transpose()?;
        
        sqlx::query(
            r#"
            INSERT INTO sync_queue (document_id, operation_type, patch)
            VALUES (?1, ?2, ?3)
            "#,
        )
        .bind(document_id.to_string())
        .bind(operation_type)
        .bind(patch_json)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
}