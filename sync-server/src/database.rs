use crate::queries::document_to_params;
use json_patch::Patch;
use sqlx::{postgres::PgPoolOptions, PgPool};
use sync_core::models::Document;
use sync_core::protocol::{ChangeEvent, ChangeEventType};
use sync_core::{SyncError, SyncResult};
use tracing::instrument;
use uuid::Uuid;

pub struct ChangeEventParams<'a> {
    pub document_id: &'a Uuid,
    pub user_id: &'a Uuid,
    pub event_type: ChangeEventType,
    pub forward_patch: Option<&'a serde_json::Value>,
    pub reverse_patch: Option<&'a serde_json::Value>,
    pub applied: bool,
}

pub struct ServerDatabase {
    pub pool: PgPool,
    pub app_namespace_id: String,
}

impl ServerDatabase {
    #[instrument(skip(database_url))]
    pub async fn new(database_url: &str, app_namespace_id: String) -> SyncResult<Self> {
        // Use smaller connection pool in test environments to avoid exhausting PostgreSQL connections
        let max_connections = if std::env::var("RUN_INTEGRATION_TESTS").is_ok() {
            3 // Tests need fewer connections per server instance
        } else {
            10 // Production default
        };

        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .max_lifetime(std::time::Duration::from_secs(30))
            .idle_timeout(std::time::Duration::from_secs(10))
            .connect(database_url)
            .await?;

        Ok(Self {
            pool,
            app_namespace_id,
        })
    }

    pub async fn new_with_options(
        database_url: &str,
        app_namespace_id: String,
        max_connections: u32,
    ) -> SyncResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .max_lifetime(std::time::Duration::from_secs(30)) // Short lifetime for tests
            .idle_timeout(std::time::Duration::from_secs(10))
            .connect(database_url)
            .await?;

        Ok(Self {
            pool,
            app_namespace_id,
        })
    }

    pub async fn run_migrations(&self) -> SyncResult<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn create_user(&self, email: &str) -> SyncResult<Uuid> {
        // Generate deterministic user ID using UUID v5
        // This MUST match the client's logic in ClientDatabase::generate_deterministic_user_id
        let app_namespace = Uuid::new_v5(&Uuid::NAMESPACE_DNS, self.app_namespace_id.as_bytes());
        let user_id = Uuid::new_v5(&app_namespace, email.as_bytes());

        let row = sqlx::query!(
            r#"
            INSERT INTO users (id, email)
            VALUES ($1, $2)
            RETURNING id
        "#,
            user_id,
            email
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row.id)
    }

    pub async fn get_user_by_email(&self, email: &str) -> SyncResult<Option<Uuid>> {
        let result = sqlx::query_scalar!("SELECT id FROM users WHERE email = $1", email)
            .fetch_optional(&self.pool)
            .await?;

        Ok(result)
    }

    pub async fn create_document(&self, doc: &Document) -> SyncResult<()> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        let params = document_to_params(doc);

        sqlx::query!(
            r#"
            INSERT INTO documents (
                id, user_id, content, sync_revision,
                created_at, updated_at, deleted_at, content_hash, size_bytes, title
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
            params.0,      // id
            params.1,      // user_id
            params.2 as _, // content_json
            params.3,      // sync_revision
            params.4,      // created_at
            params.5,      // updated_at
            params.6,      // deleted_at
            params.7 as _, // content_hash
            params.8,      // size_bytes
            params.9 as _  // title
        )
        .execute(&mut *tx)
        .await?;

        // Log the create event
        // For CREATE: forward_patch contains the full document, reverse_patch is null
        let doc_json = serde_json::to_value(doc)
            .map_err(|e| sqlx::Error::Protocol(format!("Serialization error: {}", e)))?;
        self.log_change_event(
            &mut tx,
            ChangeEventParams {
                document_id: &doc.id,
                user_id: &doc.user_id,
                event_type: ChangeEventType::Create,
                forward_patch: Some(&doc_json),
                reverse_patch: None,
                applied: true,
            },
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_document(&self, id: &Uuid) -> SyncResult<Document> {
        let row = sqlx::query!(
            r#"
            SELECT id, user_id, content, sync_revision, content_hash, title, created_at, updated_at, deleted_at
            FROM documents
            WHERE id = $1
        "#,
            id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Document {
            id: row.id,
            user_id: row.user_id,
            content: row.content,
            sync_revision: row.sync_revision,
            content_hash: row.content_hash,
            title: row.title,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
        })
    }

    pub async fn update_document(&self, doc: &Document, patch: Option<&Patch>) -> SyncResult<()> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        self.update_document_in_tx(&mut tx, doc, patch).await?;
        tx.commit().await?;
        Ok(())
    }

    // Update document within an existing transaction
    pub async fn update_document_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        doc: &Document,
        patch: Option<&Patch>,
    ) -> SyncResult<()> {
        // CRITICAL: Read the original document INSIDE the transaction with row lock
        // This prevents race conditions in computing reverse patches
        let original_doc = sqlx::query!(
            r#"
            SELECT id, user_id, content, sync_revision, content_hash, title, created_at, updated_at, deleted_at
            FROM documents
            WHERE id = $1
            FOR UPDATE
            "#,
            doc.id
        )
        .fetch_one(&mut **tx)
        .await
        .map(|row| Document {
            id: row.id,
            user_id: row.user_id,
            content: row.content,
            sync_revision: row.sync_revision,
            content_hash: row.content_hash,
            title: row.title,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
        })?;

        let params = document_to_params(doc);
        let expected_sync_revision = original_doc.sync_revision;

        // CRITICAL: Atomic version increment with optimistic locking
        // The WHERE clause ensures we only update if version hasn't changed (optimistic lock)
        let result = sqlx::query!(
            r#"
            UPDATE documents
            SET content = $2,
                sync_revision = sync_revision + 1,
                updated_at = NOW(),
                deleted_at = $3,
                content_hash = $4,
                size_bytes = $5,
                title = $6
            WHERE id = $1 AND sync_revision = $7
            "#,
            params.0,               // id
            params.2 as _,          // content_json
            params.6,               // deleted_at
            params.7,               // content_hash
            params.8,               // size_bytes
            params.9 as _,          // title
            expected_sync_revision  // optimistic lock check
        )
        .execute(&mut **tx)
        .await?;

        // Check if the update actually happened
        if result.rows_affected() == 0 {
            // Version mismatch - another transaction updated the document first
            return Err(SyncError::VersionMismatch {
                expected: expected_sync_revision,
                actual: doc.sync_revision, // The version the client sent
            });
        }

        // Compute patches for the event log
        let forward_patch_json = patch.map(|p| serde_json::to_value(p).unwrap());
        let reverse_patch_json = if let Some(fwd_patch) = patch {
            // Compute the reverse patch using the original document content
            match sync_core::patches::compute_reverse_patch(&original_doc.content, fwd_patch) {
                Ok(rev_patch) => Some(serde_json::to_value(rev_patch).unwrap()),
                Err(_) => None, // If we can't compute reverse patch, store null
            }
        } else {
            None
        };

        self.log_change_event(
            tx,
            ChangeEventParams {
                document_id: &doc.id,
                user_id: &doc.user_id,
                event_type: ChangeEventType::Update,
                forward_patch: forward_patch_json.as_ref(),
                reverse_patch: reverse_patch_json.as_ref(),
                applied: true,
            },
        )
        .await?;

        Ok(())
    }

    pub async fn delete_document(&self, document_id: &Uuid, user_id: &Uuid) -> SyncResult<()> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;

        // Get the document before deletion (for the reverse patch)
        let doc_to_delete = self.get_document(document_id).await?;

        // Soft delete the document
        sqlx::query!(
            "UPDATE documents SET deleted_at = NOW() WHERE id = $1 AND user_id = $2",
            document_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        // Log the delete event
        // For DELETE: forward_patch is null, reverse_patch contains the full document
        let doc_json = serde_json::to_value(&doc_to_delete)
            .map_err(|e| sqlx::Error::Protocol(format!("Serialization error: {}", e)))?;
        self.log_change_event(
            &mut tx,
            ChangeEventParams {
                document_id,
                user_id,
                event_type: ChangeEventType::Delete,
                forward_patch: None,
                reverse_patch: Some(&doc_json),
                applied: true,
            },
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_user_documents(&self, user_id: &Uuid) -> SyncResult<Vec<Document>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, user_id, content, sync_revision, content_hash, title,
                   created_at, updated_at, deleted_at
            FROM documents
            WHERE user_id = $1 AND deleted_at IS NULL
            ORDER BY updated_at DESC
        "#,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| Document {
                id: row.id,
                user_id: row.user_id,
                content: row.content,
                sync_revision: row.sync_revision,
                content_hash: row.content_hash,
                title: row.title,
                created_at: row.created_at,
                updated_at: row.updated_at,
                deleted_at: row.deleted_at,
            })
            .collect())
    }

    pub async fn create_revision(&self, doc: &Document, patch: Option<&Patch>) -> SyncResult<()> {
        let patch_json = patch.map(|p| serde_json::to_value(p).unwrap());
        let content_json = serde_json::to_value(&doc.content).unwrap();

        sqlx::query!(
            r#"
            INSERT INTO document_revisions (
                document_id, content, patch, sync_revision, created_by
            ) VALUES ($1, $2, $3, $4, $5)
        "#,
            doc.id,
            content_json as _,
            patch_json as _,
            doc.sync_revision,
            doc.user_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn add_active_connection(
        &self,
        user_id: &Uuid,
        connection_id: &Uuid,
    ) -> SyncResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO active_connections (user_id, connection_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id) DO UPDATE
            SET connection_id = $2, last_ping_at = NOW()
        "#,
            user_id,
            connection_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn remove_active_connection(&self, user_id: &Uuid) -> SyncResult<()> {
        sqlx::query!(
            r#"
            DELETE FROM active_connections WHERE user_id = $1
        "#,
            user_id
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // Event logging for sequence-based sync
    pub async fn log_change_event(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        params: ChangeEventParams<'_>,
    ) -> SyncResult<()> {
        let event_type_str = params.event_type.to_string();
        sqlx::query!(
            r#"
            INSERT INTO change_events (user_id, document_id, event_type, forward_patch, reverse_patch, applied)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            params.user_id,
            params.document_id,
            event_type_str,
            params.forward_patch as _,
            params.reverse_patch as _,
            params.applied
        )
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    // Get changes since a specific sequence number for sync
    pub async fn get_changes_since(
        &self,
        user_id: &Uuid,
        last_sequence: u64,
        limit: Option<u32>,
    ) -> SyncResult<Vec<ChangeEvent>> {
        let limit = limit.unwrap_or(100).min(1000); // Cap at 1000 for safety
        let last_seq_i64 = last_sequence as i64;
        let limit_i64 = limit as i64;

        let rows = sqlx::query!(
            r#"
            SELECT sequence, document_id, user_id, event_type, forward_patch, reverse_patch, created_at
            FROM change_events
            WHERE user_id = $1 AND sequence > $2
            ORDER BY sequence ASC
            LIMIT $3
            "#,
            user_id,
            last_seq_i64,
            limit_i64
        )
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::new();
        for row in rows {
            let event_type = match row.event_type.parse::<ChangeEventType>() {
                Ok(et) => et,
                Err(_) => continue, // Skip unknown event types
            };

            events.push(ChangeEvent {
                sequence: row.sequence as u64,
                document_id: row.document_id,
                user_id: row.user_id,
                event_type,
                forward_patch: row.forward_patch,
                reverse_patch: row.reverse_patch,
                created_at: row.created_at,
            });
        }

        Ok(events)
    }

    // Get the latest sequence number for a user
    pub async fn get_latest_sequence(&self, user_id: &Uuid) -> SyncResult<u64> {
        let latest_sequence = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(MAX(sequence), 0) as "latest_sequence!"
            FROM change_events
            WHERE user_id = $1
            "#,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(latest_sequence as u64)
    }
    // Get unapplied changes for a document (conflict losers)
    pub async fn get_unapplied_changes(&self, document_id: &Uuid) -> SyncResult<Vec<ChangeEvent>> {
        let rows = sqlx::query!(
            r#"
            SELECT sequence, document_id, user_id, event_type,
                   forward_patch, reverse_patch, created_at
            FROM change_events
            WHERE document_id = $1 AND applied = false
            ORDER BY sequence DESC
            "#,
            document_id
        )
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::new();
        for row in rows {
            let event_type = match row.event_type.parse::<ChangeEventType>() {
                Ok(et) => et,
                Err(_) => continue,
            };

            events.push(ChangeEvent {
                sequence: row.sequence as u64,
                document_id: row.document_id,
                user_id: row.user_id,
                event_type,
                forward_patch: row.forward_patch,
                reverse_patch: row.reverse_patch,
                created_at: row.created_at,
            });
        }

        Ok(events)
    }
}
