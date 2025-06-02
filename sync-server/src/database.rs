use sqlx::{PgPool, postgres::PgPoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
use sync_core::protocol::{ChangeEvent, ChangeEventType};
use json_patch::Patch;
use crate::queries::{Queries, DbHelpers};

pub struct ServerDatabase {
    pub pool: PgPool,
}

impl ServerDatabase {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn new_with_options(database_url: &str, max_connections: u32) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .max_lifetime(std::time::Duration::from_secs(30))  // Short lifetime for tests
            .idle_timeout(std::time::Duration::from_secs(10))
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
    
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
    
    pub async fn create_user(&self, email: &str, auth_token_hash: &str) -> Result<Uuid, sqlx::Error> {
        let row = sqlx::query(Queries::CREATE_USER)
            .bind(email)
            .bind(auth_token_hash)
            .fetch_one(&self.pool)
            .await?;
        
        Ok(row.get("id"))
    }
    
    pub async fn verify_auth_token(&self, user_id: &Uuid, token_hash: &str) -> Result<bool, sqlx::Error> {
        let row = sqlx::query(Queries::VERIFY_AUTH_TOKEN)
            .bind(user_id)
            .bind(token_hash)
            .fetch_one(&self.pool)
            .await?;
        
        let count: i64 = row.get("count");
        Ok(count > 0)
    }
    
    pub async fn create_document(&self, doc: &Document) -> Result<(), sqlx::Error> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        
        let params = DbHelpers::document_to_params(doc);
        
        // Debug: Log revision_id type
        tracing::debug!("Creating document with revision_id: {} (type: String)", params.4);
        
        // WORKAROUND for SQLx cached statement issue #2885
        // Use Queries::CREATE_DOCUMENT from queries.rs but clear any cached statements first
        // by using a fresh query string each time (append whitespace)
        let query_str = format!("{} ", Queries::CREATE_DOCUMENT); // Extra space forces new statement
        
        sqlx::query(&query_str)
            .persistent(false)  // Also disable caching
            .bind(params.0)  // id
            .bind(params.1)  // user_id
            .bind(params.2)  // title
            .bind(params.3)  // content_json
            .bind(params.4)  // revision_id
            .bind(params.5)  // version
            .bind(params.6)  // vector_clock_json
            .bind(params.7)  // created_at
            .bind(params.8)  // updated_at
            .bind(params.9)  // deleted_at
            .bind(params.10) // checksum
            .bind(params.11) // size_bytes
            .execute(&mut *tx)
            .await?;
        
        // Log the create event
        self.log_change_event(&mut tx, &doc.id, &doc.user_id, ChangeEventType::Create, &doc.revision_id, None).await?;
        
        tx.commit().await?;
        Ok(())
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, sqlx::Error> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        
        DbHelpers::parse_document(&row)
    }
    
    pub async fn update_document(&self, doc: &Document, patch: Option<&Patch>) -> Result<(), sqlx::Error> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        
        let params = DbHelpers::document_to_params(doc);
        
        // Update the document
        sqlx::query(Queries::UPDATE_DOCUMENT)
            .bind(params.0)  // id
            .bind(params.2)  // title
            .bind(params.3)  // content_json
            .bind(params.4)  // revision_id
            .bind(params.5)  // version
            .bind(params.6)  // vector_clock_json
            .bind(params.8)  // updated_at
            .bind(params.9)  // deleted_at
            .bind(params.10) // checksum
            .bind(params.11) // size_bytes
            .execute(&mut *tx)
            .await?;
        
        // Log the update event with patch data
        let patch_json = patch.map(|p| serde_json::to_value(p).unwrap());
        self.log_change_event(&mut tx, &doc.id, &doc.user_id, ChangeEventType::Update, &doc.revision_id, patch_json.as_ref()).await?;
        
        tx.commit().await?;
        Ok(())
    }
    
    pub async fn delete_document(&self, document_id: &Uuid, user_id: &Uuid, revision_id: &str) -> Result<(), sqlx::Error> {
        // Start a transaction to ensure atomicity
        let mut tx = self.pool.begin().await?;
        
        // Soft delete the document
        sqlx::query("UPDATE documents SET deleted_at = NOW() WHERE id = $1 AND user_id = $2")
            .bind(document_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        
        // Log the delete event
        self.log_change_event(&mut tx, document_id, user_id, ChangeEventType::Delete, revision_id, None).await?;
        
        tx.commit().await?;
        Ok(())
    }
    
    pub async fn get_user_documents(&self, user_id: &Uuid) -> Result<Vec<Document>, sqlx::Error> {
        let rows = sqlx::query(Queries::GET_USER_DOCUMENTS)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        
        rows.into_iter()
            .map(|row| DbHelpers::parse_document(&row))
            .collect()
    }
    
    pub async fn create_revision(
        &self,
        doc: &Document,
        patch: Option<&Patch>,
    ) -> Result<(), sqlx::Error> {
        let patch_json = patch.map(|p| serde_json::to_value(p).unwrap());
        
        sqlx::query(Queries::CREATE_REVISION)
            .bind(doc.id)
            .bind(&doc.revision_id)
            .bind(serde_json::to_value(&doc.content).unwrap())
            .bind(patch_json)
            .bind(doc.version as i64)
            .bind(doc.user_id)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn add_active_connection(&self, user_id: &Uuid, connection_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(Queries::ADD_ACTIVE_CONNECTION)
            .bind(user_id)
            .bind(connection_id)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn remove_active_connection(&self, user_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(Queries::REMOVE_ACTIVE_CONNECTION)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }

    // Event logging for sequence-based sync
    async fn log_change_event(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        document_id: &Uuid,
        user_id: &Uuid,
        event_type: ChangeEventType,
        revision_id: &str,
        json_patch: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        let event_type_str = match event_type {
            ChangeEventType::Create => "create",
            ChangeEventType::Update => "update", 
            ChangeEventType::Delete => "delete",
        };

        sqlx::query(
            r#"
            INSERT INTO change_events (user_id, document_id, event_type, revision_id, json_patch)
            VALUES ($1::UUID, $2::UUID, $3::TEXT, $4::TEXT, $5::JSONB)
            "#
        )
        .persistent(false)  // Disable prepared statement caching
        .bind(user_id)
        .bind(document_id)
        .bind(event_type_str)
        .bind(revision_id)
        .bind(json_patch)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    // Get changes since a specific sequence number for sync
    pub async fn get_changes_since(&self, user_id: &Uuid, last_sequence: u64, limit: Option<u32>) -> Result<Vec<ChangeEvent>, sqlx::Error> {
        let limit = limit.unwrap_or(100).min(1000); // Cap at 1000 for safety
        
        let rows = sqlx::query(
            r#"
            SELECT sequence, document_id, user_id, event_type, revision_id, json_patch, created_at
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
            let event_type = match event_type_str.as_str() {
                "create" => ChangeEventType::Create,
                "update" => ChangeEventType::Update,
                "delete" => ChangeEventType::Delete,
                _ => continue, // Skip unknown event types
            };

            events.push(ChangeEvent {
                sequence: row.get::<i64, _>("sequence") as u64,
                document_id: row.get("document_id"),
                user_id: row.get("user_id"),
                event_type,
                revision_id: row.get("revision_id"),
                json_patch: row.get("json_patch"),
                created_at: row.get("created_at"),
            });
        }

        Ok(events)
    }

    // Get the latest sequence number for a user
    pub async fn get_latest_sequence(&self, user_id: &Uuid) -> Result<u64, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT COALESCE(MAX(sequence), 0) as latest_sequence
            FROM change_events 
            WHERE user_id = $1
            "#
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("latest_sequence") as u64)
    }
}