use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;

crate::integration_test!(test_demo_token_authentication, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    
    // Should be able to connect with demo token
    let client = ctx.create_test_client(user_id, "demo-token").await.expect("Failed to create client");
    
    // Should be able to create and sync a document
    let _doc = client.create_document("Demo Test Doc".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
    // Verify sync worked
    let synced_docs = client.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(synced_docs.len(), 1);
    assert_eq!(synced_docs[0].title, "Demo Test Doc");
});

crate::integration_test!(test_custom_token_auto_registration, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let custom_token = format!("test-token-{}", Uuid::new_v4());
    
    // First connection should auto-register user
    let client1 = ctx.create_test_client(user_id, &custom_token).await.expect("Failed to create client");
    
    // Create a document
    let _doc = client1.create_document("Auto Registration Test".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
    // Second connection with same credentials should work
    let client2 = ctx.create_test_client(user_id, &custom_token).await.expect("Failed to create client");
    
    // Should see the same document
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Auto Registration Test");
});

crate::integration_test!(test_invalid_token_rejection, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    
    // Register user with specific token
    let valid_token = format!("valid-token-{}", Uuid::new_v4());
    let client = ctx.create_test_client(user_id, &valid_token).await.expect("Failed to create client");
    
    // Create a document
    let _doc = client.create_document("Security Test".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
    // Try to connect with wrong token - should fail
    let db_path = format!(":memory:{}:invalid", user_id);
    let result = sync_client::SyncEngine::new(
        &db_path,
        &ctx.server_url,
        "wrong-token"
    ).await;
    
    // Connection should fail or subsequent operations should fail
    match result {
        Err(_) => {
            // Expected - connection rejected
        },
        Ok(mut invalid_engine) => {
            // If connection succeeded, starting the engine should fail
            let start_result = invalid_engine.start().await;
            assert!(start_result.is_err(), "Starting engine with invalid token should fail");
        }
    }
});

crate::integration_test!(test_concurrent_sessions, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create the first client and document to ensure the user exists
    let client0 = ctx.create_test_client(user_id, token).await.expect("Failed to create client0");
    let _doc0 = client0.create_document("Doc from client 0".to_string(), json!({"test": true}))
        .await.expect("Failed to create document 0");
    
    // Create remaining clients and documents
    let mut clients = vec![client0];
    
    for i in 1..5 {
        // Small delay between client connections
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let client = ctx.create_test_client(user_id, token).await
            .expect(&format!("Failed to create client {}", i));
        
        // Create document for this client
        let _doc = client.create_document(format!("Doc from client {}", i), json!({"test": true}))
            .await.expect(&format!("Failed to create document {}", i));
        
        clients.push(client);
    }
    
    // Test for eventual convergence - all clients should eventually see all documents
    // We're testing distributed systems, so we allow reasonable time for convergence
    let timeout = tokio::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    
    loop {
        // Check if all clients have converged
        let mut all_converged = true;
        let mut client_states = Vec::new();
        
        for (i, client) in clients.iter().enumerate() {
            let docs = client.get_all_documents().await.expect("Failed to get documents");
            client_states.push((i, docs.len()));
            if docs.len() != 5 {
                all_converged = false;
            }
        }
        
        if all_converged {
            // Success! All clients have converged to the correct state
            break;
        }
        
        if start.elapsed() > timeout {
            // Log the current state for debugging
            eprintln!("Convergence timeout! Client states after {} seconds:", timeout.as_secs());
            for (i, count) in client_states {
                let docs = clients[i].get_all_documents().await.expect("Failed to get documents");
                eprintln!("  Client {}: {} documents", i, count);
                for doc in &docs {
                    eprintln!("    - {}: {}", doc.id, doc.title);
                }
            }
            panic!("Clients did not converge within {} seconds", timeout.as_secs());
        }
        
        // Check every 100ms for convergence
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    
    // Verify the final converged state
    for (i, client) in clients.iter().enumerate() {
        let docs = client.get_all_documents().await.expect("Failed to get documents");
        assert_eq!(docs.len(), 5, "After convergence, client {} has {} documents", i, docs.len());
    }
});