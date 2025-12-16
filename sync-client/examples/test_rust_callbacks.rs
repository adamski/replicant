use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use sync_client::{ClientDatabase, SyncEngine};
use tokio::time::sleep;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("warn").init();

    println!("🧪 Testing Rust event callbacks...");

    // Setup database
    std::fs::create_dir_all("databases")?;
    let db_file = "databases/callback_test.sqlite3";
    let db_url = format!("sqlite:{}?mode=rwc", db_file);

    let db = Arc::new(ClientDatabase::new(&db_url).await?);
    db.run_migrations().await?;

    // Get or create user
    let user_id = match db.get_user_id().await {
        Ok(id) => id,
        Err(_) => {
            let id = Uuid::new_v4();
            let client_id = Uuid::new_v4();
            setup_user(&db, id, client_id, "ws://nonexistent:8080/ws", "test-token").await?;
            id
        }
    };

    println!("👤 User ID: {}", user_id);

    // Try to connect to server (will fail, but we want to test offline mode)
    let sync_engine = match SyncEngine::new(
        &db_url,
        "ws://nonexistent:8080/ws",
        "test-user@example.com",
        "test-key",
        "test-secret",
    )
    .await
    {
        Ok(engine) => {
            println!("📡 Sync engine created successfully");
            // Note: Type-specific callbacks (DocumentEventCallback, SyncEventCallback, etc.)
            // are designed for C/C++ FFI. For Rust-native event handling, use process_events()
            // to poll for events or check engine.is_connected() for connection status.
            Some(Arc::new(engine))
        }
        Err(e) => {
            println!("⚠️ Offline mode: {}", e);
            None
        }
    };

    // Test local document operations
    println!("\n🔧 Testing document operations...");

    // Create some test documents
    for i in 1..=3 {
        let title = format!("Test Task {}", i);
        let content = json!({
            "title": title.clone(),
            "description": format!("This is test task number {}", i),
            "status": "pending",
            "priority": if i == 1 { "high" } else { "medium" },
            "tags": ["test", "demo"]
        });

        if let Some(engine) = &sync_engine {
            println!("  ➕ Creating document: {}", title);
            let mut full_content = content.clone();
            full_content["title"] = serde_json::json!(title);
            let doc = engine.create_document(full_content).await?;
            println!("     Created: {}", doc.id);
        } else {
            // Offline mode - create directly in database
            println!("  ➕ Creating document offline: {}", title);
            let mut full_content = content.clone();
            full_content["title"] = serde_json::json!(title);
            let doc = sync_core::models::Document {
                id: Uuid::new_v4(),
                user_id,
                content: full_content.clone(),
                sync_revision: 1,
                content_hash: None,
                title: full_content
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            };

            db.save_document(&doc).await?;
            println!("     Created: {}", doc.id);
        }

        // Give time for async operations
        sleep(Duration::from_millis(10)).await;
    }

    // Test event emission (events are queued but not processed without callbacks)
    println!("\n🧪 Testing event emission...");
    if let Some(engine) = &sync_engine {
        let events = engine.event_dispatcher();

        // Emit some test events
        let test_doc_id = Uuid::new_v4();
        let test_content = json!({"title": "Test Document", "test": "data"});

        println!("  📤 Emitting test events...");
        events.emit_document_created(&test_doc_id, &test_content);
        events.emit_sync_started();
        events.emit_sync_completed(42);
        events.emit_connection_succeeded("ws://test-server");

        // Process queued events (no callbacks registered, so just clears the queue)
        let processed = events.process_events()?;
        println!(
            "  🔄 Processed {} events (no callbacks registered)",
            processed
        );

        // Check connection status via polling API
        println!(
            "  🔗 Connection status: {}",
            if engine.is_connected() {
                "Connected"
            } else {
                "Disconnected"
            }
        );
    }

    // Wait a bit more
    sleep(Duration::from_millis(100)).await;

    println!("\n✅ Sync engine test completed!");
    println!("   Note: Type-specific callbacks (DocumentEventCallback, SyncEventCallback, etc.)");
    println!("   are designed for C/C++ FFI integration.");
    Ok(())
}

async fn setup_user(
    db: &ClientDatabase,
    user_id: Uuid,
    client_id: Uuid,
    server_url: &str,
    _token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("INSERT INTO user_config (user_id, client_id, server_url) VALUES (?1, ?2, ?3)")
        .bind(user_id.to_string())
        .bind(client_id.to_string())
        .bind(server_url)
        .execute(&db.pool)
        .await?;
    Ok(())
}
