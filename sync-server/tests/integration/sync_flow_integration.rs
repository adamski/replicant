use crate::integration::helpers::*;
use serde_json::json;

crate::integration_test!(test_basic_sync_flow, |ctx: TestContext| async move {
    let email = "alice@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-alice").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Create two clients with robust retry logic
    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Client 1 creates a document
    let _doc = client1.create_document(json!({"title": "Sync Test Doc", "test": true})).await.expect("Failed to create document");

    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title_or_default(), "Sync Test Doc");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_bidirectional_sync, |ctx: TestContext| async move {
    let email = "bob@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-bob").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Both clients create documents
    let _doc1 = client1.create_document(json!({"title": "Doc from Client 1", "test": true})).await.expect("Failed to create document");
    let _doc2 = client2.create_document(json!({"title": "Doc from Client 2", "test": true})).await.expect("Failed to create document");

    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Both should see both documents
    let docs1 = client1.get_all_documents().await.expect("Failed to get documents");
    let docs2 = client2.get_all_documents().await.expect("Failed to get documents");

    assert_eq!(docs1.len(), 2);
    assert_eq!(docs2.len(), 2);

    // Verify both have the same documents
    let titles1: Vec<String> = docs1.iter().map(|d| d.title_or_default().to_string()).collect();
    let titles2: Vec<String> = docs2.iter().map(|d| d.title_or_default().to_string()).collect();

    assert!(titles1.contains(&"Doc from Client 1".to_string()));
    assert!(titles1.contains(&"Doc from Client 2".to_string()));
    assert!(titles2.contains(&"Doc from Client 1".to_string()));
    assert!(titles2.contains(&"Doc from Client 2".to_string()));
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_update_propagation, |ctx: TestContext| async move {
    let email = "charlie@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-charlie").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Client 1 creates a document
    let doc = client1.create_document(json!({"title": "Update Test", "text": "Original content"})).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Client 1 updates the document
    client1.update_document(doc.id, json!({"text": "Updated content"})).await.expect("Failed to update document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Client 2 should see the update
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content["text"], "Updated content");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_delete_propagation, |ctx: TestContext| async move {
    let email = "dave@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-dave").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Client 1 creates documents
    let _doc1 = client1.create_document(json!({"title": "Keep Me", "test": true})).await.expect("Failed to create document");
    let doc2 = client1.create_document(json!({"title": "Delete Me", "test": true})).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    assert_eq!(client2.get_all_documents().await.expect("Failed to get documents").len(), 2);
    
    // Client 1 deletes one document
    client1.delete_document(doc2.id).await.expect("Failed to delete document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Client 2 should only see one document
    let remaining = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].title_or_default(), "Keep Me");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_large_document_sync, |ctx: TestContext| async move {
    let email = "eve@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-eve").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Create a large document
    let large_array: Vec<serde_json::Value> = (0..1000)
        .map(|i| json!({
            "index": i,
            "data": format!("Item number {} with some content", i),
            "nested": {
                "field1": "value1",
                "field2": i * 2
            }
        }))
        .collect();
    
    let content = json!({
        "title": "Large Document",
        "items": large_array,
        "metadata": {
            "count": 1000,
            "created": chrono::Utc::now().to_rfc3339()
        }
    });

    // Client 1 creates the large document
    let _doc = client1.create_document(content).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Verify the document synced correctly
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content["items"].as_array().unwrap().len(), 1000);
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_simultaneous_offline_outage_scenario, |ctx: TestContext| async move {
    let email = "frank@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-frank").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Phase 1: All clients online and synced
    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client1");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client2");
    let client3 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client3");
    
    // Create initial shared document
    let shared_doc = client1.create_document(
        json!({"title": "Shared Document", "content": "initial content", "version": 0})
    ).await.expect("Failed to create shared document");
    
    // Wait for initial sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Verify all clients have the document (with debugging)
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    let docs3 = client3.get_all_documents().await.unwrap();
    
    // Initial sync successful
    
    assert_eq!(docs1.len(), 1, "Client1 should have 1 document after initial sync");
    assert_eq!(docs2.len(), 1, "Client2 should have 1 document after initial sync");
    assert_eq!(docs3.len(), 1, "Client3 should have 1 document after initial sync");
    
    // Phase 2: Simulate simultaneous internet outage
    // All clients go offline (we simulate this by dropping their connections)
    drop(client1);
    drop(client2);
    drop(client3);
    
    // Wait for server to detect disconnections
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Phase 3: Clients work offline and make conflicting changes
    // Each client reconnects as new instance (simulating restart after outage)
    // and makes different changes to the same document
    
    // Client 1 comes back and updates the document
    println!("🔌 Client1 reconnecting after outage...");
    let client1_back = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to reconnect client1");
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await; // Let it sync existing state
    
    let docs_before_update = client1_back.get_all_documents().await.unwrap();
    println!("📊 Client1 after reconnect: {} docs", docs_before_update.len());
    
    client1_back.update_document(
        shared_doc.id, 
        json!({"content": "updated by client 1 after outage", "version": 1, "editor": "client1"})
    ).await.expect("Failed to update from client1");
    
    println!("✏️ Client1 made update after outage");
    
    // Client 2 comes back and makes conflicting update
    let client2_back = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to reconnect client2");
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; // Let it sync existing state
    
    client2_back.update_document(
        shared_doc.id, 
        json!({"content": "updated by client 2 after outage", "version": 2, "editor": "client2"})
    ).await.expect("Failed to update from client2");
    
    // Client 3 comes back and also makes conflicting update
    let client3_back = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to reconnect client3");
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; // Let it sync existing state
    
    client3_back.update_document(
        shared_doc.id, 
        json!({"content": "updated by client 3 after outage", "version": 3, "editor": "client3"})
    ).await.expect("Failed to update from client3");
    
    // Phase 4: All clients come back online simultaneously (internet restored)
    // Wait for conflict resolution and convergence
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    
    // Phase 5: Verify eventual consistency
    let docs1 = client1_back.get_all_documents().await.expect("Failed to get documents from client1");
    let docs2 = client2_back.get_all_documents().await.expect("Failed to get documents from client2");
    let docs3 = client3_back.get_all_documents().await.expect("Failed to get documents from client3");
    
    // All clients should have the same number of documents
    assert_eq!(docs1.len(), 1, "Client1 should have 1 document");
    assert_eq!(docs2.len(), 1, "Client2 should have 1 document");
    assert_eq!(docs3.len(), 1, "Client3 should have 1 document");
    
    // All clients should converge to the same final state
    // (The exact winner depends on conflict resolution algorithm - last write wins, vector clock, etc.)
    let final_content1 = &docs1[0].content;
    let final_content2 = &docs2[0].content;
    let final_content3 = &docs3[0].content;
    
    assert_eq!(final_content1, final_content2, "Client1 and Client2 should have same final content");
    assert_eq!(final_content2, final_content3, "Client2 and Client3 should have same final content");
    
    // Verify the document has been properly updated (not stuck at initial state)
    assert_ne!(final_content1["content"], "initial content", "Document should not be stuck at initial content");
    
    // Verify one of the clients won the conflict resolution
    let final_editor = final_content1["editor"].as_str().unwrap();
    assert!(
        ["client1", "client2", "client3"].contains(&final_editor),
        "Final editor should be one of the clients: {}", final_editor
    );
    
    println!("✅ Simultaneous outage test passed - final state: {:?}", final_content1);
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);


crate::integration_test!(test_array_duplication_bug, |ctx: TestContext| async move {
    let email = "grace@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-grace").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Create two clients with robust retry logic
    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client1");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client2");
    
    // Client 1 creates a document with an array
    let doc = client1.create_document(
        json!({
            "title": "Array Test Doc",
            "tags": ["existing"]
        })
    ).await.expect("Failed to create document");
    
    // Wait for sync to client2
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Verify client2 has the document
    let docs2 = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs2.len(), 1);
    assert_eq!(docs2[0].content["tags"].as_array().unwrap().len(), 1);
    assert_eq!(docs2[0].content["tags"][0], "existing");
    
    // Client 1 adds an item to the array (this is where the bug might occur)
    let mut updated_content = doc.content.clone();
    if let Some(tags) = updated_content["tags"].as_array_mut() {
        tags.push(json!("test"));
    }
    
    client1.update_document(doc.id, updated_content).await.expect("Failed to update document");
    
    // Wait for sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Get updated documents from both clients
    let docs1_updated = client1.get_all_documents().await.expect("Failed to get client1 documents");
    let docs2_updated = client2.get_all_documents().await.expect("Failed to get client2 documents");
    
    assert_eq!(docs1_updated.len(), 1);
    assert_eq!(docs2_updated.len(), 1);
    
    // Check for array duplication bug
    let tags1 = docs1_updated[0].content["tags"].as_array().unwrap();
    let tags2 = docs2_updated[0].content["tags"].as_array().unwrap();
    
    println!("Client1 tags: {:?}", tags1);
    println!("Client2 tags: {:?}", tags2);
    
    // Both should have exactly 2 items, not duplicates
    assert_eq!(tags1.len(), 2, "Client1 should have exactly 2 tags, got: {:?}", tags1);
    assert_eq!(tags2.len(), 2, "Client2 should have exactly 2 tags, got: {:?}", tags2);
    
    // Check content is correct
    assert_eq!(tags1[0], "existing");
    assert_eq!(tags1[1], "test");
    assert_eq!(tags2[0], "existing");
    assert_eq!(tags2[1], "test");
    
    // Ensure no duplicates
    assert_ne!(tags1[0], tags1[1], "Should not have duplicate tags in client1");
    assert_ne!(tags2[0], tags2[1], "Should not have duplicate tags in client2");
    
    println!("✅ Array operations test passed - no duplication detected");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}, true);