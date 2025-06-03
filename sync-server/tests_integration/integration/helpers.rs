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
        // Retry logic to handle authentication race conditions
        let max_retries = 3;
        let mut last_error = None;
        
        for attempt in 0..max_retries {
            match self.create_test_client_attempt(user_id, token, attempt).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        // Exponential backoff with jitter
                        let delay_ms = 100u64 * (1 << attempt) + (attempt as u64 * 50);
                        tracing::debug!("Client creation attempt {} failed for user {}, retrying in {}ms", attempt + 1, user_id, delay_ms);
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| "Unknown error creating test client".into()))
    }
    
    async fn create_test_client_attempt(&self, user_id: Uuid, token: &str, attempt: usize) -> Result<SyncClient, Box<dyn std::error::Error + Send + Sync>> {
        // Use a block to ensure the permit is released after connection
        let (db_path, ws_url) = {
            // Acquire semaphore permit to limit concurrent connections
            let semaphore = get_connection_semaphore().await;
            let _permit = semaphore.acquire().await.unwrap();
            
            tracing::debug!("Creating test client for user {} (attempt {}, connection queued)", user_id, attempt + 1);
            
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
        
        // Start the sync engine with error handling
        engine.start().await.map_err(|e| {
            tracing::debug!("Engine start failed for user {} on attempt {}: {}", user_id, attempt + 1, e);
            e
        })?;
        
        // Adaptive delay based on attempt number
        let auth_delay = if attempt == 0 { 200u64 } else { 300u64 + (attempt as u64 * 100) };
        tokio::time::sleep(tokio::time::Duration::from_millis(auth_delay)).await;
        
        tracing::debug!("Test client created successfully for user {} on attempt {}", user_id, attempt + 1);
        
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
        
        tracing::debug!("Cleaning database for test isolation");
        
        // First, clean up change_events (no foreign key dependencies)
        sqlx::query("DELETE FROM change_events")
            .execute(&pool)
            .await
            .ok();
            
        // Then patches (depends on documents)
        sqlx::query("DELETE FROM patches")
            .execute(&pool)
            .await
            .ok();
            
        // Then documents (depends on users)
        sqlx::query("DELETE FROM documents")
            .execute(&pool)
            .await
            .ok();
            
        // Finally users - clean up ALL users to ensure complete isolation
        sqlx::query("DELETE FROM users")
            .execute(&pool)
            .await
            .ok();
            
        // Reset sequences to ensure consistent IDs across test runs
        sqlx::query("ALTER SEQUENCE IF EXISTS change_events_sequence_number_seq RESTART WITH 1")
            .execute(&pool)
            .await
            .ok();
            
        tracing::debug!("Database cleanup completed");
        pool.close().await;
    }
    
    pub async fn full_teardown_and_setup(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("Starting full teardown and setup for test isolation");
        
        // Step 1: Kill any existing sync-server processes
        self.kill_all_sync_servers().await;
        
        // Step 2: Drop and recreate the database
        self.recreate_database().await?;
        
        // Step 3: Start a fresh server instance
        self.start_fresh_server().await?;
        
        // Step 4: Wait for server to be ready
        self.wait_for_server().await?;
        
        tracing::info!("Full teardown and setup completed successfully");
        Ok(())
    }
    
    async fn kill_all_sync_servers(&self) {
        tracing::debug!("Killing all sync-server processes");
        
        // Kill processes by port
        if let Ok(output) = tokio::process::Command::new("lsof")
            .args(&["-ti", ":8080"])
            .output()
            .await 
        {
            let pids = String::from_utf8_lossy(&output.stdout);
            for pid in pids.lines() {
                if let Ok(pid_num) = pid.trim().parse::<u32>() {
                    tracing::debug!("Killing process on port 8080: {}", pid_num);
                    let _ = tokio::process::Command::new("kill")
                        .args(&["-9", &pid_num.to_string()])
                        .output()
                        .await;
                }
            }
        }
        
        // Kill processes by name
        let _ = tokio::process::Command::new("pkill")
            .args(&["-f", "sync-server"])
            .output()
            .await;
            
        // Give processes time to die
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    
    async fn recreate_database(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::debug!("Recreating database for fresh state");
        
        // Extract database name from URL
        let db_name = self.db_url.split('/').last().unwrap_or("sync_test_db_local");
        let base_url = self.db_url.rsplit_once('/').map(|(base, _)| base).unwrap_or(&self.db_url);
        
        // Connect to postgres database to drop/create our test database
        let postgres_url = format!("{}/postgres", base_url);
        let pool = sqlx::postgres::PgPool::connect(&postgres_url).await?;
        
        // Drop database (disconnect all clients first)
        sqlx::query(&format!("DROP DATABASE IF EXISTS {}", db_name))
            .execute(&pool)
            .await
            .ok(); // Ignore errors
            
        // Create database
        sqlx::query(&format!("CREATE DATABASE {}", db_name))
            .execute(&pool)
            .await?;
            
        pool.close().await;
        
        // Run migrations on the new database
        tracing::debug!("Running migrations on fresh database");
        
        // Find the project root directory (where Cargo.toml is)
        let current_dir = std::env::current_dir()?;
        let project_root = if current_dir.join("sync-server").exists() {
            current_dir
        } else if current_dir.parent().map(|p| p.join("sync-server").exists()).unwrap_or(false) {
            current_dir.parent().unwrap().to_path_buf()
        } else {
            return Err("Could not find project root directory".into());
        };
        
        let migration_result = tokio::process::Command::new("sqlx")
            .args(&["migrate", "run", "--source", "sync-server/migrations"])
            .current_dir(&project_root)
            .env("DATABASE_URL", &self.db_url)
            .output()
            .await?;
            
        if !migration_result.status.success() {
            let stderr = String::from_utf8_lossy(&migration_result.stderr);
            let stdout = String::from_utf8_lossy(&migration_result.stdout);
            return Err(format!("Migration failed: stdout: {}, stderr: {}", stdout, stderr).into());
        }
        
        Ok(())
    }
    
    async fn start_fresh_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        tracing::debug!("Starting fresh sync-server instance");
        
        // Find the project root directory
        let current_dir = std::env::current_dir()?;
        let project_root = if current_dir.join("sync-server").exists() {
            current_dir
        } else if current_dir.parent().map(|p| p.join("sync-server").exists()).unwrap_or(false) {
            current_dir.parent().unwrap().to_path_buf()
        } else {
            return Err("Could not find project root directory".into());
        };
        
        // Start the server in background
        tokio::process::Command::new("cargo")
            .args(&["run", "--bin", "sync-server"])
            .current_dir(&project_root)
            .env("DATABASE_URL", &self.db_url)
            .env("RUST_LOG", "info")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        
        // Give server time to start
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        
        Ok(())
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
            
            // Full teardown and setup for complete isolation
            ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
            
            // Run test
            let test_fn = $body;
            test_fn(ctx.clone()).await;
            
            // Note: Cleanup will happen at start of next test
        }
    };
}