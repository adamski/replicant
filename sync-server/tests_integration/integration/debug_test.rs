use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;

crate::integration_test!(test_simple_broadcast, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    println!("Creating first client...");
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client1");
    
    println!("Creating document from client1...");
    let doc1 = client1.create_document(json!({"title": "Doc from client 1", "test": true}))
        .await.expect("Failed to create document 1");
    println!("Created document: {}", doc1.id);
    
    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    println!("Creating second client...");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client2");
    
    // Wait for sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    println!("Checking client1 documents...");
    let docs1 = client1.get_all_documents().await.expect("Failed to get docs from client1");
    println!("Client1 sees {} documents", docs1.len());
    
    println!("Checking client2 documents...");
    let docs2 = client2.get_all_documents().await.expect("Failed to get docs from client2");
    println!("Client2 sees {} documents", docs2.len());
    
    // Client2 should see the document created by client1
    assert_eq!(docs2.len(), 1, "Client2 should see 1 document but sees {}", docs2.len());
});