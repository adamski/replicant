use sqlx::{PgPool, postgres::PgPoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
use json_patch::Patch;

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
    
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
    
    pub async fn create_user(&self, email: &str, auth_token_hash: &str) -> Result<Uuid, sqlx::Error> {
        let row = sqlx::query(
            r#"
            INSERT INTO users (email, auth_token_hash)
            VALUES ($1, $2)
            RETURNING id
            "#,
        )
        .bind(email)
        .bind(auth_token_hash)
        .fetch_one(&self.pool)
        .await?;
        
        Ok(row.get("id"))
    }
    
    pub async fn verify_auth_token(&self, user_id: &Uuid, token_hash: &str) -> Result<bool, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM users
            WHERE id = $1 AND auth_token_hash = $2
            "#,
        )
        .bind(user_id)
        .bind(token_hash)
        .fetch_one(&self.pool)
        .await?;
        
        let count: i64 = row.get("count");
        Ok(count > 0)
    }
    
    pub async fn create_document(&self, doc: &Document) -> Result<(), sqlx::Error> {
        let content_json = serde_json::to_value(&doc.content).unwrap();
        let vector_clock_json = serde_json::to_value(&doc.vector_clock).unwrap();
        
        sqlx::query(
            r#"
            INSERT INTO documents (
                id, user_id, title, content, revision_id, version,
                vector_clock, created_at, updated_at, deleted_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(doc.id)
        .bind(doc.user_id)
        .bind(&doc.title)
        .bind(content_json)
        .bind(doc.revision_id)
        .bind(doc.version as i64)
        .bind(vector_clock_json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .bind(doc.deleted_at)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, title, content, revision_id, version,
                   vector_clock, created_at, updated_at, deleted_at
            FROM documents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        
        Ok(Document {
            id: row.get("id"),
            user_id: row.get("user_id"),
            title: row.get("title"),
            content: row.get("content"),
            revision_id: row.get("revision_id"),
            version: row.get("version"),
            vector_clock: serde_json::from_value(row.get("vector_clock")).unwrap(),
            created_at: row.get::<chrono::NaiveDateTime, _>("created_at").and_utc(),
            updated_at: row.get::<chrono::NaiveDateTime, _>("updated_at").and_utc(),
            deleted_at: row.get::<Option<chrono::NaiveDateTime>, _>("deleted_at").map(|dt| dt.and_utc()),
        })
    }
    
    pub async fn update_document(&self, doc: &Document) -> Result<(), sqlx::Error> {
        let content_json = serde_json::to_value(&doc.content).unwrap();
        let vector_clock_json = serde_json::to_value(&doc.vector_clock).unwrap();
        
        sqlx::query(
            r#"
            UPDATE documents
            SET title = $2, content = $3, revision_id = $4, version = $5,
                vector_clock = $6, updated_at = $7, deleted_at = $8
            WHERE id = $1
            "#,
        )
        .bind(doc.id)
        .bind(&doc.title)
        .bind(content_json)
        .bind(doc.revision_id)
        .bind(doc.version as i64)
        .bind(vector_clock_json)
        .bind(doc.updated_at)
        .bind(doc.deleted_at)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn get_user_documents(&self, user_id: &Uuid) -> Result<Vec<Document>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, title, content, revision_id, version,
                   vector_clock, created_at, updated_at, deleted_at
            FROM documents
            WHERE user_id = $1 AND deleted_at IS NULL
            ORDER BY updated_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(rows.into_iter().map(|row| Document {
            id: row.get("id"),
            user_id: row.get("user_id"),
            title: row.get("title"),
            content: row.get("content"),
            revision_id: row.get("revision_id"),
            version: row.get("version"),
            vector_clock: serde_json::from_value(row.get("vector_clock")).unwrap(),
            created_at: row.get::<chrono::NaiveDateTime, _>("created_at").and_utc(),
            updated_at: row.get::<chrono::NaiveDateTime, _>("updated_at").and_utc(),
            deleted_at: row.get::<Option<chrono::NaiveDateTime>, _>("deleted_at").map(|dt| dt.and_utc()),
        }).collect())
    }
    
    pub async fn create_revision(
        &self,
        doc: &Document,
        patch: Option<&Patch>,
    ) -> Result<(), sqlx::Error> {
        let patch_json = patch.map(|p| serde_json::to_value(p).unwrap());
        
        sqlx::query(
            r#"
            INSERT INTO document_revisions (
                document_id, revision_id, content, patch, version, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(doc.id)
        .bind(doc.revision_id)
        .bind(serde_json::to_value(&doc.content).unwrap())
        .bind(patch_json)
        .bind(doc.version as i64)
        .bind(doc.user_id)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn add_active_connection(&self, user_id: &Uuid, connection_id: &Uuid) -> Result<(), sqlx::Error> {
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
    
    pub async fn remove_active_connection(&self, user_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "DELETE FROM active_connections WHERE user_id = $1",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
}