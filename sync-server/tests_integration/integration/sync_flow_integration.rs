use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;

crate::integration_test!(test_basic_sync_flow, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create two clients
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    
    // Client 1 creates a document
    let _doc = client1.create_document("Sync Test Doc".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Sync Test Doc");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
});

crate::integration_test!(test_bidirectional_sync, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    
    // Both clients create documents
    let _doc1 = client1.create_document("Doc from Client 1".to_string(), json!({"test": true})).await.expect("Failed to create document");
    let _doc2 = client2.create_document("Doc from Client 2".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    
    // Both should see both documents
    let docs1 = client1.get_all_documents().await.expect("Failed to get documents");
    let docs2 = client2.get_all_documents().await.expect("Failed to get documents");
    
    assert_eq!(docs1.len(), 2);
    assert_eq!(docs2.len(), 2);
    
    // Verify both have the same documents
    let titles1: Vec<String> = docs1.iter().map(|d| d.title.clone()).collect();
    let titles2: Vec<String> = docs2.iter().map(|d| d.title.clone()).collect();
    
    assert!(titles1.contains(&"Doc from Client 1".to_string()));
    assert!(titles1.contains(&"Doc from Client 2".to_string()));
    assert!(titles2.contains(&"Doc from Client 1".to_string()));
    assert!(titles2.contains(&"Doc from Client 2".to_string()));
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
});

crate::integration_test!(test_update_propagation, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    
    // Client 1 creates a document
    let doc = client1.create_document("Update Test".to_string(), json!({"text": "Original content"})).await.expect("Failed to create document");
    
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
});

crate::integration_test!(test_delete_propagation, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    
    // Client 1 creates documents
    let _doc1 = client1.create_document("Keep Me".to_string(), json!({"test": true})).await.expect("Failed to create document");
    let doc2 = client1.create_document("Delete Me".to_string(), json!({"test": true})).await.expect("Failed to create document");
    
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
    assert_eq!(remaining[0].title, "Keep Me");
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
});

crate::integration_test!(test_large_document_sync, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    let client2 = ctx.create_test_client(user_id, token).await.expect("Failed to create client");
    
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
        "items": large_array,
        "metadata": {
            "count": 1000,
            "created": chrono::Utc::now().to_rfc3339()
        }
    });
    
    // Client 1 creates the large document
    let _doc = client1.create_document("Large Document".to_string(), content).await.expect("Failed to create document");
    
    // Wait for automatic sync
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Verify the document synced correctly
    let docs = client2.get_all_documents().await.expect("Failed to get documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content["items"].as_array().unwrap().len(), 1000);
    
    // Keep clients alive briefly to avoid disconnect race
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
});