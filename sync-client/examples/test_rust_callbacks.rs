use sync_client::{ClientDatabase, SyncEngine};
use sync_client::events::{EventDispatcher, EventType};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Debug)]
struct TestState {
    events_received: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .init();

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

    // Create shared state for callback testing
    let state = Arc::new(Mutex::new(TestState {
        events_received: Vec::new(),
    }));

    // Try to connect to server (will fail, but we want to test offline callbacks)
    let sync_engine = match SyncEngine::new(&db_url, "ws://nonexistent:8080/ws", "test-token", "test-user@example.com").await {
        Ok(engine) => {
            // Register event callbacks
            let events = engine.event_dispatcher();
            let state_clone = state.clone();
            
            println!("📡 Registering Rust event callback...");
            events.register_rust_callback(
                Box::new(move |event_type, document_id, title, _content, error, numeric_data, boolean_data, _context| {
                    let mut test_state = state_clone.lock().unwrap();
                    
                    let event_desc = match event_type {
                        EventType::DocumentCreated => {
                            format!("📄 Document created: {}", title.unwrap_or("untitled"))
                        }
                        EventType::DocumentUpdated => {
                            format!("✏️ Document updated: {}", title.unwrap_or("untitled"))
                        }
                        EventType::DocumentDeleted => {
                            format!("🗑️ Document deleted: {}", document_id.unwrap_or("unknown"))
                        }
                        EventType::SyncStarted => {
                            "🔄 Sync started".to_string()
                        }
                        EventType::SyncCompleted => {
                            format!("✅ Sync completed: {} docs", numeric_data)
                        }
                        EventType::ConnectionSucceeded => {
                            "🔗 Connected to server".to_string()
                        }
                        EventType::ConnectionLost => {
                            "❌ Disconnected from server".to_string()
                        }
                        EventType::ConnectionAttempted => {
                            "🔄 Attempting to connect...".to_string()
                        }
                        EventType::SyncError => {
                            format!("🚨 Sync error: {}", error.unwrap_or("unknown"))
                        }
                        _ => format!("❓ Unknown event: {:?}", event_type)
                    };
                    
                    println!("  📥 Callback received: {}", event_desc);
                    test_state.events_received.push(event_desc);
                }),
                std::ptr::null_mut(),
                None,
            )?;

            Some(Arc::new(engine))
        }
        Err(e) => {
            println!("⚠️ Offline mode: {}", e);
            None
        }
    };

    // Test local document operations with event callbacks
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
                revision_id: sync_core::models::Document::initial_revision(&full_content),
                version: 1,
                vector_clock: sync_core::models::VectorClock::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                deleted_at: None,
            };
            
            db.save_document(&doc).await?;
            println!("     Created: {}", doc.id);
        }

        // Give time for events to process
        sleep(Duration::from_millis(10)).await;
        
        // Process any pending events
        if let Some(engine) = &sync_engine {
            let events = engine.event_dispatcher();
            let processed = events.process_events()?;
            if processed > 0 {
                println!("     🔄 Processed {} events", processed);
            }
        }
    }

    // Test direct event emission to verify callback mechanism
    println!("\n🧪 Testing direct event emission...");
    if let Some(engine) = &sync_engine {
        let events = engine.event_dispatcher();
        
        // Manually emit some events to test the callback mechanism
        let test_doc_id = Uuid::new_v4();
        let test_content = json!({"title": "Test Document", "test": "data"});

        println!("  📤 Emitting test events...");
        events.emit_document_created(&test_doc_id, &test_content);
        events.emit_sync_started();
        events.emit_sync_completed(42);
        events.emit_connection_succeeded("ws://test-server");
        
        // Process the emitted events
        let processed = events.process_events()?;
        println!("  🔄 Processed {} emitted events", processed);
    }

    // Wait a bit more and process final events
    sleep(Duration::from_millis(100)).await;
    if let Some(engine) = &sync_engine {
        let events = engine.event_dispatcher();
        let processed = events.process_events()?;
        if processed > 0 {
            println!("🔄 Final event processing: {} events", processed);
        }
    }

    // Show results
    println!("\n📊 Test Results:");
    let final_state = state.lock().unwrap();
    println!("   Events received: {}", final_state.events_received.len());
    for (i, event) in final_state.events_received.iter().enumerate() {
        println!("   {}. {}", i + 1, event);
    }

    println!("\n✅ Rust callback test completed!");
    Ok(())
}

async fn setup_user(
    db: &ClientDatabase,
    user_id: Uuid,
    client_id: Uuid,
    server_url: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id.to_string())
    .bind(client_id.to_string())
    .bind(server_url)
    .bind(token)
    .execute(&db.pool)
    .await?;
    Ok(())
}