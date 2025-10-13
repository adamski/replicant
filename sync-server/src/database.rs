use sqlx::{PgPool, postgres::PgPoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
use sync_core::protocol::{ChangeEvent, ChangeEventType};
use json_patch::Patch;
use crate::queries::{document_to_params, parse_document};
use sync_core::SyncResult;
use tracing::instrument;

pub struct ChangeEventParams<'a> {
    pub document_id: &'a Uuid,
    pub user_id: &'a Uuid,
    pub event_type: ChangeEventType,
    pub revision_id: &'a str,
    pub forward_patch: Option<&'a serde_json::Value>,
    pub reverse_patch: Option<&'a serde_json::Value>,
    pub applied: bool,
}

pub struct ServerDatabase {
    pub pool: PgPool,
}

impl ServerDatabase {

    #[instrument(skip(database_url))]
    pub async fn new(database_url: &str) -> SyncResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn new_with_options(database_url: &str, max_connections: u32) -> SyncResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .max_lifetime(std::time::Duration::from_secs(30))  // Short lifetime for tests
            .idle_timeout(std::time::Duration::from_secs(10))
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn run_migrations(&self) -> SyncResult<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_user(
        &self,
        email: &str,
        username: Option<&str>,
    ) -> SyncResult<Uuid> {
        let row = sqlx::query(
            r#"
            INSERT INTO users (email, username)
            VALUES ($1, $2)
            RETURNING id
        "#,
        )
        .bind(email)
        .bind(username)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("id"))
    }

    pub async fn get_user_by_email(
        &self,
        email: &str,
    ) -> SyncResult<Option<Uuid>> {
        let result = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM users WHERE email = $1"
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn get_user_by_username(
        &self,
        username: &str,
    ) -> SyncResult<Option<Uuid>> {
        let result = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM users WHERE username = $1"
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }


    pub async fn create_document(&self, doc: &Document) -> SyncResult<()> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        let params = document_to_params(doc);

        // Debug: Log revision_id type
        tracing::debug!(
            "Creating document with revision_id: {} (type: String)",
            params.3
        );

        // WORKAROUND for SQLx cached statement issue #2885
        sqlx::query(
            r#"
            INSERT INTO documents (
                id, user_id, content, revision_id, version,
                vector_clock, created_at, updated_at, deleted_at, checksum, size_bytes
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        )
            .persistent(false) // Disable caching
            .bind(params.0)  // id
            .bind(params.1)  // user_id
            .bind(params.2)  // content_json
            .bind(params.3)  // revision_id
            .bind(params.4)  // version
            .bind(params.5)  // vector_clock_json
            .bind(params.6)  // created_at
            .bind(params.7)  // updated_at
            .bind(params.8)  // deleted_at
            .bind(params.9)  // checksum
            .bind(params.10) // size_bytes
            .execute(&mut *tx)
            .await?;

        // Log the create event
        // For CREATE: forward_patch contains the full document, reverse_patch is null
        let doc_json = serde_json::to_value(doc).map_err(|e| sqlx::Error::Protocol(format!("Serialization error: {}", e)))?;
        self.log_change_event(&mut tx, ChangeEventParams {
            document_id: &doc.id,
            user_id: &doc.user_id,
            event_type: ChangeEventType::Create,
            revision_id: &doc.revision_id,
            forward_patch: Some(&doc_json),
            reverse_patch: None,
            applied: true,
        }).await?;

        tx.commit().await?;
        Ok(())

    }
    
    pub async fn get_document(&self, id: &Uuid) -> SyncResult<Document> {
        let row = sqlx::query(r#"
            SELECT id, user_id, content, revision_id, version, vector_clock, created_at, updated_at, deleted_at
            FROM documents
            WHERE id = $1
        "#)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;

        parse_document(&row)

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
        patch: Option<&Patch>
    ) -> SyncResult<()> {
        // Get the original document state before update (for computing reverse patch)
        let original_doc = self.get_document(&doc.id).await?;

        let params = document_to_params(doc);

        // Update the document
        sqlx::query(
            r#"
            UPDATE documents
            SET content = $2, revision_id = $3, version = $4,
                vector_clock = $5, updated_at = $6, deleted_at = $7,
                checksum = $8, size_bytes = $9
            WHERE id = $1
        "#,
        )
            .bind(params.0)  // id
            .bind(params.2)  // content_json
            .bind(params.3)  // revision_id
            .bind(params.4)  // version
            .bind(params.5)  // vector_clock_json
            .bind(params.7)  // updated_at
            .bind(params.8)  // deleted_at
            .bind(params.9)  // checksum
            .bind(params.10) // size_bytes
            .execute(&mut **tx)
            .await?;

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

        self.log_change_event(tx, ChangeEventParams {
            document_id: &doc.id,
            user_id: &doc.user_id,
            event_type: ChangeEventType::Update,
            revision_id: &doc.revision_id,
            forward_patch: forward_patch_json.as_ref(),
            reverse_patch: reverse_patch_json.as_ref(),
            applied: true,
        }).await?;

        Ok(())
    }

    pub async fn delete_document(&self, document_id: &Uuid, user_id: &Uuid, revision_id: &str) -> SyncResult<()> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;

        // Get the document before deletion (for the reverse patch)
        let doc_to_delete = self.get_document(document_id).await?;

        // Soft delete the document
        sqlx::query("UPDATE documents SET deleted_at = NOW() WHERE id = $1 AND user_id = $2")
            .bind(document_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        // Log the delete event
        // For DELETE: forward_patch is null, reverse_patch contains the full document
        let doc_json = serde_json::to_value(&doc_to_delete).map_err(|e| sqlx::Error::Protocol(format!("Serialization error: {}", e)))?;
        self.log_change_event(&mut tx, ChangeEventParams {
            document_id,
            user_id,
            event_type: ChangeEventType::Delete,
            revision_id,
            forward_patch: None,
            reverse_patch: Some(&doc_json),
            applied: true,
        }).await?;
        
        tx.commit().await?;
        Ok(())

    }
    
    pub async fn get_user_documents(&self, user_id: &Uuid) -> SyncResult<Vec<Document>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, content, revision_id, version,
                   vector_clock, created_at, updated_at, deleted_at
            FROM documents
            WHERE user_id = $1 AND deleted_at IS NULL
            ORDER BY updated_at DESC
        "#,
        )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter().map(|row| parse_document(&row)).collect()

    }
    
    pub async fn create_revision(
        &self,
        doc: &Document,
        patch: Option<&Patch>,
    ) -> SyncResult<()> {
        let patch_json = patch.map(|p| serde_json::to_value(p).unwrap());

        sqlx::query(
            r#"
            INSERT INTO document_revisions (
                document_id, revision_id, content, patch, version, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6)
        "#,
        )
        .bind(doc.id)
        .bind(&doc.revision_id)
        .bind(serde_json::to_value(&doc.content).unwrap())
        .bind(patch_json)
        .bind(doc.version)
        .bind(doc.user_id)
        .execute(&self.pool)
        .await?;

        Ok(())

    }
    
    pub async fn add_active_connection(&self, user_id: &Uuid, connection_id: &Uuid) -> SyncResult<()> {
        sqlx::query(
            r#"
            INSERT INTO active_connections (user_id, connection_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id) DO UPDATE
            SET connection_id = $2, last_ping_at = NOW()
        "#,
        )
            .bind(user_id)
            .bind(connection_id)
            .execute(&self.pool)
            .await?;

        Ok(())

    }
    
    pub async fn remove_active_connection(&self, user_id: &Uuid) -> SyncResult<()> {
        sqlx::query(
            r#"
            DELETE FROM active_connections WHERE user_id = $1
        "#,
        )
            .bind(user_id)
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
        sqlx::query(
            r#"
            INSERT INTO change_events (user_id, document_id, event_type, revision_id, forward_patch, reverse_patch, applied)
            VALUES ($1::UUID, $2::UUID, $3::TEXT, $4::TEXT, $5::JSONB, $6::JSONB, $7::BOOLEAN)
            "#
        )
        .persistent(false)  // Disable prepared statement caching
        .bind(params.user_id)
        .bind(params.document_id)
        .bind(params.event_type.to_string())
        .bind(params.revision_id)
        .bind(params.forward_patch)
        .bind(params.reverse_patch)
        .bind(params.applied)
        .execute(&mut **tx)
        .await?;

        Ok(())

    }

    // Get changes since a specific sequence number for sync
    pub async fn get_changes_since(&self, user_id: &Uuid, last_sequence: u64, limit: Option<u32>) -> SyncResult<Vec<ChangeEvent>> {
        let limit = limit.unwrap_or(100).min(1000); // Cap at 1000 for safety

        let rows = sqlx::query(
            r#"
            SELECT sequence, document_id, user_id, event_type, revision_id, forward_patch, reverse_patch, created_at
            FROM change_events
            WHERE user_id = $1 AND sequence > $2
            ORDER BY sequence ASC
            LIMIT $3
            "#
        )
            .bind(user_id)
            .bind(last_sequence as i64)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

        let mut events = Vec::new();
        for row in rows {
            let event_type_str: String = row.get("event_type");
            let event_type = match event_type_str.parse::<ChangeEventType>() {
                Ok(et) => et,
                Err(_) => continue, // Skip unknown event types
            };

            events.push(ChangeEvent {
                sequence: row.get::<i64, _>("sequence") as u64,
                document_id: row.get("document_id"),
                user_id: row.get("user_id"),
                event_type,
                revision_id: row.get("revision_id"),
                forward_patch: row.get("forward_patch"),
                reverse_patch: row.get("reverse_patch"),
                created_at: row.get("created_at"),
            });
        }

        Ok(events)

    }

    // Get the latest sequence number for a user
    pub async fn get_latest_sequence(&self, user_id: &Uuid) -> SyncResult<u64> {
        let row = sqlx::query(
            r#"
            SELECT COALESCE(MAX(sequence), 0) as latest_sequence
            FROM change_events
            WHERE user_id = $1
            "#,
        )
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get::<i64, _>("latest_sequence") as u64)

    }
    // Get unapplied changes for a document (conflict losers)
    pub async fn get_unapplied_changes(&self, document_id: &Uuid) -> SyncResult<Vec<ChangeEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT sequence, document_id, user_id, event_type, revision_id,
                   forward_patch, reverse_patch, created_at
            FROM change_events
            WHERE document_id = $1 AND applied = false
            ORDER BY sequence DESC
            "#
        )
            .bind(document_id)
            .fetch_all(&self.pool)
            .await?;

        let mut events = Vec::new();
        for row in rows {
            let event_type_str: String = row.get("event_type");
            let event_type = match event_type_str.parse::<ChangeEventType>() {
                Ok(et) => et,
                Err(_) => continue,
            };

            events.push(ChangeEvent {
                sequence: row.get::<i64, _>("sequence") as u64,
                document_id: row.get("document_id"),
                user_id: row.get("user_id"),
                event_type,
                revision_id: row.get("revision_id"),
                forward_patch: row.get("forward_patch"),
                reverse_patch: row.get("reverse_patch"),
                created_at: row.get("created_at"),
            });
        }

        Ok(events)
    }
}
