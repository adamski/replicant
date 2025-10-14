use crate::integration::helpers::*;
use serde_json::json;
use futures_util::future;

crate::integration_test!(test_concurrent_edit_conflict_resolution, |ctx: TestContext| async move {
    let email = "alice@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-alice").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Both clients start with the same document
    let doc = client1.create_document(json!({"title": "Conflict Test", "test": true})).await.expect("Failed to create document");
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Both clients go offline (simulate by not syncing)
    // Both make different edits
    client1.update_document(doc.id, json!({"text": "Client 1 edit", "version": 1})).await.expect("Failed to update document");
    
    client2.update_document(doc.id, json!({"text": "Client 2 edit", "version": 2})).await.expect("Failed to update document");
    
    // Wait for automatic sync - conflict should be resolved
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Both should converge to the same state
    let final1 = client1.get_all_documents().await.expect("Failed to get documents");
    let final2 = client2.get_all_documents().await.expect("Failed to get documents");
    
    assert_eq!(final1.len(), 1);
    assert_eq!(final2.len(), 1);
    
    // Content should be the same (last-write-wins based on vector clock)
    assert_eq!(final1[0].content, final2[0].content);
    assert_eq!(final1[0].version, final2[0].version);
});

crate::integration_test!(test_delete_update_conflict, |ctx: TestContext| async move {
    let email = "bob@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-bob").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Create and sync document
    let doc = client1.create_document(json!({"title": "Delete-Update Conflict", "test": true})).await.expect("Failed to create document");
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Client 1 deletes while client 2 updates
    client1.delete_document(doc.id).await.expect("Failed to delete document");
    
    client2.update_document(doc.id, json!({"text": "Updated while being deleted"})).await.expect("Failed to update document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Delete should win (or system should handle gracefully)
    let final1 = client1.get_all_documents().await.expect("Failed to get documents");
    let final2 = client2.get_all_documents().await.expect("Failed to get documents");
    
    // Both should agree on final state
    assert_eq!(final1.len(), final2.len());
});

crate::integration_test!(test_rapid_concurrent_updates, |ctx: TestContext| async move {
    let email = "charlie@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-charlie").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Create multiple clients
    let mut clients = Vec::new();
    for _ in 0..5 {
        clients.push(ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client"));
    }
    
    // First client creates document
    let doc = clients[0].create_document(json!({"title": "Rapid Update Test", "test": true})).await.unwrap();
    let doc_id = doc.id;
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // All clients make rapid updates
    let handles: Vec<_> = clients.into_iter().enumerate().map(|(i, client)| {
        tokio::spawn(async move {
            for j in 0..10 {
                let _ = client.update_document(doc_id, json!({
                    "client": i,
                    "update": j,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                })).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            client
        })
    }).collect();
    
    // Wait for all updates to complete
    let clients: Vec<_> = future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Wait for automatic sync to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // All clients should have converged
    let mut final_states = Vec::new();
    for client in &clients {
        let docs = client.get_all_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        final_states.push(docs[0].content.clone());
    }
    
    // All should have the same final content
    for state in &final_states[1..] {
        assert_eq!(state, &final_states[0], "All clients should converge to same state");
    }
});

crate::integration_test!(test_vector_clock_convergence, |ctx: TestContext| async move {
    let email = "dave@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-dave").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let client3 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    
    // Create document on client 1
    let doc = client1.create_document(json!({"title": "Vector Clock Test", "test": true})).await.unwrap();
    let doc_id = doc.id;
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Each client makes updates in isolation
    for i in 0..3 {
        // Client 1 update
        client1.update_document(doc_id, json!({"client1_update": i})).await.unwrap();
        
        // Client 2 update
        client2.update_document(doc_id, json!({"client2_update": i})).await.unwrap();
        
        // Client 3 update
        client3.update_document(doc_id, json!({"client3_update": i})).await.unwrap();
    }
    
    // Wait for automatic sync to ensure convergence
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    
    // All should have the same final state
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    let docs3 = client3.get_all_documents().await.unwrap();
    
    assert_eq!(docs1[0].content, docs2[0].content);
    assert_eq!(docs2[0].content, docs3[0].content);
    assert_eq!(docs1[0].version, docs2[0].version);
    assert_eq!(docs2[0].version, docs3[0].version);
});