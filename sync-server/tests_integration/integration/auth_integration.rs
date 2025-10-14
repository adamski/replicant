use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;

crate::integration_test!(test_demo_token_authentication, |ctx: TestContext| async move {
    let email = "alice@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-alice").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Connect with proper credentials
    let client = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
        .expect("Failed to create client");

    // Should be able to create and sync a document
    let _doc = client.create_document(json!({"title": "Demo Test Doc", "test": true})).await.expect("Failed to create document");

    // Verify sync worked
    let synced_docs = client.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(synced_docs.len(), 1);
    assert_eq!(synced_docs[0].title_or_default(), "Demo Test Doc");
});

crate::integration_test!(test_custom_token_auto_registration, |ctx: TestContext| async move {
    let email = "bob@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-bob").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // First connection should work with proper credentials
    let client1 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
        .expect("Failed to create client1");

    // Create a document
    let _doc = client1.create_document(json!({"title": "Auto Registration Test", "test": true})).await.expect("Failed to create document");

    // Wait for document to be processed by server
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Second connection with same credentials should work
    let client2 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
        .expect("Failed to create client2");

    // Wait for sync to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Should see the same document
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1, "Client2 should see 1 document but found {}", docs.len());
    assert_eq!(docs[0].title_or_default(), "Auto Registration Test");
});

crate::integration_test!(test_invalid_token_rejection, |ctx: TestContext| async move {
    let email = "charlie@test.local";

    // Generate valid credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-charlie").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Connect with valid credentials
    let client = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
        .expect("Failed to create client");

    // Create a document
    let _doc = client.create_document(json!({"title": "Security Test", "test": true})).await.expect("Failed to create document");

    // Try to connect with wrong credentials - should fail
    let db_path = format!(":memory:{}:invalid", user_id);
    let ws_url = format!("{}/ws", ctx.server_url);
    let result = sync_client::SyncEngine::new(
        &db_path,
        &ws_url,
        email,
        "rpa_wrong_key",
        "rps_wrong_secret"
    ).await;

    // Connection should fail (authentication happens during new())
    assert!(result.is_err(), "Creating engine with invalid credentials should fail");
});

crate::integration_test!(test_concurrent_sessions, |ctx: TestContext| async move {
    let email = "dave@test.local";

    // Generate proper HMAC credentials
    let (api_key, api_secret) = ctx.generate_test_credentials("test-dave").await
        .expect("Failed to generate credentials");

    // Create user
    let user_id = ctx.create_test_user(email).await.expect("Failed to create user");

    // Create the first client and document
    let client0 = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
        .expect("Failed to create client0");
    let _doc0 = client0.create_document(json!({"title": "Doc from client 0", "test": true}))
        .await.expect("Failed to create document 0");

    // Create remaining clients and documents
    let mut clients = vec![client0];

    for i in 1..5 {
        // Small delay between client connections
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = ctx.create_test_client(email, user_id, &api_key, &api_secret).await
            .expect(&format!("Failed to create client {}", i));

        // Create document for this client
        let _doc = client.create_document(json!({"title": format!("Doc from client {}", i), "test": true}))
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
                    eprintln!("    - {}: {}", doc.id, doc.title_or_default());
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