use crate::integration::helpers::*;
use uuid::Uuid;

integration_test!(test_demo_token_authentication, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    
    // Should be able to connect with demo token
    let client = ctx.create_test_client(user_id, "demo-token").await;
    
    // Should be able to create and sync a document
    let doc = TestContext::create_test_document(user_id, "Demo Test Doc");
    client.create_document(doc.clone()).await.unwrap();
    
    // Verify sync worked
    let synced_docs = client.get_all_documents().await.unwrap();
    assert_eq!(synced_docs.len(), 1);
    assert_eq!(synced_docs[0].title, "Demo Test Doc");
});

integration_test!(test_custom_token_auto_registration, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let custom_token = format!("test-token-{}", Uuid::new_v4());
    
    // First connection should auto-register user
    let client1 = ctx.create_test_client(user_id, &custom_token).await;
    
    // Create a document
    let doc = TestContext::create_test_document(user_id, "Auto Registration Test");
    client1.create_document(doc.clone()).await.unwrap();
    
    // Second connection with same credentials should work
    let client2 = ctx.create_test_client(user_id, &custom_token).await;
    
    // Should see the same document
    let docs = client2.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Auto Registration Test");
});

integration_test!(test_invalid_token_rejection, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    
    // Register user with specific token
    let valid_token = format!("valid-token-{}", Uuid::new_v4());
    let client = ctx.create_test_client(user_id, &valid_token).await;
    
    // Create a document
    let doc = TestContext::create_test_document(user_id, "Security Test");
    client.create_document(doc).await.unwrap();
    
    // Try to connect with wrong token - should fail
    let result = SyncClient::new(
        &ctx.server_url,
        user_id,
        "wrong-token",
        ":memory:"
    ).await;
    
    // Connection should fail or subsequent operations should fail
    match result {
        Err(_) => {
            // Expected - connection rejected
        },
        Ok(invalid_client) => {
            // If connection succeeded, operations should fail
            let sync_result = invalid_client.sync().await;
            assert!(sync_result.is_err(), "Sync should fail with invalid token");
        }
    }
});

integration_test!(test_concurrent_sessions, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create multiple concurrent sessions
    let mut clients = Vec::new();
    for i in 0..5 {
        let client = ctx.create_test_client(user_id, token).await;
        
        // Each client creates a document
        let doc = TestContext::create_test_document(user_id, &format!("Doc from client {}", i));
        client.create_document(doc).await.unwrap();
        
        clients.push(client);
    }
    
    // Wait a bit for sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // All clients should see all documents
    for (i, client) in clients.iter().enumerate() {
        let docs = client.get_all_documents().await.unwrap();
        assert_eq!(docs.len(), 5, "Client {} should see all 5 documents", i);
    }
});