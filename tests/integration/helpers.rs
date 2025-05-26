use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;
use sync_client::SyncEngine as SyncClient;
use sync_core::models::Document;
use serde_json::json;
use tokio_tungstenite::{connect_async, WebSocketStream, MaybeTlsStream};
use futures_util::{StreamExt, SinkExt};
use tungstenite::Message;
use std::sync::Arc;

pub struct TestClient {
    engine: Arc<SyncClient>,
}

impl TestClient {
    pub async fn create_document(&self, doc: Document) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.create_document(doc.title, doc.content).await?;
        Ok(())
    }
    
    pub async fn update_document(&self, doc: &Document) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.update_document(doc.id, doc.content.clone()).await?;
        Ok(())
    }
    
    pub async fn delete_document(&self, id: &Uuid) -> Result<(), Box<dyn std::error::Error>> {
        self.engine.delete_document(*id).await?;
        Ok(())
    }
    
    pub async fn get_all_documents(&self) -> Result<Vec<Document>, Box<dyn std::error::Error>> {
        Ok(self.engine.get_all_documents().await?)
    }
    
    pub async fn sync(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Sync is handled automatically by SyncEngine
        Ok(())
    }
}

pub struct TestContext {
    pub server_url: String,
    pub db_url: String,
}

impl TestContext {
    pub fn new() -> Self {
        Self {
            server_url: std::env::var("SYNC_SERVER_URL")
                .unwrap_or_else(|_| "ws://localhost:8081".to_string()),
            db_url: std::env::var("TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5433/sync_test_db".to_string()),
        }
    }
    
    pub async fn create_test_client(&self, user_id: Uuid, token: &str) -> TestClient {
        let db_path = format!(":memory:{}:{}", user_id, Uuid::new_v4());
        let mut engine = SyncClient::new(
            &db_path,
            &self.server_url,
            token
        ).await.expect("Failed to create sync engine");
        
        engine.start().await.expect("Failed to start sync engine");
        
        TestClient { engine: std::sync::Arc::new(engine) }
    }
    
    pub async fn create_authenticated_websocket(&self, user_id: Uuid, token: &str) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
        let url = format!("{}/sync?user_id={}&token={}", 
            self.server_url.replace("ws://", "http://"),
            user_id,
            token
        );
        
        let (ws_stream, _) = connect_async(&url)
            .await
            .expect("Failed to connect to WebSocket");
            
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
            revision_id: Uuid::new_v4(),
            version: 1,
            vector_clock: sync_core::models::VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }
    
    pub async fn wait_for_server(&self) -> Result<(), Box<dyn std::error::Error>> {
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
    
    pub async fn cleanup_database(&self) {
        // Connect to database and clean up test data
        let pool = sqlx::postgres::PgPool::connect(&self.db_url)
            .await
            .expect("Failed to connect to test database");
            
        // Delete test data (be careful with WHERE clauses!)
        sqlx::query("DELETE FROM patches WHERE created_at > NOW() - INTERVAL '1 hour'")
            .execute(&pool)
            .await
            .ok();
            
        sqlx::query("DELETE FROM documents WHERE created_at > NOW() - INTERVAL '1 hour'")
            .execute(&pool)
            .await
            .ok();
            
        sqlx::query("DELETE FROM users WHERE email LIKE 'test_%'")
            .execute(&pool)
            .await
            .ok();
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
            
            let ctx = TestContext::new();
            ctx.wait_for_server().await.expect("Server not ready");
            
            // Run test
            $body(ctx).await;
        }
    };
}