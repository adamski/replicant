#[cfg(test)]
mod full_sync_tests {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use tokio::task::JoinHandle;
    use uuid::Uuid;
    use serde_json::json;
    use axum::Router;
    
    // Start test server in background
    async fn start_test_server() -> (JoinHandle<()>, String) {
        use sync_server::{database::ServerDatabase, auth::AuthState};
        use axum::{
            routing::{get, post},
            extract::{ws::WebSocketUpgrade, State},
            response::Response,
        };
        use tower_http::{cors::CorsLayer, trace::TraceLayer};
        
        // Use test database
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/sync_test_db".to_string());
        
        // Create test database (drop if exists)
        let temp_pool = sqlx::postgres::PgPoolOptions::new()
            .connect("postgres://postgres:postgres@localhost:5432/postgres")
            .await
            .expect("Failed to connect to postgres");
        
        let _ = sqlx::query("DROP DATABASE IF EXISTS sync_test_db")
            .execute(&temp_pool)
            .await;
        
        sqlx::query("CREATE DATABASE sync_test_db")
            .execute(&temp_pool)
            .await
            .expect("Failed to create test database");
        
        // Now connect to test database
        let db = Arc::new(
            ServerDatabase::new(&database_url)
                .await
                .expect("Failed to connect to test database")
        );
        
        // Run migrations
        sqlx::migrate!("sync-server/migrations")
            .run(&db.pool)
            .await
            .expect("Failed to run migrations");
        
        // Create test user
        let test_user_id = Uuid::new_v4();
        let test_token_hash = "test_hash"; // In real scenario, this would be hashed
        db.create_user("test@example.com", test_token_hash)
            .await
            .expect("Failed to create test user");
        
        // App state
        #[derive(Clone)]
        struct AppState {
            db: Arc<ServerDatabase>,
            auth: AuthState,
        }
        
        let app_state = Arc::new(AppState {
            db,
            auth: AuthState::new(),
        });
        
        // Create simple auth state for testing
        app_state.auth.create_session(test_user_id, "test-token".to_string());
        
        // Build router
        let app = Router::new()
            .route("/ws", get(|ws: WebSocketUpgrade, State(state): State<Arc<AppState>>| async move {
                ws.on_upgrade(move |socket| sync_server::websocket::handle_websocket(socket, state))
            }))
            .route("/health", get(|| async { "OK" }))
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http())
            .with_state(app_state);
        
        // Start server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind");
        
        let addr = listener.local_addr().unwrap();
        let server_url = format!("ws://{}/ws", addr);
        
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("Server failed");
        });
        
        // Give server time to start
        sleep(Duration::from_millis(100)).await;
        
        (handle, server_url)
    }
    
    #[tokio::test]
    #[ignore] // Requires PostgreSQL to be running
    async fn test_full_sync_cycle() {
        // Start test server
        let (_server_handle, server_url) = start_test_server().await;
        
        // Create client database
        let client_db = ":memory:";
        let client_database = sync_client::ClientDatabase::new(client_db)
            .await
            .expect("Failed to create client database");
        
        // Create schema
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
        .execute(&client_database.pool)
        .await
        .expect("Failed to create schema");
        
        // Insert user config
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_config (user_id, server_url, auth_token) VALUES (?1, ?2, ?3)"
        )
        .bind(user_id.to_string())
        .bind(&server_url)
        .bind("test-token")
        .execute(&client_database.pool)
        .await
        .expect("Failed to insert user config");
        
        // Create sync engine
        let sync_engine = sync_client::SyncEngine::new(
            client_db,
            &server_url,
            "test-token",
            "test-user@example.com"
        )
        .await
        .expect("Failed to create sync engine");
        
        // Create a document
        let doc = sync_engine.create_document(
            "Test Document".to_string(),
            json!({
                "content": "This is a test document",
                "tags": ["test", "integration"]
            })
        )
        .await
        .expect("Failed to create document");
        
        // Give time for sync
        sleep(Duration::from_millis(500)).await;
        
        // Update the document
        sync_engine.update_document(
            doc.id,
            json!({
                "content": "This is an updated test document",
                "tags": ["test", "integration", "updated"],
                "modified": true
            })
        )
        .await
        .expect("Failed to update document");
        
        // Give time for sync
        sleep(Duration::from_millis(500)).await;
        
        println!("Full sync test completed successfully!");
    }
}

// Make these modules accessible for tests
pub use sync_server;
pub use sync_client;