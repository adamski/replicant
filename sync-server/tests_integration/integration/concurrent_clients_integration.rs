use crate::integration::helpers::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Barrier;
use futures_util::future;

crate::integration_test!(test_many_concurrent_clients, |ctx: TestContext| async move {
    let email = "alice@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-alice").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client_count = 20;
    
    // Create many clients concurrently
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(client_count));

    for i in 0..client_count {
        let ctx_clone = ctx.clone();
        let barrier_clone = barrier.clone();
        let api_key_clone = api_key.clone();
        let api_secret_clone = api_secret.clone();

        let handle = tokio::spawn(async move {
            let client = ctx_clone.create_test_client(email, user_id, &api_key_clone, &api_secret_clone).await.expect("Failed to create client");
            
            // Wait for all clients to be ready
            barrier_clone.wait().await;
            
            // Each client creates a document
            let _doc = client.create_document(
                json!({"title": format!("Client {} Document", i), "test": true})
            ).await.unwrap();
            
            // Wait for automatic sync
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            client
        });
        
        handles.push(handle);
    }
    
    // Wait for all clients to complete
    let clients: Vec<_> = future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Give time for all syncs to propagate
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Wait for final sync to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // All clients should see all documents
    for (i, client) in clients.iter().enumerate() {
        let docs = client.get_all_documents().await.unwrap();
        assert_eq!(
            docs.len(), 
            client_count, 
            "Client {} should see all {} documents", 
            i, 
            client_count
        );
    }
});

crate::integration_test!(test_concurrent_updates_same_document, |ctx: TestContext| async move {
    let email = "bob@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-bob").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    let client_count = 10;

    // First client creates the document
    let client0 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    let doc = client0.create_document(json!({"title": "Concurrent Update Target", "test": true})).await.unwrap();
    let doc_id = doc.id;
    
    // Wait for document to be processed by server
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Create multiple clients
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(client_count));

    for i in 0..client_count {
        let ctx_clone = ctx.clone();
        let barrier_clone = barrier.clone();
        let api_key_clone = api_key.clone();
        let api_secret_clone = api_secret.clone();

        let handle = tokio::spawn(async move {
            let client = ctx_clone.create_test_client(email, user_id, &api_key_clone, &api_secret_clone).await.expect("Failed to create client");
            
            // Wait for automatic sync to get the document
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            
            // Wait for all clients to be ready
            barrier_clone.wait().await;
            
            // All clients update the same document simultaneously
            client.update_document(doc_id, json!({
                "updater": i,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })).await.unwrap();
            
            // Wait for automatic sync
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            client
        });
        
        handles.push(handle);
    }
    
    // Wait for all updates
    let clients: Vec<_> = future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Wait for automatic sync to ensure convergence
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    
    // All clients should have converged to the same state
    let mut final_contents = Vec::new();
    for client in &clients {
        let docs = client.get_all_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        final_contents.push(docs[0].content.clone());
    }
    
    // All should have the same content
    for content in &final_contents[1..] {
        assert_eq!(content, &final_contents[0]);
    }
});

crate::integration_test!(test_server_under_load, |ctx: TestContext| async move {
    let user_count = 10;
    let docs_per_user = 20;
    
    let mut handles = Vec::new();
    
    for user_idx in 0..user_count {
        let ctx_clone = ctx.clone();

        let handle = tokio::spawn(async move {
            // Generate credentials for this user
            let email = format!("user{}@test.local", user_idx);
            let (api_key, api_secret) = ctx_clone.generate_test_credentials(&format!("test-user{}", user_idx)).await
                .expect("Failed to generate credentials");

            // Create user
            let user_id = ctx_clone.create_test_user(&email).await.expect("Failed to create user");

            let client = ctx_clone.create_test_client(&email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
            
            // Each user creates multiple documents
            for doc_idx in 0..docs_per_user {
                let _doc = client.create_document(
                    json!({"title": format!("User {} Doc {}", user_idx, doc_idx), "test": true})
                ).await.unwrap();
                
                // Small delay to spread out load
                if doc_idx % 5 == 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
            
            // Wait for automatic sync
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            
            // Verify all documents
            let docs = client.get_all_documents().await.unwrap();
            assert_eq!(docs.len(), docs_per_user);
            
            (user_id, client)
        });
        
        handles.push(handle);
    }
    
    // Wait for all users to complete
    let results: Vec<_> = future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    assert_eq!(results.len(), user_count);
    
    // Each user should only see their own documents
    for (user_id, client) in results {
        let docs = client.get_all_documents().await.unwrap();
        assert_eq!(docs.len(), docs_per_user);
        
        // All documents should belong to this user
        for doc in docs {
            assert_eq!(doc.user_id, user_id);
        }
    }
});

crate::integration_test!(test_connection_stability, |ctx: TestContext| async move {
    let email = "charlie@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-charlie").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");
    
    // Create multiple clients that connect and disconnect
    for round in 0..5 {
        let mut clients = Vec::new();
        
        // Connect several clients
        for i in 0..5 {
            let client = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
            
            // Create a document
            let _doc = client.create_document(
                json!({"title": format!("Round {} Client {} Doc", round, i), "test": true})
            ).await.unwrap();
            
            clients.push(client);
        }
        
        // Wait for automatic sync
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        
        // Clients go out of scope and disconnect
        drop(clients);
        
        // Small delay between rounds
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
    // Final client should see all documents
    let final_client = ctx.create_test_client(email, user_id, &api_key, &api_secret).await.expect("Failed to create client");
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    let docs = final_client.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 25); // 5 rounds * 5 clients
});