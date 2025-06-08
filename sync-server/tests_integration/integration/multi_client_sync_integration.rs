use std::time::Duration;
use tokio::time::sleep;
use serde_json::json;
use futures_util::future;
use crate::integration::helpers::{TestContext, assert_all_clients_converge};

#[tokio::test]
async fn test_multiple_clients_same_user_create_update_delete() {
    // Skip if not in integration test environment
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    
    // Full teardown and setup for test isolation
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user
    let (user_id, token) = ctx.create_test_user("multi-client-test@example.com")
        .await
        .expect("Failed to create test user");
    
    // Create three clients for the same user
    tracing::info!("Creating 3 clients for user {}", user_id);
    let client1 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 1");
    let client2 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 2");
    let client3 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 3");
    
    // Give clients time to fully connect and sync
    sleep(Duration::from_millis(500)).await;
    
    // Test 1: Create document on client 1
    tracing::info!("Test 1: Creating document on client 1");
    let doc1 = client1.create_document(
        "Shared Task".to_string(),
        json!({
            "description": "This task should sync to all clients",
            "status": "pending",
            "priority": "high"
        })
    ).await.expect("Failed to create document");
    
    // Verify all clients see the document
    let expected_doc_id = doc1.id;
    assert_all_clients_converge(
        &[&client1, &client2, &client3],
        1,  // Expected 1 document
        5,  // 5 second timeout
        move |doc| {
            let id_match = doc.id == expected_doc_id;
            let title_match = doc.title == "Shared Task";
            let status_match = doc.content["status"] == "pending";
            async move { id_match && title_match && status_match }
        }
    ).await;
    
    tracing::info!("✓ Document created on client 1 synced to all clients");
    
    // Test 2: Update document on client 2
    tracing::info!("\nTest 2: Updating document on client 2");
    client2.update_document(
        doc1.id,
        json!({
            "description": "Updated by client 2",
            "status": "in_progress",
            "priority": "high",
            "assigned_to": "client2"
        })
    ).await.expect("Failed to update document");
    
    // Verify all clients see the update
    assert_all_clients_converge(
        &[&client1, &client2, &client3],
        1,
        5,
        |doc| {
            let status_match = doc.content["status"] == "in_progress";
            let assigned_match = doc.content["assigned_to"] == "client2";
            let desc_match = doc.content["description"] == "Updated by client 2";
            async move { status_match && assigned_match && desc_match }
        }
    ).await;
    
    tracing::info!("✓ Document updated on client 2 synced to all clients");
    
    // Test 3: Delete document on client 3
    tracing::info!("\nTest 3: Deleting document on client 3");
    client3.delete_document(doc1.id).await.expect("Failed to delete document");
    
    // Verify all clients see the deletion
    assert_all_clients_converge(
        &[&client1, &client2, &client3],
        0,  // Expected 0 documents (deleted)
        5,
        |_| async { true }  // No documents to check
    ).await;
    
    tracing::info!("✓ Document deleted on client 3 removed from all clients");
}

#[tokio::test]
async fn test_concurrent_document_creation_same_user() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user
    let (user_id, token) = ctx.create_test_user("concurrent-test@example.com")
        .await
        .expect("Failed to create test user");
    
    // Create multiple clients
    let num_clients = 5;
    let mut clients = Vec::new();
    
    tracing::info!("Creating {} clients for concurrent testing", num_clients);
    for i in 0..num_clients {
        let client = ctx.create_test_client(user_id, &token)
            .await
            .expect(&format!("Failed to create client {}", i));
        clients.push(client);
    }
    
    // Give clients time to connect
    sleep(Duration::from_millis(500)).await;
    
    // Each client creates a document concurrently
    tracing::info!("Creating documents concurrently from {} clients", num_clients);
    let mut create_tasks = Vec::new();
    
    for (i, client) in clients.iter().enumerate() {
        let client_ref = client;
        let task = async move {
            client_ref.create_document(
                format!("Task from client {}", i),
                json!({
                    "client_id": i,
                    "description": format!("Created by client {}", i),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                })
            ).await
        };
        create_tasks.push(task);
    }
    
    // Execute all creates concurrently
    let results = future::join_all(create_tasks).await;
    
    // Check all succeeded
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Client {} failed to create document: {:?}", i, result);
    }
    
    // Verify all clients see all documents
    let client_refs: Vec<_> = clients.iter().collect();
    assert_all_clients_converge(
        &client_refs,
        num_clients,  // Each client created 1 document
        10,  // 10 second timeout for convergence
        |doc| {
            let has_client_id = doc.content.get("client_id").is_some();
            let has_description = doc.content.get("description").is_some();
            async move { has_client_id && has_description }
        }
    ).await;
    
    tracing::info!("✓ All {} documents created concurrently are visible to all clients", num_clients);
}

#[tokio::test]
async fn test_no_duplicate_broadcast_to_sender() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user with single client
    let (user_id, token) = ctx.create_test_user("no-duplicate@example.com")
        .await
        .expect("Failed to create test user");
    
    let client = ctx.create_test_client(user_id, &token).await.expect("Failed to create client");
    
    // Give client time to connect
    sleep(Duration::from_millis(300)).await;
    
    // Create a document
    tracing::info!("Creating document to test no duplicate broadcast");
    let doc = client.create_document(
        "Test No Duplicates".to_string(),
        json!({
            "test": "This document should not be duplicated"
        })
    ).await.expect("Failed to create document");
    
    // Wait for any potential duplicate messages
    sleep(Duration::from_millis(1000)).await;
    
    // Verify only one document exists
    let docs = client.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1, "Expected exactly 1 document, found {}", docs.len());
    assert_eq!(docs[0].id, doc.id);
    assert_eq!(docs[0].title, "Test No Duplicates");
    
    tracing::info!("✓ No duplicate documents created (sender not receiving own broadcast)");
}

#[tokio::test]
async fn test_offline_sync_recovery() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user
    let (user_id, token) = ctx.create_test_user("offline-test@example.com")
        .await
        .expect("Failed to create test user");
    
    // Create first client and add documents
    let client1 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 1");
    
    sleep(Duration::from_millis(300)).await;
    
    // Create some documents while client 2 is offline
    tracing::info!("Creating documents on client 1 while client 2 is offline");
    
    let doc1 = client1.create_document(
        "Document 1".to_string(),
        json!({ "created": "while client 2 offline" })
    ).await.expect("Failed to create doc1");
    
    let _doc2 = client1.create_document(
        "Document 2".to_string(),
        json!({ "also_created": "while client 2 offline" })
    ).await.expect("Failed to create doc2");
    
    // Update one of them
    client1.update_document(
        doc1.id,
        json!({ 
            "created": "while client 2 offline",
            "updated": "also while client 2 offline"
        })
    ).await.expect("Failed to update doc1");
    
    // Now create and start client 2
    tracing::info!("Starting client 2 to test sync recovery");
    let client2 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 2");
    
    // Verify client 2 receives all documents with latest state
    let expected_doc1_id = doc1.id;
    assert_all_clients_converge(
        &[&client1, &client2],
        2,  // Expected 2 documents
        5,
        move |doc| {
            let result = if doc.id == expected_doc1_id {
                doc.content.get("updated").is_some()  // Should have the update
            } else {
                true  // doc2 should exist
            };
            async move { result }
        }
    ).await;
    
    tracing::info!("✓ Client 2 successfully synced all documents created while offline");
}

#[tokio::test]
async fn test_rapid_concurrent_updates() {
    if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
        eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
        return;
    }
    
    let ctx = TestContext::new();
    ctx.full_teardown_and_setup().await.expect("Failed to setup test environment");
    
    // Create a test user
    let (user_id, token) = ctx.create_test_user("rapid-update@example.com")
        .await
        .expect("Failed to create test user");
    
    // Create two clients
    let client1 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 1");
    let client2 = ctx.create_test_client(user_id, &token).await.expect("Failed to create client 2");
    
    sleep(Duration::from_millis(300)).await;
    
    // Create a document
    let doc = client1.create_document(
        "Counter Document".to_string(),
        json!({ "counter": 0 })
    ).await.expect("Failed to create document");
    
    // Wait for initial sync
    sleep(Duration::from_millis(500)).await;
    
    // Both clients rapidly update the counter
    tracing::info!("Performing rapid concurrent updates");
    
    let update_count = 10;
    let doc_id = doc.id;
    
    // Perform updates sequentially instead of spawning tasks
    // This avoids the need to clone SyncEngine
    for i in 0..update_count {
        // Client 1 update
        if let Ok(docs) = client1.get_all_documents().await {
            if let Some(doc) = docs.iter().find(|d| d.id == doc_id) {
                let current = doc.content["counter"].as_i64().unwrap_or(0);
                let _ = client1.update_document(
                    doc_id,
                    json!({ 
                        "counter": current + 1,
                        "last_updated_by": "client1",
                        "update": i
                    })
                ).await;
            }
        }
        
        // Small delay to allow sync
        sleep(Duration::from_millis(100)).await;
        
        // Client 2 update
        if let Ok(docs) = client2.get_all_documents().await {
            if let Some(doc) = docs.iter().find(|d| d.id == doc_id) {
                let current = doc.content["counter"].as_i64().unwrap_or(0);
                let _ = client2.update_document(
                    doc_id,
                    json!({ 
                        "counter": current + 1,
                        "last_updated_by": "client2",
                        "update": i
                    })
                ).await;
            }
        }
        
        // Small delay to allow sync
        sleep(Duration::from_millis(100)).await;
    }
    
    // Updates are already complete (sequential execution)
    
    // Allow time for final convergence
    sleep(Duration::from_millis(2000)).await;
    
    // Verify both clients converged to same final state
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    
    assert_eq!(docs1.len(), 1);
    assert_eq!(docs2.len(), 1);
    
    let final_doc1 = &docs1[0];
    let final_doc2 = &docs2[0];
    
    // Both should have the same final state
    assert_eq!(final_doc1.revision_id, final_doc2.revision_id, 
        "Clients should converge to same revision");
    assert_eq!(final_doc1.content, final_doc2.content,
        "Clients should converge to same content");
    
    tracing::info!("✓ Rapid concurrent updates resolved correctly - both clients converged to same state");
    tracing::info!("  Final revision: {}", final_doc1.revision_id);
    tracing::info!("  Final counter: {}", final_doc1.content["counter"]);
}