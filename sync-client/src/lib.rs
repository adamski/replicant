pub mod database;
pub mod sync_engine;
pub mod websocket;
pub mod offline_queue;
pub mod errors;
pub mod queries;
pub mod events;

// C FFI module
pub mod ffi;

// C FFI test functions (debug builds only)
#[cfg(debug_assertions)]
pub mod ffi_test;

pub use database::ClientDatabase;
pub use sync_engine::SyncEngine;
pub use websocket::WebSocketClient;
pub use errors::ClientError;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use serde_json::json;
    use sync_core::models::{Document, VectorClock};
    use sqlx::Row;
    
    #[tokio::test]
    async fn test_client_database_operations() {
        // Create in-memory SQLite database
        let db = ClientDatabase::new(":memory:").await.unwrap();
        
        // Create schema manually for in-memory database
        sqlx::query(
            r#"
            CREATE TABLE user_config (
                user_id TEXT PRIMARY KEY,
                server_url TEXT NOT NULL,
                last_sync_at TIMESTAMP,
                auth_token TEXT
            );
            
            CREATE TABLE documents (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content JSON NOT NULL,
                revision_id TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                vector_clock JSON,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                deleted_at TIMESTAMP,
                local_changes JSON,
                sync_status TEXT DEFAULT 'synced',
                last_synced_revision TEXT,
                CHECK (sync_status IN ('synced', 'pending', 'conflict'))
            );
            "#
        )
        .execute(&db.pool)
        .await
        .unwrap();
        
        // Insert test user
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_config (user_id, server_url, auth_token) VALUES (?1, ?2, ?3)"
        )
        .bind(user_id.to_string())
        .bind("ws://localhost:8080/ws")
        .bind("test-token")
        .execute(&db.pool)
        .await
        .unwrap();
        
        // Test get_user_id
        let retrieved_user_id = db.get_user_id().await.unwrap();
        assert_eq!(retrieved_user_id, user_id);
        
        // Create a test document
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: "Test Document".to_string(),
            content: json!({
                "text": "Hello, World!",
                "tags": ["test"]
            }),
            revision_id: Document::initial_revision(&json!({
                "text": "Hello, World!"
            })),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        // Save document
        db.save_document(&doc).await.unwrap();
        
        // Retrieve document
        let loaded_doc = db.get_document(&doc.id).await.unwrap();
        assert_eq!(loaded_doc.id, doc.id);
        assert_eq!(loaded_doc.title, doc.title);
        assert_eq!(loaded_doc.content, doc.content);
    }
    
    #[test]
    fn test_offline_queue_message_extraction() {
        use sync_core::protocol::ClientMessage;
        use crate::offline_queue::{extract_document_id, operation_type};
        
        let doc_id = Uuid::new_v4();
        
        // Test create message
        let create_msg = ClientMessage::CreateDocument {
            document: Document {
                id: doc_id,
                user_id: Uuid::new_v4(),
                title: "Test".to_string(),
                content: json!({}),
                revision_id: Document::initial_revision(&json!({})),
                version: 1,
                vector_clock: VectorClock::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            }
        };
        
        assert_eq!(extract_document_id(&create_msg), Some(doc_id));
        assert_eq!(operation_type(&create_msg), "create");
        
        // Test delete message
        let delete_msg = ClientMessage::DeleteDocument {
            document_id: doc_id,
            revision_id: "1-test".to_string(),
        };
        
        assert_eq!(extract_document_id(&delete_msg), Some(doc_id));
        assert_eq!(operation_type(&delete_msg), "delete");
    }
    
    #[tokio::test]
    async fn test_delete_document() {
        // Create in-memory SQLite database
        let db = ClientDatabase::new(":memory:").await.unwrap();
        
        // Create schema manually for in-memory database
        sqlx::query(
            r#"
            CREATE TABLE user_config (
                user_id TEXT PRIMARY KEY,
                server_url TEXT NOT NULL,
                last_sync_at TIMESTAMP,
                auth_token TEXT
            );
            
            CREATE TABLE documents (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                title TEXT NOT NULL,
                content JSON NOT NULL,
                revision_id TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                vector_clock JSON,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                deleted_at TIMESTAMP,
                local_changes JSON,
                sync_status TEXT DEFAULT 'synced',
                last_synced_revision TEXT,
                CHECK (sync_status IN ('synced', 'pending', 'conflict'))
            );
            "#
        )
        .execute(&db.pool)
        .await
        .unwrap();
        
        // Insert test user
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_config (user_id, server_url, auth_token) VALUES (?1, ?2, ?3)"
        )
        .bind(user_id.to_string())
        .bind("ws://localhost:8080/ws")
        .bind("test-token")
        .execute(&db.pool)
        .await
        .unwrap();
        
        // Create a test document
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: "Test Document".to_string(),
            content: json!({
                "text": "Hello, World!"
            }),
            revision_id: Document::initial_revision(&json!({
                "text": "Hello, World!"
            })),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        // Save document
        db.save_document(&doc).await.unwrap();
        
        // Delete document
        db.delete_document(&doc.id).await.unwrap();
        
        // Try to retrieve deleted document - it should still exist but with deleted_at set
        let loaded_doc = db.get_document(&doc.id).await.unwrap();
        assert_eq!(loaded_doc.id, doc.id);
        
        // Check that deleted_at is set
        let row = sqlx::query("SELECT deleted_at FROM documents WHERE id = ?")
            .bind(doc.id.to_string())
            .fetch_one(&db.pool)
            .await
            .unwrap();
        
        let deleted_at: Option<String> = row.try_get("deleted_at").unwrap();
        assert!(deleted_at.is_some());
    }
}