use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;

integration_test!(test_basic_sync_flow, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create two clients
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Client 1 creates a document
    let doc = TestContext::create_test_document(user_id, "Sync Test Doc");
    client1.create_document(doc.clone()).await.unwrap();
    
    // Client 2 syncs and should see the document
    client2.sync().await.unwrap();
    let docs = client2.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Sync Test Doc");
});

integration_test!(test_bidirectional_sync, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Both clients create documents
    let doc1 = TestContext::create_test_document(user_id, "Doc from Client 1");
    let doc2 = TestContext::create_test_document(user_id, "Doc from Client 2");
    
    client1.create_document(doc1).await.unwrap();
    client2.create_document(doc2).await.unwrap();
    
    // Both sync
    client1.sync().await.unwrap();
    client2.sync().await.unwrap();
    
    // Both should see both documents
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    
    assert_eq!(docs1.len(), 2);
    assert_eq!(docs2.len(), 2);
    
    // Verify both have the same documents
    let titles1: Vec<String> = docs1.iter().map(|d| d.title.clone()).collect();
    let titles2: Vec<String> = docs2.iter().map(|d| d.title.clone()).collect();
    
    assert!(titles1.contains(&"Doc from Client 1".to_string()));
    assert!(titles1.contains(&"Doc from Client 2".to_string()));
    assert!(titles2.contains(&"Doc from Client 1".to_string()));
    assert!(titles2.contains(&"Doc from Client 2".to_string()));
});

integration_test!(test_update_propagation, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Client 1 creates a document
    let mut doc = TestContext::create_test_document(user_id, "Update Test");
    doc.content = json!({"text": "Original content"});
    client1.create_document(doc.clone()).await.unwrap();
    
    // Client 2 syncs to get the document
    client2.sync().await.unwrap();
    
    // Client 1 updates the document
    doc.content = json!({"text": "Updated content"});
    doc.version += 1;
    client1.update_document(&doc).await.unwrap();
    
    // Client 2 syncs again
    client2.sync().await.unwrap();
    
    // Client 2 should see the update
    let docs = client2.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content["text"], "Updated content");
});

integration_test!(test_delete_propagation, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Client 1 creates documents
    let doc1 = TestContext::create_test_document(user_id, "Keep Me");
    let doc2 = TestContext::create_test_document(user_id, "Delete Me");
    
    client1.create_document(doc1.clone()).await.unwrap();
    client1.create_document(doc2.clone()).await.unwrap();
    
    // Client 2 syncs
    client2.sync().await.unwrap();
    assert_eq!(client2.get_all_documents().await.unwrap().len(), 2);
    
    // Client 1 deletes one document
    client1.delete_document(&doc2.id).await.unwrap();
    
    // Client 2 syncs again
    client2.sync().await.unwrap();
    
    // Client 2 should only see one document
    let remaining = client2.get_all_documents().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].title, "Keep Me");
});

integration_test!(test_large_document_sync, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Create a large document
    let mut doc = TestContext::create_test_document(user_id, "Large Document");
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
    
    doc.content = json!({
        "items": large_array,
        "metadata": {
            "count": 1000,
            "created": chrono::Utc::now().to_rfc3339()
        }
    });
    
    // Client 1 creates the large document
    client1.create_document(doc.clone()).await.unwrap();
    
    // Client 2 syncs
    client2.sync().await.unwrap();
    
    // Verify the document synced correctly
    let docs = client2.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content["items"].as_array().unwrap().len(), 1000);
});