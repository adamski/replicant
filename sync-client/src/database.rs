use sqlx::{SqlitePool, sqlite::SqlitePoolOptions, Row};
use uuid::Uuid;
use sync_core::models::{Document, SyncStatus};
use sync_core::protocol::ChangeEventType;
use crate::errors::ClientError;
use crate::queries::{Queries, DbHelpers};
use json_patch;

#[derive(Debug, Clone)]
pub struct PendingDocumentInfo {
    pub id: Uuid,
    pub last_synced_revision: Option<String>,
    pub is_deleted: bool,
}

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
    
    pub async fn ensure_user_config(&self, server_url: &str, auth_token: &str) -> Result<(), ClientError> {
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
    
    pub async fn ensure_user_config_with_identifier(&self, server_url: &str, auth_token: &str, user_identifier: &str) -> Result<(), ClientError> {
        // Check if user_config already exists
        let exists = sqlx::query("SELECT COUNT(*) as count FROM user_config")
            .fetch_one(&self.pool)
            .await?;
        
        let count: i64 = exists.try_get("count")?;
        
        if count == 0 {
            // No user config exists, create with deterministic user ID
            let user_id = Self::generate_deterministic_user_id(user_identifier);
            let client_id = Uuid::new_v4(); // Client ID should always be unique per instance
            
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
    
    fn generate_deterministic_user_id(user_identifier: &str) -> Uuid {
        // Generate deterministic user ID using UUID v5
        // Use the same logic as the task list example
        const APP_ID: &str = "com.example.sync-task-list";
        
        // Create a two-level namespace hierarchy:
        // 1. DNS namespace -> Application namespace (using APP_ID)
        // 2. Application namespace -> User ID (using user identifier)
        let app_namespace = Uuid::new_v5(&Uuid::NAMESPACE_DNS, APP_ID.as_bytes());
        Uuid::new_v5(&app_namespace, user_identifier.as_bytes())
    }
    
    pub async fn get_user_id(&self) -> Result<Uuid, ClientError> {
        let row = sqlx::query(Queries::GET_USER_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.try_get("user_id")?;
        Ok(Uuid::parse_str(&user_id)?)
    }
    
    pub async fn get_client_id(&self) -> Result<Uuid, ClientError> {
        let row = sqlx::query(Queries::GET_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let client_id: String = row.try_get("client_id")?;
        Ok(Uuid::parse_str(&client_id)?)
    }
    
    pub async fn get_user_and_client_id(&self) -> Result<(Uuid, Uuid), ClientError> {
        let row = sqlx::query(Queries::GET_USER_AND_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;
        
        let user_id: String = row.try_get("user_id")?;
        let client_id: String = row.try_get("client_id")?;
        Ok((Uuid::parse_str(&user_id)?, Uuid::parse_str(&client_id)?))
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, ClientError> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        
        DbHelpers::parse_document(&row)
    }
    
    pub async fn save_document(&self, doc: &Document) -> Result<(), ClientError> {
        self.save_document_with_status(doc, None).await
    }
    
    pub(crate) async fn save_document_with_status(&self, doc: &Document, sync_status: Option<SyncStatus>) -> Result<(), ClientError> {
        let status_str = sync_status.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "synced".to_string());
        tracing::info!("DATABASE: 💾 Saving document {} with status: {}, revision: {}", 
                     doc.id, status_str, doc.revision_id);
        
        let params = DbHelpers::document_to_params(doc, sync_status)?;
        
        sqlx::query(Queries::UPSERT_DOCUMENT)
            .bind(params.0)  // id
            .bind(params.1)  // user_id
            .bind(params.2)  // content
            .bind(params.3)  // revision_id
            .bind(params.4)  // version
            .bind(params.5)  // vector_clock
            .bind(params.6)  // created_at
            .bind(params.7)  // updated_at
            .bind(params.8)  // deleted_at
            .bind(params.9)  // sync_status
            .execute(&self.pool)
            .await?;
        
        tracing::info!("DATABASE: ✅ Document {} saved successfully", doc.id);
        Ok(())
    }
    
    pub async fn get_pending_documents(&self) -> Result<Vec<PendingDocumentInfo>, ClientError> {
        tracing::info!("DATABASE: 🔍 Querying for pending documents...");
        
        let rows = sqlx::query(Queries::GET_PENDING_DOCUMENTS)
            .bind(SyncStatus::Pending.to_string())
            .fetch_all(&self.pool)
            .await?;
        
        tracing::info!("DATABASE: Found {} pending documents", rows.len());
        
        let mut pending_docs = Vec::new();
        for row in rows {
            let id: String = row.try_get("id")?;
            let last_synced_revision: Option<String> = row.try_get("last_synced_revision")?;
            let deleted_at: Option<String> = row.try_get("deleted_at")?;
            
            let doc_info = PendingDocumentInfo {
                id: Uuid::parse_str(&id)?,
                last_synced_revision: last_synced_revision.clone(),
                is_deleted: deleted_at.is_some(),
            };
            
            tracing::info!("DATABASE: Pending doc: {} | Last synced rev: {:?} | Deleted: {}", 
                         doc_info.id, last_synced_revision, doc_info.is_deleted);
            
            pending_docs.push(doc_info);
        }
        
        Ok(pending_docs)
    }
    
    pub async fn mark_synced(&self, document_id: &Uuid, revision_id: &str) -> Result<(), ClientError> {
        tracing::info!("DATABASE: 🔄 Marking document {} as synced with revision {}", document_id, revision_id);
        
        let result = sqlx::query(Queries::MARK_DOCUMENT_SYNCED)
            .bind(SyncStatus::Synced.to_string())
            .bind(revision_id)
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;
            
        tracing::info!("DATABASE: ✅ Marked {} as synced, rows affected: {}", document_id, result.rows_affected());
        
        // Verify the update worked
        let verify_result = sqlx::query("SELECT last_synced_revision FROM documents WHERE id = ?")
            .bind(document_id.to_string())
            .fetch_one(&self.pool)
            .await;
            
        match verify_result {
            Ok(row) => {
                let stored_revision: Option<String> = row.try_get("last_synced_revision").unwrap_or(None);
                tracing::info!("DATABASE: 🔍 Verification: last_synced_revision = {:?}", stored_revision);
            }
            Err(e) => {
                tracing::error!("DATABASE: ❌ Failed to verify mark_synced: {}", e);
            }
        }
        
        Ok(())
    }
    
    pub async fn delete_document(&self, document_id: &Uuid) -> Result<(), ClientError> {
        sqlx::query("UPDATE documents SET deleted_at = datetime('now'), sync_status = ? WHERE id = ?")
            .bind(SyncStatus::Pending.to_string())
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
        operation_type: ChangeEventType,
        patch: Option<&json_patch::Patch>,
    ) -> Result<(), ClientError> {
        let patch_json = patch.map(|p| serde_json::to_string(p)).transpose()?;
        
        let operation_type_str = match operation_type {
            ChangeEventType::Create => "create",
            ChangeEventType::Update => "update",
            ChangeEventType::Delete => "delete",
        };
        
        tracing::info!("DATABASE: queue_sync_operation called: doc_id={}, op_type={}, patch_size={}", 
                     document_id, operation_type_str, 
                     patch_json.as_ref().map(|p| p.len()).unwrap_or(0));
        
        let result = sqlx::query(Queries::INSERT_SYNC_QUEUE)
            .bind(document_id.to_string())
            .bind(operation_type_str)
            .bind(patch_json.clone())
            .execute(&self.pool)
            .await?;
        
        tracing::info!("DATABASE: sync_queue insert successful: rows_affected={}, doc_id={}", 
                     result.rows_affected(), document_id);
        
        // Verify the insert by immediately querying
        let count_result = sqlx::query("SELECT COUNT(*) as count FROM sync_queue WHERE document_id = ?")
            .bind(document_id.to_string())
            .fetch_one(&self.pool)
            .await;
        
        match count_result {
            Ok(row) => {
                let count: i64 = row.try_get("count").unwrap_or(0);
                tracing::info!("DATABASE: sync_queue verification: {} entries for doc_id={}", count, document_id);
            }
            Err(e) => {
                tracing::error!("DATABASE: Failed to verify sync_queue insert: {}", e);
            }
        }
        
        Ok(())
    }
    
    pub async fn get_queued_patch(&self, document_id: &Uuid) -> Result<Option<json_patch::Patch>, ClientError> {
        let row = sqlx::query(
            "SELECT patch FROM sync_queue WHERE document_id = ? AND operation_type = 'update' ORDER BY created_at DESC LIMIT 1"
        )
        .bind(document_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        
        match row {
            Some(row) => {
                let patch_json: Option<String> = row.try_get("patch")?;
                match patch_json {
                    Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }
    
    pub async fn remove_from_sync_queue(&self, document_id: &Uuid) -> Result<(), ClientError> {
        sqlx::query("DELETE FROM sync_queue WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}