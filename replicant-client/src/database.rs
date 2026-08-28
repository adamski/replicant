use crate::queries::{DbHelpers, Queries};
use json_patch;
use replicant_core::protocol::ChangeEventType;
use replicant_core::{
    models::{Document, SyncStatus},
    SyncError, SyncResult,
};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use uuid::Uuid;

/// The state a caller verified before deciding to write. Passing it back to a
/// `*_if_unchanged` write turns read-check-write into a compare-and-swap, so a
/// local edit landing in between is never silently clobbered.
///
/// `content` compares as its stored JSON text: `serde_json` orders object keys
/// deterministically, so equal `Value`s always serialise to equal strings.
#[derive(Debug, Clone)]
pub struct DocumentPreImage {
    pub sync_revision: i64,
    pub content: serde_json::Value,
    pub sync_status: SyncStatus,
}

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
        _user_identifier: &str,
    ) -> SyncResult<()> {
        // Check if user_config already exists
        let exists = sqlx::query("SELECT COUNT(*) as count FROM user_config")
            .fetch_one(&self.pool)
            .await?;

        let count: i64 = exists.try_get("count")?;

        if count == 0 {
            // Provisional random id; the server's canonical id is adopted on first contact.
            let user_id = Uuid::new_v4();
            let client_id = Uuid::new_v4(); // Client ID should always be unique per instance

            sqlx::query(
                "INSERT INTO user_config (user_id, client_id, server_url, identity_adopted) VALUES (?1, ?2, ?3, 0)",
            )
            .bind(user_id.to_string())
            .bind(client_id.to_string())
            .bind(server_url)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn get_user_id(&self) -> SyncResult<Uuid> {
        let row = sqlx::query(Queries::GET_USER_ID)
            .fetch_one(&self.pool)
            .await?;

        let user_id: String = row.try_get("user_id")?;
        Ok(Uuid::parse_str(&user_id)?)
    }

    pub async fn is_identity_adopted(&self) -> SyncResult<bool> {
        let row = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
            .fetch_one(&self.pool)
            .await?;
        let adopted: i64 = row.try_get("identity_adopted")?;
        Ok(adopted != 0)
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

    /// Atomically adopt the server's canonical id: re-stamp local documents
    /// owned by `old_id` and flip `user_config` to `canonical_id` with
    /// `identity_adopted = 1`. A crash mid-adoption leaves the old id intact.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::InvalidOperation`] if `canonical_id` is nil, equals
    /// `old_id`, the identity was already adopted, or no `user_config` row
    /// matches `old_id`; database failures surface as the underlying error.
    pub async fn adopt_identity(&self, old_id: Uuid, canonical_id: Uuid) -> SyncResult<()> {
        if canonical_id.is_nil() || old_id == canonical_id {
            return Err(SyncError::InvalidOperation(
                "adopt_identity: canonical_id must be non-nil and differ from old_id".to_string(),
            ));
        }
        if self.is_identity_adopted().await? {
            return Err(SyncError::InvalidOperation(
                "adopt_identity: identity has already been adopted".to_string(),
            ));
        }

        let mut tx = self.pool.begin().await?;

        sqlx::query(Queries::RESTAMP_DOCUMENTS_USER_ID)
            .bind(canonical_id.to_string())
            .bind(old_id.to_string())
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query(Queries::ADOPT_USER_CONFIG_IDENTITY)
            .bind(canonical_id.to_string())
            .bind(old_id.to_string())
            .execute(&mut *tx)
            .await?;

        if result.rows_affected() != 1 {
            return Err(SyncError::InvalidOperation(format!(
                "adopt_identity: expected to update 1 user_config row for old_id {}, got {}",
                old_id,
                result.rows_affected()
            )));
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_document(&self, id: &Uuid) -> SyncResult<Document> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;

        DbHelpers::parse_document(&row)
    }

    /// Read the row's `sync_status`. `Ok(None)` means the document is not in
    /// the local database. `Document` does not carry the status, but the
    /// broadcast guard has to know whether local edits are outstanding.
    ///
    /// An unparseable status fails CLOSED (`Conflict`): `Synced` is the one
    /// value that authorises a blind patch apply, so nothing may reach it by
    /// way of a fallback.
    pub async fn get_sync_status(&self, id: &Uuid) -> SyncResult<Option<SyncStatus>> {
        let row = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => {
                let raw: String = row.try_get("sync_status")?;
                Ok(Some(raw.parse::<SyncStatus>().unwrap_or_else(|_| {
                    tracing::warn!(
                        "DATABASE: Unrecognised sync_status {:?} for {}, treating as conflict",
                        raw,
                        id
                    );
                    SyncStatus::Conflict
                })))
            }
            None => Ok(None),
        }
    }

    /// Overwrite a document only if it still matches `expected`. Returns
    /// `false` when a concurrent local write landed first, in which case the
    /// caller must back off instead of overwriting it.
    ///
    /// Attribution columns are deliberately left untouched: this write carries
    /// sync state, not authorship.
    pub async fn save_document_if_unchanged(
        &self,
        doc: &Document,
        new_status: SyncStatus,
        expected: &DocumentPreImage,
    ) -> SyncResult<bool> {
        let params = DbHelpers::document_to_params(doc, Some(new_status))?;

        let result = sqlx::query(
            "UPDATE documents SET content = ?, sync_revision = ?, updated_at = ?, sync_status = ?, title = ? \
             WHERE id = ? AND content = ? AND sync_revision = ? AND sync_status = ?",
        )
        .bind(params.2) // content
        .bind(params.3) // sync_revision
        .bind(params.5) // updated_at
        .bind(params.7) // sync_status
        .bind(params.8) // title
        .bind(doc.id.to_string())
        .bind(serde_json::to_string(&expected.content)?)
        .bind(expected.sync_revision)
        .bind(expected.sync_status.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            tracing::warn!(
                "DATABASE: Compare-and-swap for {} lost to a concurrent local write",
                doc.id
            );
            return Ok(false);
        }

        if let Err(e) = self.update_fts_for_document(&doc.id).await {
            tracing::warn!("FTS: Failed to update index for {}: {:?}", doc.id, e);
        }

        Ok(true)
    }

    /// Rebase a document carrying unsent local edits onto a fresh server base:
    /// adopt `new_content` at the server's revision and replace the queued
    /// update patch, in one transaction.
    ///
    /// `new_content` is what the local copy should now show. A resync passes the
    /// content it already has (the user's unsent edit stays visible); a
    /// `hash_mismatch` rebase passes the merged result of replaying the queued
    /// patch onto the server's content.
    ///
    /// Prior `update` rows for the document are deleted rather than added to.
    /// `get_queued_patch` reads exactly one row, so a leftover row with an
    /// outdated base hash would be retried — and rejected — forever.
    ///
    /// Returns `false` if the row no longer matches `expected`.
    #[allow(clippy::too_many_arguments)]
    pub async fn rebase_pending_document_if_unchanged(
        &self,
        document_id: &Uuid,
        new_content: &serde_json::Value,
        new_sync_revision: i64,
        patch: &json_patch::Patch,
        base_content_hash: &str,
        expected: &DocumentPreImage,
    ) -> SyncResult<bool> {
        let mut tx = self.pool.begin().await?;

        let result = sqlx::query(
            "UPDATE documents SET content = ?, sync_revision = ?, updated_at = ?, sync_status = ? \
             WHERE id = ? AND content = ? AND sync_revision = ? AND sync_status = ?",
        )
        .bind(serde_json::to_string(new_content)?)
        .bind(new_sync_revision)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(SyncStatus::Pending.to_string())
        .bind(document_id.to_string())
        .bind(serde_json::to_string(&expected.content)?)
        .bind(expected.sync_revision)
        .bind(expected.sync_status.to_string())
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            tx.rollback().await?;
            tracing::warn!(
                "DATABASE: Rebase of {} lost to a concurrent local write",
                document_id
            );
            return Ok(false);
        }

        sqlx::query("DELETE FROM sync_queue WHERE document_id = ? AND operation_type = ?")
            .bind(document_id.to_string())
            .bind(ChangeEventType::Update.to_string())
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO sync_queue (document_id, operation_type, patch, old_content_hash) VALUES (?, ?, ?, ?)",
        )
        .bind(document_id.to_string())
        .bind(ChangeEventType::Update.to_string())
        .bind(serde_json::to_string(patch)?)
        .bind(base_content_hash)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        if let Err(e) = self.update_fts_for_document(document_id).await {
            tracing::warn!("FTS: Failed to update index for {}: {:?}", document_id, e);
        }

        Ok(true)
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
            .bind(params.9) // author_name
            .bind(params.10) // visibility
            .bind(params.11) // provenance
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

    pub async fn update_attribution(
        &self,
        document_id: &Uuid,
        author_name: Option<String>,
        visibility: Option<String>,
        provenance: Option<serde_json::Value>,
    ) -> SyncResult<()> {
        tracing::info!("DATABASE: 🔄 Updating document {} attribution", document_id);

        let provenance_str = provenance.map(|v| v.to_string());

        let result = sqlx::query(
            "UPDATE documents SET author_name = ?, visibility = ?, provenance = ? WHERE id = ?",
        )
        .bind(author_name)
        .bind(visibility)
        .bind(provenance_str)
        .bind(document_id.to_string())
        .execute(&self.pool)
        .await?;

        tracing::info!(
            "DATABASE: ✅ Updated {} attribution, rows affected: {}",
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

    pub async fn get_all_document_ids(&self, include_deleted: bool) -> SyncResult<Vec<Uuid>> {
        let query = if include_deleted {
            "SELECT id FROM documents"
        } else {
            "SELECT id FROM documents WHERE deleted_at IS NULL"
        };

        let rows = sqlx::query(query).fetch_all(&self.pool).await?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.get("id");
                Ok(Uuid::parse_str(&id)?)
            })
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
            "SELECT patch, old_content_hash FROM sync_queue WHERE document_id = ? AND operation_type = 'update' ORDER BY created_at DESC, id DESC LIMIT 1"
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
    /// Returns all documents matching the query.
    pub async fn search_documents(&self, query: &str, limit: i64) -> SyncResult<Vec<Document>> {
        let rows = sqlx::query(Queries::SEARCH_DOCUMENTS)
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| DbHelpers::parse_document(&row))
            .collect()
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    async fn fresh_db() -> ClientDatabase {
        let db = ClientDatabase::new(":memory:").await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    #[tokio::test]
    async fn user_config_has_identity_adopted_defaulting_to_zero() {
        let db = fresh_db().await;
        db.ensure_user_config("ws://localhost/ws").await.unwrap();

        let row = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let adopted: i64 = row.try_get("identity_adopted").unwrap();
        assert_eq!(adopted, 0);
    }

    #[tokio::test]
    async fn ensure_user_config_with_identifier_generates_random_v4_id() {
        let db = fresh_db().await;
        db.ensure_user_config_with_identifier("ws://localhost/ws", "test@example.com")
            .await
            .unwrap();

        let user_id = db.get_user_id().await.unwrap();
        // No longer derived from the email.
        assert_ne!(user_id.to_string(), "71b2b712-7878-56ee-8323-43809b8198a5");
        assert_eq!(user_id.get_version(), Some(uuid::Version::Random));

        let row = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let adopted: i64 = row.try_get("identity_adopted").unwrap();
        assert_eq!(adopted, 0);
    }

    async fn seed_document(db: &ClientDatabase, owner: Option<Uuid>) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO documents (id, user_id, content) VALUES (?1, ?2, ?3)")
            .bind(id.to_string())
            .bind(owner.map(|u| u.to_string()))
            .bind("{}")
            .execute(&db.pool)
            .await
            .unwrap();
        id
    }

    async fn count_docs_for(db: &ClientDatabase, owner: Uuid) -> i64 {
        sqlx::query("SELECT COUNT(*) as c FROM documents WHERE user_id = ?1")
            .bind(owner.to_string())
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("c")
            .unwrap()
    }

    #[tokio::test]
    async fn adopt_identity_restamps_docs_and_flips_flag() {
        let db = fresh_db().await;
        db.ensure_user_config("ws://localhost/ws").await.unwrap();
        let provisional = db.get_user_id().await.unwrap();
        let canonical = Uuid::new_v4();

        seed_document(&db, Some(provisional)).await;
        seed_document(&db, Some(provisional)).await;
        let public_id = seed_document(&db, None).await;

        db.adopt_identity(provisional, canonical).await.unwrap();

        // Identity flipped in user_config.
        assert_eq!(db.get_user_id().await.unwrap(), canonical);
        let adopted: i64 = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("identity_adopted")
            .unwrap();
        assert_eq!(adopted, 1);

        // Owned documents re-stamped; none remain under the provisional id.
        assert_eq!(count_docs_for(&db, canonical).await, 2);
        assert_eq!(count_docs_for(&db, provisional).await, 0);

        // Public (null-owner) document untouched.
        let pub_null: i64 =
            sqlx::query("SELECT COUNT(*) as c FROM documents WHERE id = ?1 AND user_id IS NULL")
                .bind(public_id.to_string())
                .fetch_one(&db.pool)
                .await
                .unwrap()
                .try_get("c")
                .unwrap();
        assert_eq!(pub_null, 1);
    }

    #[tokio::test]
    async fn adopt_identity_with_no_documents_still_flips_flag() {
        let db = fresh_db().await;
        db.ensure_user_config("ws://localhost/ws").await.unwrap();
        let provisional = db.get_user_id().await.unwrap();
        let canonical = Uuid::new_v4();

        db.adopt_identity(provisional, canonical).await.unwrap();

        assert_eq!(db.get_user_id().await.unwrap(), canonical);
        let adopted: i64 = sqlx::query("SELECT identity_adopted FROM user_config LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap()
            .try_get("identity_adopted")
            .unwrap();
        assert_eq!(adopted, 1);
    }

    #[tokio::test]
    async fn adopt_identity_rejects_nil_canonical_id() {
        let db = fresh_db().await;
        db.ensure_user_config("ws://localhost/ws").await.unwrap();
        let provisional = db.get_user_id().await.unwrap();
        assert!(db.adopt_identity(provisional, Uuid::nil()).await.is_err());
    }

    #[tokio::test]
    async fn adopt_identity_rejects_second_adoption() {
        let db = fresh_db().await;
        db.ensure_user_config("ws://localhost/ws").await.unwrap();
        let provisional = db.get_user_id().await.unwrap();
        let canonical = Uuid::new_v4();
        db.adopt_identity(provisional, canonical).await.unwrap();
        assert!(db.adopt_identity(canonical, Uuid::new_v4()).await.is_err());
    }
}
