use sqlx::{SqlitePool, sqlite::SqlitePoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
use crate::errors::ClientError;
use crate::queries::{Queries, DbHelpers};
use json_patch;

pub struct ClientDatabase {
    pub pool: SqlitePool,
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
        let row = sqlx::query(Queries::GET_USER_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.try_get("user_id")?;
        Ok(Uuid::parse_str(&user_id)?)
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, ClientError> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        
        DbHelpers::parse_document(&row)
    }
    
    pub async fn save_document(&self, doc: &Document) -> Result<(), ClientError> {
        let params = DbHelpers::document_to_params(doc)?;
        
        sqlx::query(Queries::UPSERT_DOCUMENT)
            .bind(params.0)  // id
            .bind(params.1)  // user_id
            .bind(params.2)  // title
            .bind(params.3)  // content
            .bind(params.4)  // revision_id
            .bind(params.5)  // version
            .bind(params.6)  // vector_clock
            .bind(params.7)  // created_at
            .bind(params.8)  // updated_at
            .bind(params.9)  // deleted_at
            .bind(params.10) // sync_status
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn get_pending_documents(&self) -> Result<Vec<Uuid>, ClientError> {
        let rows = sqlx::query(Queries::GET_PENDING_DOCUMENTS)
            .fetch_all(&self.pool)
            .await?;
        
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                Ok(Uuid::parse_str(&id)?)
            })
            .collect()
    }
    
    pub async fn mark_synced(&self, document_id: &Uuid, revision_id: &str) -> Result<(), ClientError> {
        sqlx::query(Queries::MARK_DOCUMENT_SYNCED)
            .bind(document_id.to_string())
            .bind(revision_id)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn delete_document(&self, document_id: &Uuid) -> Result<(), ClientError> {
        sqlx::query("UPDATE documents SET deleted_at = datetime('now'), sync_status = 'pending' WHERE id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn get_all_documents(&self) -> Result<Vec<Document>, ClientError> {
        let rows = sqlx::query("SELECT * FROM documents WHERE deleted_at IS NULL")
            .fetch_all(&self.pool)
            .await?;
        
        rows.into_iter()
            .map(|row| DbHelpers::parse_document(&row))
            .collect()
    }
    
    pub async fn queue_sync_operation(
        &self,
        document_id: &Uuid,
        operation_type: &str,
        patch: Option<&json_patch::Patch>,
    ) -> Result<(), ClientError> {
        let patch_json = patch.map(|p| serde_json::to_string(p)).transpose()?;
        
        sqlx::query(Queries::INSERT_SYNC_QUEUE)
            .bind(document_id.to_string())
            .bind(operation_type)
            .bind(patch_json)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
}