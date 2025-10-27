#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use uuid::Uuid;
    use serde_json::json;
    use sync_core::models::{Document, VectorClock};
    use sync_client::{SyncEngine, ClientDatabase};
    use sync_server::{database::ServerDatabase, auth::AuthState};
    
    // Helper to create test databases
    async fn setup_test_env() -> (String, String, String) {
        // Generate unique test database names
        let test_id = Uuid::new_v4().to_string().replace("-", "");
        let client_db = format!(":memory:");  // SQLite in-memory
        let server_db = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postgres".to_string());
        let server_url = "ws://127.0.0.1:8081/ws";
        
        (client_db, server_db, server_url.to_string())
    }
    
    #[tokio::test]
    async fn test_basic_sync_flow() {
        // This test requires a running PostgreSQL instance
        let (client_db, server_db, server_url) = setup_test_env().await;
        
        // Create server database and run migrations
        let app_namespace_id = "com.example.sync-task-list".to_string();
        let server_database = Arc::new(
            ServerDatabase::new(&server_db, app_namespace_id)
                .await
                .expect("Failed to connect to server database")
        );
        
        // Note: In a real test, we'd need to:
        // 1. Start the server in a background task
        // 2. Create a test user
        // 3. Run the actual sync operations
        
        // For now, let's test the components individually
        
        // Test 1: Client database operations
        let client_database = ClientDatabase::new(&client_db)
            .await
            .expect("Failed to create client database");
        
        // Create the schema manually for SQLite in-memory
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
            
            CREATE TABLE sync_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                document_id TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                patch JSON,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                retry_count INTEGER DEFAULT 0,
                FOREIGN KEY (document_id) REFERENCES documents(id),
                CHECK (operation_type IN ('create', 'update', 'delete'))
            );
            "#
        )
        .execute(&client_database.pool)
        .await
        .expect("Failed to create schema");
        
        // This test uses direct database operations, not the helper functions
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_config (user_id, server_url) VALUES (?1, ?2)"
        )
        .bind(user_id.to_string())
        .bind(&server_url)
        .execute(&client_database.pool)
        .await
        .expect("Failed to insert user config");
        
        // Test creating and saving a document
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title: "Test Document".to_string(),
            content: json!({
                "text": "Hello, World!"
            }),
            revision_id: Uuid::new_v4(),
            version: 1,
            vector_clock: VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        client_database.save_document(&doc)
            .await
            .expect("Failed to save document");
        
        // Verify document was saved
        let loaded_doc = client_database.get_document(&doc.id)
            .await
            .expect("Failed to load document");
        
        assert_eq!(loaded_doc.id, doc.id);
        assert_eq!(loaded_doc.title, doc.title);
    }
    
    #[tokio::test]
    async fn test_vector_clock_sync() {
        let mut vc1 = VectorClock::new();
        let mut vc2 = VectorClock::new();
        
        vc1.increment("client1");
        vc2.increment("client2");
        
        // Test concurrent detection
        assert!(vc1.is_concurrent(&vc2));
        
        // Merge clocks
        vc1.merge(&vc2);
        
        // After merge, vc1 should have both entries
        assert_eq!(vc1.0.get("client1"), Some(&1));
        assert_eq!(vc1.0.get("client2"), Some(&1));
        
        // And should no longer be concurrent with vc2
        assert!(!vc1.is_concurrent(&vc2));
    }
    
    #[tokio::test] 
    async fn test_json_patch_sync() {
        use sync_core::patches::{create_patch, apply_patch};
        
        let original = json!({
            "name": "John",
            "age": 30,
            "items": ["apple", "banana"]
        });
        
        let modified = json!({
            "name": "John",
            "age": 31,
            "items": ["apple", "banana", "orange"],
            "city": "New York"
        });
        
        // Create patch
        let patch = create_patch(&original, &modified)
            .expect("Failed to create patch");
        
        // Apply patch to original
        let mut result = original.clone();
        apply_patch(&mut result, &patch)
            .expect("Failed to apply patch");
        
        // Verify result matches modified
        assert_eq!(result, modified);
    }
}

// Add this to make the client database pool accessible for tests
impl sync_client::database::ClientDatabase {
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}