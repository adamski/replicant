use sqlx::{SqlitePool, sqlite::SqlitePoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
use crate::errors::ClientError;
use crate::queries::{Queries, DbHelpers};
use json_patch;
use crate::ClientResult;

pub struct ClientDatabase {
    pub pool: SqlitePool,
}

impl ClientDatabase {
    pub async fn new(database_url: &str) -> ClientResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn run_migrations(&self) -> ClientResult<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
    
    pub async fn ensure_user_config(&self, server_url: &str, auth_token: &str) -> ClientResult<()> {
        // Check if user_config already exists
        let exists = sqlx::query("SELECT COUNT(*) as count FROM user_config")
            .fetch_one(&self.pool)
            .await?;
        
        let count: i64 = exists.try_get("count")?;
        
        if count == 0 {
            // No user config exists, create default
            let user_id = Uuid::new_v4();
            let client_id = Uuid::new_v4();
            
            sqlx::query("INSERT INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)")
                .bind(user_id.to_string())
                .bind(client_id.to_string())
                .bind(server_url)
                .bind(auth_token)
                .execute(&self.pool)
                .await?;
        }
        
        Ok(())
    }
    
    pub async fn get_user_id(&self) -> ClientResult<Uuid> {
        let row = sqlx::query(Queries::GET_USER_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.try_get("user_id")?;
        Ok(Uuid::parse_str(&user_id)?)
    }
    
    pub async fn get_client_id(&self) -> ClientResult<Uuid> {
        let row = sqlx::query(Queries::GET_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let client_id: String = row.try_get("client_id")?;
        Ok(Uuid::parse_str(&client_id)?)
    }
    
    pub async fn get_user_and_client_id(&self) -> ClientResult<(Uuid, Uuid)> {
        let row = sqlx::query(Queries::GET_USER_AND_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.try_get("user_id")?;
        let client_id: String = row.try_get("client_id")?;
        Ok((Uuid::parse_str(&user_id)?, Uuid::parse_str(&client_id)?))
    }
    
    pub async fn get_document(&self, id: &Uuid) -> ClientResult<Document> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        
        DbHelpers::parse_document(&row)
    }
    
    pub async fn save_document(&self, doc: &Document) -> ClientResult<()> {
        self.save_document_with_status(doc, None).await
    }
    
    pub(crate) async fn save_document_with_status(&self, doc: &Document, sync_status: Option<&str>) -> ClientResult<()> {
        let params = DbHelpers::document_to_params(doc, sync_status)?;
        
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
    
    pub async fn get_pending_documents(&self) -> ClientResult<Vec<Uuid>> {
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
    
    pub async fn mark_synced(&self, document_id: &Uuid, revision_id: &str) -> ClientResult<()> {
        sqlx::query(Queries::MARK_DOCUMENT_SYNCED)
            .bind(document_id.to_string())
            .bind(revision_id)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn delete_document(&self, document_id: &Uuid) -> ClientResult<()> {
        sqlx::query("UPDATE documents SET deleted_at = datetime('now'), sync_status = 'pending' WHERE id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn get_all_documents(&self) -> ClientResult<Vec<Document>> {
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
    ) -> ClientResult<()> {
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