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
    
    // Create multiple concurrent sessions
    let mut clients = Vec::new();
    for i in 0..5 {
        let client = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
        
        // Each client creates a document
        let _doc = client.create_document(format!("Doc from client {}", i), json!({"test": true})).await.expect("Failed to create document");
        
        clients.push(client);
    }
    
    // Wait a bit for sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // All clients should see all documents
    for (i, client) in clients.iter().enumerate() {
        let docs = client.get_all_documents().await.expect("Failed to get documents");
        assert_eq!(docs.len(), 5, "Client {} should see all 5 documents", i);
    }
});