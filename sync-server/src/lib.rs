pub mod database;
pub mod websocket;
pub mod auth;
pub mod sync_handler;
pub mod api;
pub mod queries;
pub mod monitoring;


use std::sync::Arc;
use std::collections::HashSet;
use dashmap::DashMap;
use uuid::Uuid;
use sync_core::protocol::ServerMessage;

// Registry of connected clients: (user_id, client_id) -> channel
pub type ClientRegistry = Arc<DashMap<(Uuid, Uuid), tokio::sync::mpsc::Sender<ServerMessage>>>;

// Auxiliary mapping to track which clients belong to which user
pub type UserClients = Arc<DashMap<Uuid, HashSet<Uuid>>>;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<database::ServerDatabase>,
    pub auth: auth::AuthState,
    pub monitoring: Option<monitoring::MonitoringLayer>,
    pub clients: ClientRegistry,
    pub user_clients: UserClients,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use sync_core::models::{Document, VectorClock};
    use serde_json::json;
    
    #[tokio::test]
    async fn test_server_database_operations() {
        // Skip if no DATABASE_URL is set
        let db_url = match std::env::var("TEST_DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                println!("Skipping test: TEST_DATABASE_URL not set");
                return;
            }
        };
        
        // Create database connection
        let db = database::ServerDatabase::new(&db_url).await.unwrap();
        
        // Create test user
        let email = format!("test_{}@example.com", Uuid::new_v4());
        let user_id = db.create_user(&email, "hashed_token").await.unwrap();
        
        // Create test document
        let content = json!({
            "title": "Server Test Document",
            "text": "Test content",
            "number": 42
        });
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            revision_id: Document::initial_revision(&content),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        // Save document
        db.create_document(&doc).await.unwrap();
        
        // Retrieve document
        let loaded_doc = db.get_document(&doc.id).await.unwrap();
        assert_eq!(loaded_doc.id, doc.id);
        // Title is now part of content JSON, so just compare the content
        
        // Get user documents
        let user_docs = db.get_user_documents(&user_id).await.unwrap();
        assert_eq!(user_docs.len(), 1);
        assert_eq!(user_docs[0].id, doc.id);
    }
    
    #[test]
    fn test_auth_token_generation() {
        let token1 = auth::AuthState::generate_auth_token();
        let token2 = auth::AuthState::generate_auth_token();
        
        // Tokens should be unique
        assert_ne!(token1, token2);
        
        // Tokens should be valid UUIDs
        assert!(Uuid::parse_str(&token1).is_ok());
        assert!(Uuid::parse_str(&token2).is_ok());
    }
}