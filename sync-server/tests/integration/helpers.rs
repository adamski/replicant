use anyhow::{Context, Result};
use hmac::{Hmac, Mac};
use libc::kill;
use serde_json::json;
use sha2::Sha256;
use std::sync::Arc;
use std::time::Duration;
use sync_client::SyncEngine as SyncClient;
use sync_core::models::Document;
use tokio::process::Child;
use tokio::sync::{Mutex, Semaphore};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

// Global semaphore to limit concurrent client connections in tests
static CLIENT_CONNECTION_SEMAPHORE: tokio::sync::OnceCell<Arc<Semaphore>> =
    tokio::sync::OnceCell::const_new();

// Required for cargo-llvm-cov to cover sync-server artifacts
const SERVER_BIN: &str = env!("CARGO_BIN_EXE_sync-server");
async fn get_connection_semaphore() -> &'static Arc<Semaphore> {
    CLIENT_CONNECTION_SEMAPHORE
        .get_or_init(|| async {
            Arc::new(Semaphore::new(10)) // Allow max 10 concurrent client connections
        })
        .await
}

// Remove TestClient wrapper - we'll use SyncEngine directly

#[derive(Clone)]
pub struct TestContext {
    pub server_url: String,
    pub db_url: String,
    pub server_process: Arc<Mutex<Option<Child>>>,
}

impl TestContext {
    pub fn new() -> Self {
        // Generate unique database name for this test using timestamp and random UUID
        let unique_db_name = format!(
            "sync_test_{}_{}",
            chrono::Utc::now().timestamp_micros(),
            Uuid::new_v4().to_string().replace("-", "").chars().take(8).collect::<String>()
        );

        let db_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
            format!("postgres://{}@localhost:5432/{}", user, unique_db_name)
        });

        // Use a random port between 9000-19000 for this test to avoid conflicts
        // Widened range from 1000 to 10000 ports to reduce collision probability
        // when running many tests in parallel
        let port = 9000 + (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() % 10000) as u16;

        let server_url = std::env::var("SYNC_SERVER_URL")
            .unwrap_or_else(|_| format!("ws://localhost:{}", port));

        Self {
            server_url,
            db_url,
            server_process: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn generate_test_credentials(&self, name: &str) -> Result<(String, String)> {
        // Connect to test database
        let pool = sqlx::postgres::PgPool::connect(&self.db_url)
            .await
            .context("Failed to connect to test database")?;

        // Generate credentials using AuthState's generate_api_credentials()
        use sync_server::auth::AuthState;
        let credentials = AuthState::generate_api_credentials();

        // Save to api_credentials table
        sqlx::query("INSERT INTO api_credentials (api_key, secret, name) VALUES ($1, $2, $3)")
            .bind(&credentials.api_key)
            .bind(&credentials.secret)
            .bind(name)
            .execute(&pool)
            .await
            .context("Failed to save test credentials")?;

        pool.close().await;

        Ok((credentials.api_key, credentials.secret))
    }

    pub async fn create_test_user(&self, email: &str) -> Result<Uuid> {
        // Create user directly in database (since REST endpoint was removed)
        // WebSocket auto-creation is the production flow, but tests need user_id upfront
        let pool = sqlx::postgres::PgPool::connect(&self.db_url)
            .await
            .context("Failed to connect to test database")?;

        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email) VALUES ($1, $2)")
            .bind(user_id)
            .bind(email)
            .execute(&pool)
            .await
            .context("Failed to insert test user")?;

        pool.close().await;
        Ok(user_id)
    }

    pub async fn create_test_client(
        &self,
        email: &str,
        user_id: Uuid,
        api_key: &str,
        api_secret: &str,
    ) -> Result<SyncClient> {
        // Retry logic to handle authentication race conditions
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 0..max_retries {
            match self
                .create_test_client_attempt(email, user_id, api_key, api_secret, attempt)
                .await
            {
                Ok(client) => return Ok(client),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        // Exponential backoff with jitter
                        let delay_ms = 100u64 * (1 << attempt) + (attempt as u64 * 50);
                        tracing::debug!(
                            "Client creation attempt {} failed for user {}, retrying in {}ms",
                            attempt + 1,
                            user_id,
                            delay_ms
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error creating test client")))
    }

    async fn create_test_client_attempt(
        &self,
        email: &str,
        user_id: Uuid,
        api_key: &str,
        api_secret: &str,
        attempt: usize,
    ) -> Result<SyncClient> {
        // Use a block to ensure the permit is released after connection
        let (db_path, ws_url, _db) = {
            // Acquire semaphore permit to limit concurrent connections
            let semaphore = get_connection_semaphore().await;
            let _permit = semaphore.acquire().await.unwrap();

            tracing::debug!(
                "Creating test client for user {} (attempt {}, connection queued)",
                user_id,
                attempt + 1
            );

            // Use an in-memory database but with a proper connection string
            let db_path = format!("file:memdb_{}?mode=memory&cache=shared", Uuid::new_v4());

            // Initialize the client database with the user
            let db = sync_client::ClientDatabase::new(&db_path).await?;
            db.run_migrations().await?;

            // Generate a unique client_id for this test client
            let client_id = Uuid::new_v4();
            // Set up user config in the client database with client_id
            // Note: API credentials are NOT stored in database - they're passed to SyncEngine
            sqlx::query(
                "INSERT INTO user_config (user_id, client_id, server_url) VALUES (?1, ?2, ?3)",
            )
            .bind(user_id.to_string())
            .bind(client_id.to_string())
            .bind(&self.server_url)
            .execute(&db.pool)
            .await?;
            // Create the sync engine with full WebSocket URL
            let ws_url = format!("{}/ws", self.server_url);

            // Permit is released here when _permit goes out of scope
            // we need to keep our in memory db alive to create our SyncEngine
            // with the same user_config as above
            (db_path, ws_url, db)
        };

        // Create the engine without holding the semaphore
        // Connection starts automatically, no need to call start()
        let engine = SyncClient::new(
            &db_path, &ws_url, email, api_key,    // rpa_ prefixed key
            api_secret, // rps_ prefixed secret
        )
        .await?;

        // Small delay to ensure connection is established
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Adaptive delay based on attempt number
        let auth_delay = if attempt == 0 {
            200u64
        } else {
            300u64 + (attempt as u64 * 100)
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(auth_delay)).await;

        tracing::debug!(
            "Test client created successfully for user {} on attempt {}",
            user_id,
            attempt + 1
        );

        Ok(engine)
    }

    pub async fn create_authenticated_websocket(
        &self,
        email: &str,
        _token: &str,
    ) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
        use futures_util::SinkExt;
        use sync_core::protocol::ClientMessage;
        use tokio_tungstenite::tungstenite::Message;
        let (api_key, api_secret) = self
            .generate_test_credentials("test-bob")
            .await
            .expect("Failed to generate credentials");
        let url = format!("{}/ws", self.server_url);
        let (mut ws, _) = connect_async(&url)
            .await
            .expect("Failed to connect to WebSocket");
        let now = chrono::Utc::now().timestamp();
        let signature = create_hmac_signature(&api_secret, now, email, &api_key, "");
        // Send authenticate message
        let client_id = Uuid::new_v4();
        let auth_msg = ClientMessage::Authenticate {
            email: email.to_string(),
            client_id,
            api_key: Some(api_key.clone()),
            signature: Some(signature),
            timestamp: Some(now),
        };
        let json_msg = serde_json::to_string(&auth_msg).unwrap();
        ws.send(Message::Text(json_msg)).await.unwrap();

        // Wait for auth response
        use futures_util::StreamExt;
        if let Some(Ok(Message::Text(response))) = ws.next().await {
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

        ws
    }

    #[allow(dead_code)]
    pub fn create_test_document(user_id: Uuid, title: &str) -> Document {
        let content = json!({
            "title": title,
            "text": format!("Content for {}", title),
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            revision_id: Document::initial_revision(&content),
            version: 1,
            version_vector: sync_core::models::VersionVector::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }

    pub async fn wait_for_server(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let max_wait = Duration::from_secs(30);

        loop {
            if start.elapsed() > max_wait {
                anyhow::bail!("Server did not become ready in time");
            }

            // Try to connect
            match reqwest::get(
                &self
                    .server_url
                    .replace("ws://", "http://")
                    .replace("wss://", "https://"),
            )
            .await
            {
                Ok(_) => return Ok(()),
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    #[allow(dead_code)]
    pub async fn reset_server_state(&self) -> Result<()> {
        // Reset server in-memory state via API (much faster than restart)
        let client = reqwest::Client::new();
        let server_base = self
            .server_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");

        let response = client
            .post(&format!("{}/test/reset", server_base))
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to reset server state: {}", response.status());
        }

        Ok(())
    }

    #[allow(dead_code)]
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
        sqlx::query("DELETE FROM patches").execute(&pool).await.ok();

        // Then documents (depends on users)
        sqlx::query("DELETE FROM documents")
            .execute(&pool)
            .await
            .ok();

        // Finally users - clean up ALL users to ensure complete isolation
        sqlx::query("DELETE FROM users").execute(&pool).await.ok();

        // Reset sequences to ensure consistent IDs across test runs
        sqlx::query("ALTER SEQUENCE IF EXISTS change_events_sequence_number_seq RESTART WITH 1")
            .execute(&pool)
            .await
            .ok();

        tracing::debug!("Database cleanup completed");
        pool.close().await;
    }

    pub async fn full_teardown_and_setup(&mut self) -> Result<()> {
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

    pub async fn kill_all_sync_servers(&mut self) {
        let mut l = self.server_process.lock().await;
        if let Some(mut child) = l.take() {
            if let Some(pid) = child.id() {
                let _ = unsafe { kill(pid as i32, libc::SIGINT) };
                child.wait().await.unwrap();
                tracing::info!("Killed server process: {:?}", pid);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    async fn recreate_database(&self) -> Result<()> {
        tracing::debug!("Recreating database for fresh state");

        // Extract database name from URL
        let db_name = self
            .db_url
            .split('/')
            .last()
            .unwrap_or("sync_test_db_local");
        let base_url = self
            .db_url
            .rsplit_once('/')
            .map(|(base, _)| base)
            .unwrap_or(&self.db_url);

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
        let project_root = std::env::current_dir()?.join("../");

        let migration_result = tokio::process::Command::new("sqlx")
            .args(&["migrate", "run", "--source", "sync-server/migrations"])
            .current_dir(&project_root)
            .env("DATABASE_URL", &self.db_url)
            .output()
            .await?;

        if !migration_result.status.success() {
            let stderr = String::from_utf8_lossy(&migration_result.stderr);
            let stdout = String::from_utf8_lossy(&migration_result.stdout);
            anyhow::bail!("Migration failed: stdout: {}, stderr: {}", stdout, stderr);
        }

        Ok(())
    }

    pub async fn start_fresh_server(&mut self) -> Result<()> {
        tracing::debug!("Starting fresh sync-server instance");

        // Find the project root directory
        let project_root = std::env::current_dir()?.join("../");

        // Extract port from server_url
        let bind_address = if let Some(port_str) = self.server_url.split(':').last() {
            format!("0.0.0.0:{}", port_str)
        } else {
            "0.0.0.0:8080".to_string()
        };

        // Start the server in background
        // Note: Using null() for stdout/stderr to ensure proper process cleanup
        tracing::debug!("Starting server on {} with database {}", bind_address, self.db_url);
        let server = tokio::process::Command::new(SERVER_BIN)
            .current_dir(&project_root)
            .env("DATABASE_URL", &self.db_url)
            .env("BIND_ADDRESS", &bind_address)
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let mut w = self.server_process.lock().await;
        *w = Some(server);

        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        Ok(())
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let server_process = self.server_process.clone();
        let db_url = self.db_url.clone();

        // handle any dangling process and cleanup database
        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            // Kill server process - FIXED: now kills when server IS stored, not when it's None
            let mut l = server_process.lock().await;
            if let Some(mut child) = l.take() {
                if let Some(pid) = child.id() {
                    tracing::debug!("Cleaning up server process with PID: {}", pid);
                    // Use SIGTERM for graceful shutdown, then force kill if needed
                    let _ = unsafe { kill(pid as i32, libc::SIGTERM) };

                    // Give it 1 second to shut down gracefully
                    let timeout = tokio::time::Duration::from_secs(1);
                    match tokio::time::timeout(timeout, child.wait()).await {
                        Ok(_) => {
                            tracing::debug!("Server process {} terminated gracefully", pid);
                        }
                        Err(_) => {
                            // Force kill if it didn't shut down
                            tracing::warn!("Server process {} didn't terminate, force killing", pid);
                            let _ = unsafe { kill(pid as i32, libc::SIGKILL) };
                            let _ = child.wait().await;
                        }
                    }
                }
            }

            // Drop the unique test database
            if let Some(db_name) = db_url.split('/').last() {
                if db_name.starts_with("sync_test_") {
                    let base_url = db_url.rsplit_once('/').map(|(base, _)| base).unwrap_or(&db_url);
                    let postgres_url = format!("{}/postgres", base_url);

                    if let Ok(pool) = sqlx::postgres::PgPool::connect(&postgres_url).await {
                        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {}", db_name))
                            .execute(&pool)
                            .await;
                        pool.close().await;
                        tracing::debug!("Dropped test database: {}", db_name);
                    }
                }
            }
        });
    }
}

#[allow(dead_code)]
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

    panic!(
        "Assertion did not become true within {} seconds",
        timeout_secs
    );
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
            let docs = client
                .get_all_documents()
                .await
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
            tracing::info!(
                "All {} clients converged to {} documents in {:?}",
                clients.len(),
                expected_count,
                start.elapsed()
            );
            return;
        }

        if start.elapsed() > timeout {
            // Log detailed state for debugging
            eprintln!(
                "\n=== Convergence Timeout After {} seconds ===",
                timeout_secs
            );
            eprintln!(
                "Expected: {} documents across {} clients",
                expected_count,
                clients.len()
            );
            eprintln!("\nActual client states:");

            for (i, count) in &client_states {
                eprintln!("\nClient {}: {} documents", i, count);
                if let Ok(docs) = clients[*i].get_all_documents().await {
                    for doc in &docs {
                        eprintln!(
                            "  - {} | {} | rev: {}",
                            doc.id,
                            doc.title_or_default(),
                            doc.revision_id
                        );
                    }
                }
            }

            panic!("Clients did not converge within {} seconds", timeout_secs);
        }

        // Check every 100ms
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub fn create_hmac_signature(
    secret: &str,
    timestamp: i64,
    email: &str,
    api_key: &str,
    body: &str,
) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");

    let message = format!("{}.{}.{}.{}", timestamp, email, api_key, body);
    mac.update(message.as_bytes());

    hex::encode(mac.finalize().into_bytes())
}

#[macro_export]
macro_rules! integration_test {
    ($name:ident, $body:expr, $online:expr) => {
        #[tokio::test]
        async fn $name() {
            // Skip if not in integration test environment
            if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
                eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
                return;
            }

            let mut ctx = crate::integration::helpers::TestContext::new();
            // Full teardown and setup for complete isolation

            match $online {
                true => ctx
                    .full_teardown_and_setup()
                    .await
                    .expect("Failed to setup test environment"),
                false => (),
            }

            // Run test
            let test_fn = $body;
            test_fn(ctx.clone()).await;
            // Kill sync-server subprocess
            ctx.kill_all_sync_servers().await;
        }
    };
}
