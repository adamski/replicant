use crate::queries::{DbHelpers, Queries};
use json_patch;
use replicant_core::protocol::ChangeEventType;
use replicant_core::{
    models::{Document, SyncStatus},
    SyncResult,
};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct PendingDocumentInfo {
    pub id: Uuid,
    pub is_deleted: bool,
}

pub struct ClientDatabase {
    pub pool: SqlitePool,
}

impl ClientDatabase {
    pub async fn new(database_url: &str) -> SyncResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> SyncResult<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn ensure_user_config(&self, server_url: &str) -> SyncResult<()> {
        // Check if user_config already exists
        let exists = sqlx::query("SELECT COUNT(*) as count FROM user_config")
            .fetch_one(&self.pool)
            .await?;

        let count: i64 = exists.try_get("count")?;

        if count == 0 {
            // No user config exists, create default
            let user_id = Uuid::new_v4();
            let client_id = Uuid::new_v4();

            sqlx::query(
                "INSERT INTO user_config (user_id, client_id, server_url) VALUES (?1, ?2, ?3)",
            )
            .bind(user_id.to_string())
            .bind(client_id.to_string())
            .bind(server_url)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn ensure_user_config_with_identifier(
        &self,
        server_url: &str,
        user_identifier: &str,
    ) -> SyncResult<()> {
        // Check if user_config already exists
        let exists = sqlx::query("SELECT COUNT(*) as count FROM user_config")
            .fetch_one(&self.pool)
            .await?;

        let count: i64 = exists.try_get("count")?;

        if count == 0 {
            // No user config exists, create with deterministic user ID
            let user_id = Self::generate_deterministic_user_id(user_identifier);
            let client_id = Uuid::new_v4(); // Client ID should always be unique per instance

            sqlx::query(
                "INSERT INTO user_config (user_id, client_id, server_url) VALUES (?1, ?2, ?3)",
            )
            .bind(user_id.to_string())
            .bind(client_id.to_string())
            .bind(server_url)
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

    pub async fn get_user_id(&self) -> SyncResult<Uuid> {
        let row = sqlx::query(Queries::GET_USER_ID)
            .fetch_one(&self.pool)
            .await?;

        let user_id: String = row.try_get("user_id")?;
        Ok(Uuid::parse_str(&user_id)?)
    }

    pub async fn get_client_id(&self) -> SyncResult<Uuid> {
        let row = sqlx::query(Queries::GET_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;

        let client_id: String = row.try_get("client_id")?;
        Ok(Uuid::parse_str(&client_id)?)
    }

    pub async fn get_user_and_client_id(&self) -> SyncResult<(Uuid, Uuid)> {
        let row = sqlx::query(Queries::GET_USER_AND_CLIENT_ID)
            .fetch_one(&self.pool)
            .await?;

        let user_id: String = row.try_get("user_id")?;
        let client_id: String = row.try_get("client_id")?;
        Ok((Uuid::parse_str(&user_id)?, Uuid::parse_str(&client_id)?))
    }

    pub async fn get_document(&self, id: &Uuid) -> SyncResult<Document> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;

        DbHelpers::parse_document(&row)
    }

    pub async fn save_document(&self, doc: &Document) -> SyncResult<()> {
        self.save_document_with_status(doc, None).await
    }

    pub(crate) async fn save_document_with_status(
        &self,
        doc: &Document,
        sync_status: Option<SyncStatus>,
    ) -> SyncResult<()> {
        let status_str = sync_status
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "synced".to_string());
        tracing::info!(
            "DATABASE: 💾 Saving document {} with status: {}, sync_revision: {}",
            doc.id,
            status_str,
            doc.sync_revision
        );

        let params = DbHelpers::document_to_params(doc, sync_status)?;

        sqlx::query(Queries::UPSERT_DOCUMENT)
            .bind(params.0) // id
            .bind(params.1) // user_id
            .bind(params.2) // content
            .bind(params.3) // version
            .bind(params.4) // created_at
            .bind(params.5) // updated_at
            .bind(params.6) // deleted_at
            .bind(params.7) // sync_status
            .bind(params.8) // title
            .execute(&self.pool)
            .await?;

        tracing::info!("DATABASE: ✅ Document {} saved successfully", doc.id);

        // Update FTS index for this document
        if let Err(e) = self.update_fts_for_document(&doc.id).await {
            tracing::warn!("FTS: Failed to update index for {}: {:?}", doc.id, e);
        }

        Ok(())
    }

    pub async fn get_pending_documents(&self) -> SyncResult<Vec<PendingDocumentInfo>> {
        tracing::info!("DATABASE: 🔍 Querying for pending documents...");
        let rows = sqlx::query(Queries::GET_PENDING_DOCUMENTS)
            .bind(SyncStatus::Pending.to_string())
            .fetch_all(&self.pool)
            .await?;

        tracing::info!("DATABASE: Found {} pending documents", rows.len());

        let mut pending_docs = Vec::new();
        for row in rows {
            let id: String = row.try_get("id")?;
            let deleted_at: Option<String> = row.try_get("deleted_at")?;

            let doc_info = PendingDocumentInfo {
                id: Uuid::parse_str(&id)?,
                is_deleted: deleted_at.is_some(),
            };

            tracing::info!(
                "DATABASE: Pending doc: {} | Deleted: {}",
                doc_info.id,
                doc_info.is_deleted
            );

            pending_docs.push(doc_info);
        }

        Ok(pending_docs)
    }

    pub async fn mark_synced(&self, document_id: &Uuid) -> SyncResult<()> {
        tracing::info!("DATABASE: 🔄 Marking document {} as synced", document_id);

        let result = sqlx::query(Queries::MARK_DOCUMENT_SYNCED)
            .bind(SyncStatus::Synced.to_string())
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;

        tracing::info!(
            "DATABASE: ✅ Marked {} as synced, rows affected: {}",
            document_id,
            result.rows_affected()
        );

        Ok(())
    }

    pub async fn update_sync_revision(
        &self,
        document_id: &Uuid,
        sync_revision: i64,
    ) -> SyncResult<()> {
        tracing::info!(
            "DATABASE: 🔄 Updating document {} sync_revision to {}",
            document_id,
            sync_revision
        );

        let result = sqlx::query("UPDATE documents SET sync_revision = ? WHERE id = ?")
            .bind(sync_revision)
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;

        tracing::info!(
            "DATABASE: ✅ Updated {} sync_revision, rows affected: {}",
            document_id,
            result.rows_affected()
        );

        Ok(())
    }

    pub async fn delete_document(&self, document_id: &Uuid) -> SyncResult<()> {
        sqlx::query("UPDATE documents SET deleted_at = ?, sync_status = ? WHERE id = ?")
            .bind(chrono::Utc::now())
            .bind(SyncStatus::Pending.to_string())
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;

        // Remove from FTS index
        if let Err(e) = self.update_fts_for_document(document_id).await {
            tracing::warn!("FTS: Failed to remove {} from index: {:?}", document_id, e);
        }

        Ok(())
    }

    pub async fn get_all_documents(&self) -> SyncResult<Vec<Document>> {
        let rows = sqlx::query("SELECT * FROM documents WHERE deleted_at IS NULL")
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| DbHelpers::parse_document(&row))
            .collect()
    }
    pub async fn count_documents(&self) -> SyncResult<i64> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    pub async fn queue_sync_operation(
        &self,
        document_id: &Uuid,
        operation_type: ChangeEventType,
        patch: Option<&json_patch::Patch>,
    ) -> SyncResult<()> {
        let patch_json = patch.map(serde_json::to_string).transpose()?;

        tracing::info!(
            "DATABASE: queue_sync_operation called: doc_id={}, op_type={}, patch_size={}",
            document_id,
            operation_type.to_string(),
            patch_json.as_ref().map(|p| p.len()).unwrap_or(0)
        );

        let result = sqlx::query(Queries::INSERT_SYNC_QUEUE)
            .bind(document_id.to_string())
            .bind(operation_type.to_string())
            .bind(patch_json.clone())
            .execute(&self.pool)
            .await?;

        tracing::info!(
            "DATABASE: sync_queue insert successful: rows_affected={}, doc_id={}",
            result.rows_affected(),
            document_id
        );

        // Verify the insert by immediately querying
        let count_result =
            sqlx::query("SELECT COUNT(*) as count FROM sync_queue WHERE document_id = ?")
                .bind(document_id.to_string())
                .fetch_one(&self.pool)
                .await;

        match count_result {
            Ok(row) => {
                let count: i64 = row.try_get("count").unwrap_or(0);
                tracing::info!(
                    "DATABASE: sync_queue verification: {} entries for doc_id={}",
                    count,
                    document_id
                );
            }
            Err(e) => {
                tracing::error!("DATABASE: Failed to verify sync_queue insert: {}", e);
            }
        }

        Ok(())
    }

    /// CRITICAL: Atomically save document and queue patch
    /// This prevents data loss if app crashes between separate operations
    pub async fn save_document_and_queue_patch(
        &self,
        doc: &Document,
        patch: &json_patch::Patch,
        operation_type: ChangeEventType,
        old_content_hash: Option<String>,
    ) -> SyncResult<()> {
        // Start a transaction for atomicity
        let mut tx = self.pool.begin().await?;

        // Save document with pending status (in transaction)
        let params = DbHelpers::document_to_params(doc, Some(SyncStatus::Pending))?;

        sqlx::query(Queries::UPSERT_DOCUMENT)
            .bind(params.0) // id
            .bind(params.1) // user_id
            .bind(params.2) // content
            .bind(params.3) // version
            .bind(params.4) // created_at
            .bind(params.5) // updated_at
            .bind(params.6) // deleted_at
            .bind(params.7) // sync_status
            .bind(params.8) // title
            .execute(&mut *tx)
            .await?;

        // Queue sync operation (in transaction)
        let patch_json = serde_json::to_string(patch)?;

        // Store old_content_hash if provided (for update operations)
        if let Some(hash) = old_content_hash {
            sqlx::query(
                "INSERT INTO sync_queue (document_id, operation_type, patch, old_content_hash) VALUES (?, ?, ?, ?)"
            )
            .bind(doc.id.to_string())
            .bind(operation_type.to_string())
            .bind(patch_json)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(Queries::INSERT_SYNC_QUEUE)
                .bind(doc.id.to_string()) // document_id
                .bind(operation_type.to_string()) // operation_type
                .bind(patch_json) // patch
                .execute(&mut *tx)
                .await?;
        }

        // Commit atomically - both operations succeed or both fail
        tx.commit().await?;

        tracing::info!(
            "DATABASE: Atomically saved document {} with pending status and queued patch",
            doc.id
        );

        // Update FTS index for this document
        if let Err(e) = self.update_fts_for_document(&doc.id).await {
            tracing::warn!("FTS: Failed to update index for {}: {:?}", doc.id, e);
        }

        Ok(())
    }

    pub async fn get_queued_patch(
        &self,
        document_id: &Uuid,
    ) -> SyncResult<Option<(json_patch::Patch, Option<String>)>> {
        let row = sqlx::query(
            "SELECT patch, old_content_hash FROM sync_queue WHERE document_id = ? AND operation_type = 'update' ORDER BY created_at DESC LIMIT 1"
        )
        .bind(document_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let patch_json: Option<String> = row.try_get("patch")?;
                let old_hash: Option<String> = row.try_get("old_content_hash").ok().flatten();
                match patch_json {
                    Some(json) => Ok(Some((serde_json::from_str(&json)?, old_hash))),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    pub async fn remove_from_sync_queue(&self, document_id: &Uuid) -> SyncResult<()> {
        sqlx::query("DELETE FROM sync_queue WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ===== FTS (Full-Text Search) Methods =====

    /// Configure which JSON paths to index for full-text search.
    /// Replaces existing configuration and rebuilds the index.
    pub async fn configure_search(&self, json_paths: &[String]) -> SyncResult<()> {
        // Use transaction to ensure config and index stay in sync
        let mut tx = self.pool.begin().await?;

        // Clear existing config
        sqlx::query(Queries::CLEAR_SEARCH_CONFIG)
            .execute(&mut *tx)
            .await?;

        // Insert new paths
        for path in json_paths {
            sqlx::query(Queries::INSERT_SEARCH_PATH)
                .bind(path)
                .execute(&mut *tx)
                .await?;
        }

        // Rebuild the index with new configuration (inline to use same transaction)
        sqlx::query(Queries::CLEAR_FTS_INDEX)
            .execute(&mut *tx)
            .await?;

        sqlx::query(Queries::REBUILD_FTS_INDEX)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            "FTS: Configured search with {} paths and rebuilt index",
            json_paths.len()
        );
        Ok(())
    }

    /// Update the FTS index entry for a single document.
    /// Call this after creating or updating a document.
    pub async fn update_fts_for_document(&self, document_id: &Uuid) -> SyncResult<()> {
        // Skip FTS update if no search paths are configured
        let has_config: (i32,) = sqlx::query_as(Queries::HAS_SEARCH_CONFIG)
            .fetch_one(&self.pool)
            .await?;
        if has_config.0 == 0 {
            return Ok(());
        }

        let doc_id_str = document_id.to_string();

        // Use transaction to ensure atomicity (no orphaned entries on crash)
        let mut tx = self.pool.begin().await?;

        // Delete existing entry
        sqlx::query(Queries::DELETE_FTS_ENTRY)
            .bind(&doc_id_str)
            .execute(&mut *tx)
            .await?;

        // Insert new entry (query handles deleted_at check internally)
        sqlx::query(Queries::UPDATE_FTS_ENTRY)
            .bind(&doc_id_str)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Rebuild the entire FTS index from all documents.
    pub async fn rebuild_fts_index(&self) -> SyncResult<()> {
        // Clear existing index
        sqlx::query(Queries::CLEAR_FTS_INDEX)
            .execute(&self.pool)
            .await?;

        // Rebuild from all non-deleted documents
        sqlx::query(Queries::REBUILD_FTS_INDEX)
            .execute(&self.pool)
            .await?;

        tracing::info!("FTS: Index rebuilt");
        Ok(())
    }

    /// Search documents using FTS5 full-text search.
    /// Returns documents matching the query, filtered by user_id.
    pub async fn search_documents(
        &self,
        user_id: &Uuid,
        query: &str,
        limit: i64,
    ) -> SyncResult<Vec<Document>> {
        let rows = sqlx::query(Queries::SEARCH_DOCUMENTS)
            .bind(user_id.to_string())
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| DbHelpers::parse_document(&row))
            .collect()
    }
}
