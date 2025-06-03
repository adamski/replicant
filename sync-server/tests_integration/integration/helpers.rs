use std::time::Duration;
use uuid::Uuid;
use sync_client::SyncEngine as SyncClient;
use sync_core::models::Document;
use serde_json::json;
use tokio_tungstenite::{connect_async, WebSocketStream, MaybeTlsStream};
use std::sync::Arc;
use tokio::sync::Semaphore;

// Global semaphore to limit concurrent client connections in tests
static CLIENT_CONNECTION_SEMAPHORE: tokio::sync::OnceCell<Arc<Semaphore>> = tokio::sync::OnceCell::const_new();

async fn get_connection_semaphore() -> &'static Arc<Semaphore> {
    CLIENT_CONNECTION_SEMAPHORE.get_or_init(|| async {
        Arc::new(Semaphore::new(10)) // Allow max 10 concurrent client connections
    }).await
}

// Remove TestClient wrapper - we'll use SyncEngine directly

#[derive(Clone)]
pub struct TestContext {
    pub server_url: String,
    pub db_url: String,
}

impl TestContext {
    pub fn new() -> Self {
        Self {
            server_url: std::env::var("SYNC_SERVER_URL")
                .unwrap_or_else(|_| "ws://localhost:8080".to_string()),
            db_url: std::env::var("TEST_DATABASE_URL")
                .unwrap_or_else(|_| {
                    let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
                    format!("postgres://{}@localhost:5432/sync_test_db_local", user)
                }),
        }
    }
    
    pub async fn create_test_user(&self, email: &str) -> Result<(Uuid, String), Box<dyn std::error::Error + Send + Sync>> {
        // Register a new user via the server API
        let client = reqwest::Client::new();
        let server_base = self.server_url.replace("ws://", "http://").replace("wss://", "https://");
        
        let response = client
            .post(&format!("{}/register", server_base))
            .json(&serde_json::json!({
                "email": email,
                "password": "test-password"
            }))
            .send()
            .await?;
        
        if response.status().is_success() {
            let result: serde_json::Value = response.json().await?;
            let user_id = Uuid::parse_str(result["user_id"].as_str().unwrap())?;
            let token = result["auth_token"].as_str().unwrap().to_string();
            Ok((user_id, token))
        } else {
            Err(format!("Failed to create user: {}", response.status()).into())
        }
    }
    
    pub async fn create_test_client(&self, user_id: Uuid, token: &str) -> Result<SyncClient, Box<dyn std::error::Error + Send + Sync>> {
        // Use a block to ensure the permit is released after connection
        let (db_path, ws_url) = {
            // Acquire semaphore permit to limit concurrent connections
            let semaphore = get_connection_semaphore().await;
            let _permit = semaphore.acquire().await.unwrap();
            
            tracing::debug!("Creating test client for user {} (connection queued)", user_id);
            
            // Use an in-memory database but with a proper connection string
            let db_path = format!("file:memdb_{}?mode=memory&cache=shared", Uuid::new_v4());
            
            // Initialize the client database with the user
            let db = sync_client::ClientDatabase::new(&db_path).await?;
            db.run_migrations().await?;
            
            // Generate a unique client_id for this test client
            let client_id = Uuid::new_v4();
            
            // Set up user config in the client database with client_id
            sqlx::query(
                "INSERT INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)"
            )
            .bind(user_id.to_string())
            .bind(client_id.to_string())
            .bind(&self.server_url)
            .bind(token)
            .execute(&db.pool)
            .await?;
            
            // Create the sync engine with full WebSocket URL
            let ws_url = format!("{}/ws", self.server_url);
            
            // Permit is released here when _permit goes out of scope
            (db_path, ws_url)
        };
        
        // Create and start the engine without holding the semaphore
        let mut engine = SyncClient::new(
            &db_path,
            &ws_url,
            token
        ).await?;
        
        // Small delay to ensure connection is established
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // Start the sync engine
        engine.start().await?;
        
        // Longer delay to ensure authentication completes and avoid race condition
        // in demo user creation when multiple clients use the same user_id
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        
        tracing::debug!("Test client created successfully for user {}", user_id);
        
        Ok(engine)
    }
    
    pub async fn create_authenticated_websocket(&self, user_id: Uuid, token: &str) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        use sync_core::protocol::ClientMessage;
        
        let url = format!("{}/ws", self.server_url);
        
        let (mut ws_stream, _) = connect_async(&url)
            .await
            .expect("Failed to connect to WebSocket");
        
        // Generate a unique client_id for this test connection
        let client_id = Uuid::new_v4();
        
        // Send authentication message
        let auth_msg = ClientMessage::Authenticate {
            user_id,
            client_id,
            auth_token: token.to_string(),
        };
        let json_msg = serde_json::to_string(&auth_msg).unwrap();
        ws_stream.send(Message::Text(json_msg)).await.unwrap();
        
        // Wait for auth response
        use futures_util::StreamExt;
        if let Some(Ok(Message::Text(response))) = ws_stream.next().await {
            use sync_core::protocol::ServerMessage;
            let msg: ServerMessage = serde_json::from_str(&response).unwrap();
            match msg {
                ServerMessage::AuthSuccess { .. } => {
                    // Authentication successful
                }
                ServerMessage::AuthError { reason } => {
                    panic!("Authentication failed: {}", reason);
                }
                _ => panic!("Expected AuthSuccess or AuthError, got {:?}", msg),
            }
        }
            
        ws_stream
    }
    
    pub fn create_test_document(user_id: Uuid, title: &str) -> Document {
        Document {
            id: Uuid::new_v4(),
            user_id,
            title: title.to_string(),
            content: json!({
                "text": format!("Content for {}", title),
                "timestamp": chrono::Utc::now().to_rfc3339()
            }),
            revision_id: Document::initial_revision(&json!({
                "text": format!("Content for {}", title),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            version: 1,
            vector_clock: sync_core::models::VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }
    
    pub async fn wait_for_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let start = std::time::Instant::now();
        let max_wait = Duration::from_secs(30);
        
        loop {
            if start.elapsed() > max_wait {
                return Err("Server did not become ready in time".into());
            }
            
            // Try to connect
            match reqwest::get(&self.server_url.replace("ws://", "http://").replace("wss://", "https://"))
                .await {
                Ok(_) => return Ok(()),
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    
    pub async fn reset_server_state(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Reset server in-memory state via API (much faster than restart)
        let client = reqwest::Client::new();
        let server_base = self.server_url.replace("ws://", "http://").replace("wss://", "https://");
        
        let response = client
            .post(&format!("{}/test/reset", server_base))
            .send()
            .await?;
            
        if !response.status().is_success() {
            return Err(format!("Failed to reset server state: {}", response.status()).into());
        }
        
        Ok(())
    }
    
    pub async fn cleanup_database(&self) {
        // Connect to database and clean up test data
        let pool = sqlx::postgres::PgPool::connect(&self.db_url)
            .await
            .expect("Failed to connect to test database");
            
        // Aggressive cleanup - delete all test data to ensure isolation
        // Delete in order due to foreign key constraints
        sqlx::query("DELETE FROM patches")
            .execute(&pool)
            .await
            .ok();
            
        sqlx::query("DELETE FROM documents")
            .execute(&pool)
            .await
            .ok();
            
        sqlx::query("DELETE FROM users WHERE email LIKE 'demo_%'")
            .execute(&pool)
            .await
            .ok();
            
        // Also clean up any stray demo users (created by demo-token auth)
        sqlx::query("DELETE FROM users WHERE id::text = auth_token")
            .execute(&pool)
            .await
            .ok();
            
        pool.close().await;
    }
}

pub async fn assert_eventually<F, Fut>(f: F, timeout_secs: u64) 
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    
    while std::time::Instant::now() < deadline {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    panic!("Assertion did not become true within {} seconds", timeout_secs);
}

/// Test helper for verifying eventual convergence in distributed systems
pub async fn assert_all_clients_converge<F, Fut>(
    clients: &[&sync_client::SyncEngine],
    expected_count: usize,
    timeout_secs: u64,
    check_fn: F,
) where
    F: Fn(&sync_core::models::Document) -> Fut + Clone,
    Fut: std::future::Future<Output = bool>,
{
    let start = tokio::time::Instant::now();
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    
    loop {
        let mut all_converged = true;
        let mut client_states = Vec::new();
        
        // Check each client's state
        for (i, client) in clients.iter().enumerate() {
            let docs = client.get_all_documents().await
                .expect("Failed to get documents");
            
            client_states.push((i, docs.len()));
            
            // Check document count
            if docs.len() != expected_count {
                all_converged = false;
                continue;
            }
            
            // Apply custom check function to each document
            for doc in &docs {
                if !check_fn(doc).await {
                    all_converged = false;
                    break;
                }
            }
        }
        
        if all_converged {
            // Success! All clients have converged
            tracing::info!("All {} clients converged to {} documents in {:?}", 
                         clients.len(), expected_count, start.elapsed());
            return;
        }
        
        if start.elapsed() > timeout {
            // Log detailed state for debugging
            eprintln!("\n=== Convergence Timeout After {} seconds ===", timeout_secs);
            eprintln!("Expected: {} documents across {} clients", expected_count, clients.len());
            eprintln!("\nActual client states:");
            
            for (i, count) in &client_states {
                eprintln!("\nClient {}: {} documents", i, count);
                if let Ok(docs) = clients[*i].get_all_documents().await {
                    for doc in &docs {
                        eprintln!("  - {} | {} | rev: {}", 
                                 doc.id, doc.title, doc.revision_id);
                    }
                }
            }
            
            panic!("Clients did not converge within {} seconds", timeout_secs);
        }
        
        // Check every 100ms
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[macro_export]
macro_rules! integration_test {
    ($name:ident, $body:expr) => {
        #[tokio::test]
        async fn $name() {
            // Skip if not in integration test environment
            if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
                eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
                return;
            }
            
            let ctx = crate::integration::helpers::TestContext::new();
            ctx.wait_for_server().await.expect("Server not ready");
            
            // Reset server in-memory state (much faster than restart)
            ctx.reset_server_state().await.expect("Failed to reset server state");
            
            // Clean database before each test to ensure isolation
            ctx.cleanup_database().await;
            
            // Run test
            let test_fn = $body;
            test_fn(ctx.clone()).await;
            
            // Clean up after test as well  
            ctx.cleanup_database().await;
        }
    };
}