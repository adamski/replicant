pub mod database;
pub mod events;
pub mod offline_queue;
pub mod queries;
pub mod sync_engine;
pub mod websocket;

// C FFI module
pub mod ffi;

// C FFI test functions (debug builds only)
#[cfg(debug_assertions)]
pub mod ffi_test;

pub use database::ClientDatabase;
pub use sync_engine::SyncEngine;
pub use websocket::WebSocketClient;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::Row;
    use sync_core::models::Document;
    use uuid::Uuid;

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
                last_sync_at TIMESTAMP
            );

            CREATE TABLE documents (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                content JSON NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                deleted_at TIMESTAMP,
                local_changes JSON,
                sync_status TEXT DEFAULT 'synced',
                CHECK (sync_status IN ('synced', 'pending', 'conflict'))
            );
            "#,
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert test user
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO user_config (user_id, server_url) VALUES (?1, ?2)")
            .bind(user_id.to_string())
            .bind("ws://localhost:8080/ws")
            .execute(&db.pool)
            .await
            .unwrap();

        // Test get_user_id
        let retrieved_user_id = db.get_user_id().await.unwrap();
        assert_eq!(retrieved_user_id, user_id);

        // Create a test document
        let content = json!({
            "title": "Test Document",
            "text": "Hello, World!",
            "tags": ["test"]
        });
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            content_hash: None,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        // Save document
        db.save_document(&doc).await.unwrap();

        // Retrieve document
        let loaded_doc = db.get_document(&doc.id).await.unwrap();
        assert_eq!(loaded_doc.id, doc.id);
        // Title is now part of content JSON, so just compare the content
        assert_eq!(loaded_doc.content, doc.content);
    }

    #[test]
    fn test_offline_queue_message_extraction() {
        use crate::offline_queue::{extract_document_id, operation_type};
        use sync_core::protocol::ClientMessage;

        let doc_id = Uuid::new_v4();

        // Test create message
        let test_content = json!({"title": "Test"});
        let create_msg = ClientMessage::CreateDocument {
            document: Document {
                id: doc_id,
                user_id: Uuid::new_v4(),
                content: test_content.clone(),
                version: 1,
                content_hash: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            },
        };

        assert_eq!(extract_document_id(&create_msg), Some(doc_id));
        assert_eq!(operation_type(&create_msg), "create");

        // Test delete message
        let delete_msg = ClientMessage::DeleteDocument {
            document_id: doc_id,
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
                last_sync_at TIMESTAMP
            );

            CREATE TABLE documents (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                content JSON NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                deleted_at TIMESTAMP,
                local_changes JSON,
                sync_status TEXT DEFAULT 'synced',
                CHECK (sync_status IN ('synced', 'pending', 'conflict'))
            );
            "#,
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Insert test user
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO user_config (user_id, server_url) VALUES (?1, ?2)")
            .bind(user_id.to_string())
            .bind("ws://localhost:8080/ws")
            .execute(&db.pool)
            .await
            .unwrap();

        // Create a test document
        let content = json!({
            "title": "Test Document",
            "text": "Hello, World!"
        });
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            content_hash: None,
            version: 1,
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
