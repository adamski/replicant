use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use serde_json::json;
use uuid::Uuid;
use crate::integration::helpers::TestContext;
use sync_client::SyncEngine;

#[derive(Debug, Clone)]
struct EventLog {
    created: Vec<(Uuid, String)>,
    updated: Vec<(Uuid, String)>,
    deleted: Vec<Uuid>,
    sync_started: usize,
    sync_completed: usize,
    conflicts: Vec<Uuid>,
    errors: Vec<String>,
}

impl EventLog {
    fn new() -> Self {
        Self {
            created: Vec::new(),
            updated: Vec::new(),
            deleted: Vec::new(),
            sync_started: 0,
            sync_completed: 0,
            conflicts: Vec::new(),
            errors: Vec::new(),
        }
    }
}

async fn create_client_with_event_tracking(
    ctx: &TestContext,
    user_id: Uuid,
    email: &str,
    api_key: &str,
    api_secret: &str,
) -> Result<(SyncEngine, Arc<Mutex<EventLog>>), Box<dyn std::error::Error + Send + Sync>> {
    // Create unique database for this client
    let db_path = format!("file:memdb_{}?mode=memory&cache=shared", Uuid::new_v4());

    // Initialize the client database
    let db = sync_client::ClientDatabase::new(&db_path).await?;
    db.run_migrations().await?;

    // Set up user config
    let client_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO user_config (user_id, client_id, server_url) VALUES (?1, ?2, ?3)"
    )
    .bind(user_id.to_string())
    .bind(client_id.to_string())
    .bind(&format!("{}/ws", ctx.server_url))
    .execute(&db.pool)
    .await?;


    // Create sync engine with proper HMAC credentials
    let engine = SyncEngine::new(&db_path, &format!("{}/ws", ctx.server_url), email, api_key, api_secret).await?;

    // Give it time to connect and perform initial sync
    sleep(Duration::from_millis(500)).await;
    
    // Set up event tracking
    let event_log = Arc::new(Mutex::new(EventLog::new()));
    let event_log_clone = event_log.clone();
    
    // Register event callbacks using the Rust callback API
    let dispatcher = engine.event_dispatcher();
    
    dispatcher.register_rust_callback(
        Box::new(move |event_type, document_id, title, _content, error, numeric_data, _boolean_data, _context| {
            use sync_client::events::EventType;
            let mut events = event_log_clone.lock().unwrap();
            
            match event_type {
                EventType::DocumentCreated => {
                    if let (Some(doc_id_str), Some(title_str)) = (document_id, title) {
                        if let Ok(doc_id) = Uuid::parse_str(doc_id_str) {
                            events.created.push((doc_id, title_str.to_string()));
                            tracing::info!("Event: Document created - {} ({})", title_str, doc_id);
                        }
                    }
                }
                EventType::DocumentUpdated => {
                    if let (Some(doc_id_str), Some(title_str)) = (document_id, title) {
                        if let Ok(doc_id) = Uuid::parse_str(doc_id_str) {
                            events.updated.push((doc_id, title_str.to_string()));
                            tracing::info!("Event: Document updated - {} ({})", title_str, doc_id);
                        }
                    }
                }
                EventType::DocumentDeleted => {
                    if let Some(doc_id_str) = document_id {
                        if let Ok(doc_id) = Uuid::parse_str(doc_id_str) {
                            events.deleted.push(doc_id);
                            tracing::info!("Event: Document deleted - {}", doc_id);
                        }
                    }
                }
                EventType::SyncStarted => {
                    events.sync_started += 1;
                    tracing::info!("Event: Sync started");
                }
                EventType::SyncCompleted => {
                    events.sync_completed += 1;
                    tracing::info!("Event: Sync completed - {} docs", numeric_data);
                }
                EventType::ConflictDetected => {
                    if let Some(doc_id_str) = document_id {
                        if let Ok(doc_id) = Uuid::parse_str(doc_id_str) {
                            events.conflicts.push(doc_id);
                            tracing::info!("Event: Conflict detected - {}", doc_id);
                        }
                    }
                }
                EventType::SyncError => {
                    if let Some(error_str) = error {
                        events.errors.push(error_str.to_string());
                        tracing::info!("Event: Sync error - {}", error_str);
                    }
                }
                _ => {}
            }
        }),
        std::ptr::null_mut(),
        None,
    )?;
    
    // Process initial events multiple times
    for _ in 0..5 {
        let _ = dispatcher.process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    Ok((engine, event_log))
}

#[tokio::test]
async fn test_offline_changes_sync_on_reconnect() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user
    let email = "offline-sync-test@example.com";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-offline-sync").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");
    
    // Create two clients with event tracking (this function manages its own setup)
    tracing::info!("Creating client 1...");
    let (client1, events1) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await
        .expect("Failed to create client 1");

    tracing::info!("Creating client 2...");
    let (client2, events2) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await
        .expect("Failed to create client 2");
    
    // Process events
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify initial sync completed for both clients
    sleep(Duration::from_millis(500)).await;
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        assert!(e1.sync_completed > 0, "Client 1 should have completed initial sync");
        assert!(e2.sync_completed > 0, "Client 2 should have completed initial sync");
    }
    
    // Create a document on client 1
    tracing::info!("Creating document on client 1...");
    let doc = client1.create_document(
        json!({ "title": "Task 1", "status": "pending", "description": "Created while online" })
    ).await.expect("Failed to create document");
    
    // Process events and wait for sync
    for _ in 0..10 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify both clients see the document and received events
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        
        assert_eq!(e1.created.len(), 1, "Client 1 should have 1 created event");
        assert_eq!(e1.created[0].0, doc.id);
        
        assert_eq!(e2.created.len(), 1, "Client 2 should have 1 created event");
        assert_eq!(e2.created[0].0, doc.id);
    }
    
    // Now simulate server going offline by killing it
    tracing::info!("Simulating server offline...");
    ctx.kill_all_sync_servers().await;
    
    // Give clients time to detect disconnection
    sleep(Duration::from_millis(1000)).await;
    
    // Make changes while offline
    tracing::info!("Making offline changes...");
    
    // Client 1: Update existing document
    let update_result = client1.update_document(
        doc.id,
        json!({ 
            "status": "in_progress", 
            "description": "Updated while offline",
            "offline_update": true 
        })
    ).await;
    assert!(update_result.is_ok(), "Should be able to update while offline");
    
    // Client 1: Create new document
    let offline_doc = client1.create_document(
        json!({ "title": "Offline Task", "created_offline": true, "client": "1" })
    ).await.expect("Should create document while offline");

    // Client 2: Create a different document while offline
    let offline_doc2 = client2.create_document(
        json!({ "title": "Client 2 Offline Task", "created_offline": true, "client": "2" })
    ).await.expect("Should create document while offline");
    
    // Process events
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify offline events were recorded locally
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        
        assert_eq!(e1.updated.len(), 1, "Client 1 should have update event");
        assert_eq!(e1.created.len(), 2, "Client 1 should have 2 created events");
        assert_eq!(e2.created.len(), 2, "Client 2 should have 2 created events");
    }
    
    // Restart server
    tracing::info!("Restarting server...");
    ctx.start_fresh_server().await.expect("Failed to start server");
    ctx.wait_for_server().await.expect("Server didn't start");
    
    // Give clients time to reconnect and sync
    tracing::info!("Waiting for reconnection and sync...");
    sleep(Duration::from_millis(3000)).await;
    
    // Process events multiple times to ensure all events are handled
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify all documents are synced to both clients
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client 1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client 2");
    
    assert_eq!(docs1.len(), 3, "Client 1 should have all 3 documents");
    assert_eq!(docs2.len(), 3, "Client 2 should have all 3 documents");
    
    // Verify the update was synced
    let updated_doc1 = docs1.iter().find(|d| d.id == doc.id).expect("Should find updated doc");
    let updated_doc2 = docs2.iter().find(|d| d.id == doc.id).expect("Should find updated doc");
    
    assert_eq!(updated_doc1.content["status"], "in_progress");
    assert_eq!(updated_doc2.content["status"], "in_progress");
    assert_eq!(updated_doc1.content["offline_update"], true);
    assert_eq!(updated_doc2.content["offline_update"], true);
    
    // Verify both offline-created documents are present on both clients
    assert!(docs1.iter().any(|d| d.id == offline_doc.id), "Client 1's offline doc should be on client 1");
    assert!(docs2.iter().any(|d| d.id == offline_doc.id), "Client 1's offline doc should be on client 2");
    assert!(docs1.iter().any(|d| d.id == offline_doc2.id), "Client 2's offline doc should be on client 1");
    assert!(docs2.iter().any(|d| d.id == offline_doc2.id), "Client 2's offline doc should be on client 2");
    
    // Verify event counts after reconnection
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        
        // Client 1 should have received client 2's offline document
        assert!(e1.created.iter().any(|(id, _)| *id == offline_doc2.id), 
            "Client 1 should have received create event for client 2's offline doc");
        
        // Client 2 should have received client 1's update and new document
        assert!(e2.updated.iter().any(|(id, _)| *id == doc.id),
            "Client 2 should have received update event");
        assert!(e2.created.iter().any(|(id, _)| *id == offline_doc.id),
            "Client 2 should have received create event for client 1's offline doc");
    }
    
    tracing::info!("✓ Offline changes successfully synced after reconnection");
}

#[tokio::test]
async fn test_task_list_scenario_with_events() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user (simulating shared identity like alice@example.com)
    let email = "alice@tasks.com";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-alice-tasks").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");
    
    // Create three clients simulating three devices
    tracing::info!("Creating 3 clients for Alice...");
    let (client1, events1) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await.expect("Failed to create client 1");
    let (client2, events2) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await.expect("Failed to create client 2");
    let (client3, events3) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await.expect("Failed to create client 3");
    
    // Process initial events
    for _ in 0..3 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        let _ = client3.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Simulate task list operations
    tracing::info!("Simulating task list operations...");
    
    // Client 1: Create some tasks
    let task1 = client1.create_document(
        json!({
            "title": "Buy groceries",
            "status": "pending",
            "priority": "high",
            "tags": ["shopping", "urgent"]
        })
    ).await.expect("Failed to create task 1");

    let task2 = client1.create_document(
        json!({
            "title": "Review PRs",
            "status": "pending",
            "priority": "medium",
            "tags": ["work", "code-review"]
        })
    ).await.expect("Failed to create task 2");
    
    // Process events and wait for sync
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        let _ = client3.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify all clients received create events
    sleep(Duration::from_millis(500)).await;
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        let e3 = events3.lock().unwrap();
        
        assert_eq!(e1.created.len(), 2, "Client 1 created 2 tasks");
        assert_eq!(e2.created.len(), 2, "Client 2 should see 2 created tasks");
        assert_eq!(e3.created.len(), 2, "Client 3 should see 2 created tasks");
    }
    
    // Client 2: Toggle task status (mark as completed)
    tracing::info!("Client 2 marking task as completed...");
    client2.update_document(
        task1.id,
        json!({
            "status": "completed",
            "priority": "high",
            "tags": ["shopping", "urgent"],
            "completed_at": chrono::Utc::now().to_rfc3339()
        })
    ).await.expect("Failed to update task");
    
    // Process events
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        let _ = client3.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }

    // Verify update events
    sleep(Duration::from_millis(500)).await;
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        let e3 = events3.lock().unwrap();
        
        assert!(e1.updated.iter().any(|(id, _)| *id == task1.id), "Client 1 should see update");
        assert!(e2.updated.iter().any(|(id, _)| *id == task1.id), "Client 2 should see update");
        assert!(e3.updated.iter().any(|(id, _)| *id == task1.id), "Client 3 should see update");
    }
    
    // Client 3: Delete a task
    tracing::info!("Client 3 deleting task...");
    client3.delete_document(task2.id).await.expect("Failed to delete task");
    
    // Process events
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        let _ = client3.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }

    // Verify delete events within 500ms
    sleep(Duration::from_millis(500)).await;
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        let e3 = events3.lock().unwrap();
        
        assert!(e1.deleted.contains(&task2.id), "Client 1 should see deletion");
        assert!(e2.deleted.contains(&task2.id), "Client 2 should see deletion");
        assert!(e3.deleted.contains(&task2.id), "Client 3 should see deletion");
    }
    
    // Verify final state consistency
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    let docs3 = client3.get_all_documents().await.unwrap();
    
    assert_eq!(docs1.len(), 1, "Should have 1 task remaining");
    assert_eq!(docs2.len(), 1, "Should have 1 task remaining");
    assert_eq!(docs3.len(), 1, "Should have 1 task remaining");
    
    // Verify the remaining task is completed
    assert_eq!(docs1[0].content["status"], "completed");
    assert_eq!(docs2[0].content["status"], "completed");
    assert_eq!(docs3[0].content["status"], "completed");
    
    tracing::info!("✓ Task list scenario completed successfully with all events properly delivered");
}

#[tokio::test]
async fn test_rapid_updates_with_event_callbacks() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create test user
    let email = "rapid-test@example.com";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-rapid").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");
    
    // Create two clients
    let (client1, events1) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await.expect("Failed to create client 1");
    let (client2, events2) = create_client_with_event_tracking(&ctx, user_id, email, &api_key, &api_secret)
        .await.expect("Failed to create client 2");
    
    // Process initial events
    let _ = client1.event_dispatcher().process_events();
    let _ = client2.event_dispatcher().process_events();
    sleep(Duration::from_millis(300)).await;
    
    // Create a counter document
    let doc = client1.create_document(
        json!({ "title": "Counter", "value": 0 })
    ).await.expect("Failed to create counter");
    
    // Wait for initial sync
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Perform rapid updates from both clients
    tracing::info!("Performing rapid updates...");
    let update_count = 5;
    
    for i in 0..update_count {
        // Client 1 update
        client1.update_document(
            doc.id,
            json!({ "value": i * 2, "last_client": 1 })
        ).await.expect("Update failed");
        
        // Process events immediately
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        
        // Small delay
        sleep(Duration::from_millis(50)).await;
        
        // Client 2 update
        client2.update_document(
            doc.id,
            json!({ "value": i * 2 + 1, "last_client": 2 })
        ).await.expect("Update failed");
        
        // Process events immediately
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        
        sleep(Duration::from_millis(50)).await;
    }
    
    // Final event processing
    for _ in 0..5 {
        let _ = client1.event_dispatcher().process_events();
        let _ = client2.event_dispatcher().process_events();
        sleep(Duration::from_millis(100)).await;
    }
    
    // Verify both clients converged to same state
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    
    assert_eq!(docs1.len(), 1);
    assert_eq!(docs2.len(), 1);
    assert_eq!(docs1[0].content, docs2[0].content, "Clients should converge to same state");
    
    // Verify event counts
    {
        let e1 = events1.lock().unwrap();
        let e2 = events2.lock().unwrap();
        
        // Each client should have seen multiple updates
        assert!(e1.updated.len() >= update_count, "Client 1 should see at least {} updates", update_count);
        assert!(e2.updated.len() >= update_count, "Client 2 should see at least {} updates", update_count);
        
        // Check for conflicts (there might be some due to rapid updates)
        tracing::info!("Client 1 conflicts: {}, Client 2 conflicts: {}", 
                     e1.conflicts.len(), e2.conflicts.len());
    }
    
    tracing::info!("✓ Rapid updates handled correctly with proper event delivery");
}

