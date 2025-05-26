use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Barrier;

integration_test!(test_many_concurrent_clients, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    let client_count = 20;
    
    // Create many clients concurrently
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(client_count));
    
    for i in 0..client_count {
        let ctx_clone = ctx.clone();
        let barrier_clone = barrier.clone();
        
        let handle = tokio::spawn(async move {
            let client = ctx_clone.create_test_client(user_id, token).await;
            
            // Wait for all clients to be ready
            barrier_clone.wait().await;
            
            // Each client creates a document
            let doc = TestContext::create_test_document(
                user_id, 
                &format!("Client {} Document", i)
            );
            client.create_document(doc).await.unwrap();
            
            // Sync to get all documents
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            client.sync().await.unwrap();
            
            client
        });
        
        handles.push(handle);
    }
    
    // Wait for all clients to complete
    let clients: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Give time for all syncs to propagate
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Final sync for all clients
    for client in &clients {
        client.sync().await.unwrap();
    }
    
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

integration_test!(test_concurrent_updates_same_document, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    let client_count = 10;
    
    // First client creates the document
    let client0 = ctx.create_test_client(user_id, token).await;
    let doc = TestContext::create_test_document(user_id, "Concurrent Update Target");
    client0.create_document(doc.clone()).await.unwrap();
    
    // Create multiple clients
    let mut handles = Vec::new();
    let barrier = Arc::new(Barrier::new(client_count));
    
    for i in 0..client_count {
        let ctx_clone = ctx.clone();
        let barrier_clone = barrier.clone();
        let doc_id = doc.id;
        
        let handle = tokio::spawn(async move {
            let client = ctx_clone.create_test_client(user_id, token).await;
            
            // Sync to get the document
            client.sync().await.unwrap();
            
            // Wait for all clients to be ready
            barrier_clone.wait().await;
            
            // All clients update the same document simultaneously
            let docs = client.get_all_documents().await.unwrap();
            if let Some(mut doc) = docs.into_iter().find(|d| d.id == doc_id) {
                doc.content = json!({
                    "updater": i,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });
                doc.version += 1;
                client.update_document(&doc).await.unwrap();
            }
            
            // Sync changes
            client.sync().await.unwrap();
            
            client
        });
        
        handles.push(handle);
    }
    
    // Wait for all updates
    let clients: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Multiple sync rounds to ensure convergence
    for _ in 0..3 {
        for client in &clients {
            client.sync().await.unwrap();
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
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

integration_test!(test_server_under_load, |ctx: TestContext| async move {
    let token = "demo-token";
    let user_count = 10;
    let docs_per_user = 20;
    
    let mut handles = Vec::new();
    
    for user_idx in 0..user_count {
        let ctx_clone = ctx.clone();
        
        let handle = tokio::spawn(async move {
            let user_id = Uuid::new_v4();
            let client = ctx_clone.create_test_client(user_id, token).await;
            
            // Each user creates multiple documents
            for doc_idx in 0..docs_per_user {
                let doc = TestContext::create_test_document(
                    user_id,
                    &format!("User {} Doc {}", user_idx, doc_idx)
                );
                
                client.create_document(doc).await.unwrap();
                
                // Occasional syncs
                if doc_idx % 5 == 0 {
                    client.sync().await.unwrap();
                }
            }
            
            // Final sync
            client.sync().await.unwrap();
            
            // Verify all documents
            let docs = client.get_all_documents().await.unwrap();
            assert_eq!(docs.len(), docs_per_user);
            
            (user_id, client)
        });
        
        handles.push(handle);
    }
    
    // Wait for all users to complete
    let results: Vec<_> = futures::future::join_all(handles)
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

integration_test!(test_connection_stability, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create multiple clients that connect and disconnect
    for round in 0..5 {
        let mut clients = Vec::new();
        
        // Connect several clients
        for i in 0..5 {
            let client = ctx.create_test_client(user_id, token).await;
            
            // Create a document
            let doc = TestContext::create_test_document(
                user_id,
                &format!("Round {} Client {} Doc", round, i)
            );
            client.create_document(doc).await.unwrap();
            
            clients.push(client);
        }
        
        // Sync all
        for client in &clients {
            client.sync().await.unwrap();
        }
        
        // Clients go out of scope and disconnect
        drop(clients);
        
        // Small delay between rounds
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
    // Final client should see all documents
    let final_client = ctx.create_test_client(user_id, token).await;
    final_client.sync().await.unwrap();
    
    let docs = final_client.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 25); // 5 rounds * 5 clients
});