use sqlx::{PgPool, postgres::PgPoolOptions, Row};
use uuid::Uuid;
use sync_core::models::Document;
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
        let params = DbHelpers::document_to_params(doc);
        
        sqlx::query(Queries::CREATE_DOCUMENT)
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
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn get_document(&self, id: &Uuid) -> Result<Document, sqlx::Error> {
        let row = sqlx::query(Queries::GET_DOCUMENT)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        
        DbHelpers::parse_document(&row)
    }
    
    pub async fn update_document(&self, doc: &Document) -> Result<(), sqlx::Error> {
        let params = DbHelpers::document_to_params(doc);
        
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
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn delete_document(&self, document_id: &Uuid, user_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE documents SET deleted_at = NOW() WHERE id = $1 AND user_id = $2")
            .bind(document_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        
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
}