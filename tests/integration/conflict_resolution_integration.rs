use crate::integration::helpers::*;
use uuid::Uuid;
use serde_json::json;
use sync_core::models::VectorClock;

integration_test!(test_concurrent_edit_conflict_resolution, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Both clients start with the same document
    let doc = TestContext::create_test_document(user_id, "Conflict Test");
    client1.create_document(doc.clone()).await.unwrap();
    client2.sync().await.unwrap();
    
    // Both clients go offline (simulate by not syncing)
    // Both make different edits
    let docs1 = client1.get_all_documents().await.unwrap();
    let mut doc1 = docs1[0].clone();
    doc1.content = json!({"text": "Client 1 edit", "version": 1});
    doc1.version += 1;
    client1.update_document(&doc1).await.unwrap();
    
    let docs2 = client2.get_all_documents().await.unwrap();
    let mut doc2 = docs2[0].clone();
    doc2.content = json!({"text": "Client 2 edit", "version": 2});
    doc2.version += 1;
    client2.update_document(&doc2).await.unwrap();
    
    // Both sync - conflict should be resolved
    client1.sync().await.unwrap();
    client2.sync().await.unwrap();
    
    // Both should converge to the same state
    let final1 = client1.get_all_documents().await.unwrap();
    let final2 = client2.get_all_documents().await.unwrap();
    
    assert_eq!(final1.len(), 1);
    assert_eq!(final2.len(), 1);
    
    // Content should be the same (last-write-wins based on vector clock)
    assert_eq!(final1[0].content, final2[0].content);
    assert_eq!(final1[0].version, final2[0].version);
});

integration_test!(test_delete_update_conflict, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    
    // Create and sync document
    let doc = TestContext::create_test_document(user_id, "Delete-Update Conflict");
    client1.create_document(doc.clone()).await.unwrap();
    client2.sync().await.unwrap();
    
    // Client 1 deletes while client 2 updates
    client1.delete_document(&doc.id).await.unwrap();
    
    let docs2 = client2.get_all_documents().await.unwrap();
    let mut doc2 = docs2[0].clone();
    doc2.content = json!({"text": "Updated while being deleted"});
    doc2.version += 1;
    client2.update_document(&doc2).await.unwrap();
    
    // Both sync
    client1.sync().await.unwrap();
    client2.sync().await.unwrap();
    
    // Delete should win (or system should handle gracefully)
    let final1 = client1.get_all_documents().await.unwrap();
    let final2 = client2.get_all_documents().await.unwrap();
    
    // Both should agree on final state
    assert_eq!(final1.len(), final2.len());
});

integration_test!(test_rapid_concurrent_updates, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create multiple clients
    let mut clients = Vec::new();
    for _ in 0..5 {
        clients.push(ctx.create_test_client(user_id, token).await);
    }
    
    // First client creates document
    let doc = TestContext::create_test_document(user_id, "Rapid Update Test");
    clients[0].create_document(doc.clone()).await.unwrap();
    
    // All clients sync to get the document
    for client in &clients[1..] {
        client.sync().await.unwrap();
    }
    
    // All clients make rapid updates
    let handles: Vec<_> = clients.into_iter().enumerate().map(|(i, client)| {
        let doc_id = doc.id;
        tokio::spawn(async move {
            for j in 0..10 {
                if let Ok(docs) = client.get_all_documents().await {
                    if let Some(mut doc) = docs.into_iter().find(|d| d.id == doc_id) {
                        doc.content = json!({
                            "client": i,
                            "update": j,
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });
                        doc.version += 1;
                        let _ = client.update_document(&doc).await;
                        let _ = client.sync().await;
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
            client
        })
    }).collect();
    
    // Wait for all updates to complete
    let clients: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    
    // Final sync for all
    for client in &clients {
        client.sync().await.unwrap();
    }
    
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

integration_test!(test_vector_clock_convergence, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let client1 = ctx.create_test_client(user_id, token).await;
    let client2 = ctx.create_test_client(user_id, token).await;
    let client3 = ctx.create_test_client(user_id, token).await;
    
    // Create document on client 1
    let doc = TestContext::create_test_document(user_id, "Vector Clock Test");
    client1.create_document(doc.clone()).await.unwrap();
    
    // Client 2 and 3 sync
    client2.sync().await.unwrap();
    client3.sync().await.unwrap();
    
    // Each client makes updates in isolation
    for i in 0..3 {
        // Client 1 update
        let docs = client1.get_all_documents().await.unwrap();
        let mut doc1 = docs[0].clone();
        doc1.content = json!({"client1_update": i});
        doc1.version += 1;
        client1.update_document(&doc1).await.unwrap();
        
        // Client 2 update
        let docs = client2.get_all_documents().await.unwrap();
        let mut doc2 = docs[0].clone();
        doc2.content = json!({"client2_update": i});
        doc2.version += 1;
        client2.update_document(&doc2).await.unwrap();
        
        // Client 3 update
        let docs = client3.get_all_documents().await.unwrap();
        let mut doc3 = docs[0].clone();
        doc3.content = json!({"client3_update": i});
        doc3.version += 1;
        client3.update_document(&doc3).await.unwrap();
    }
    
    // All sync multiple times to ensure convergence
    for _ in 0..3 {
        client1.sync().await.unwrap();
        client2.sync().await.unwrap();
        client3.sync().await.unwrap();
    }
    
    // All should have the same final state
    let docs1 = client1.get_all_documents().await.unwrap();
    let docs2 = client2.get_all_documents().await.unwrap();
    let docs3 = client3.get_all_documents().await.unwrap();
    
    assert_eq!(docs1[0].content, docs2[0].content);
    assert_eq!(docs2[0].content, docs3[0].content);
    assert_eq!(docs1[0].version, docs2[0].version);
    assert_eq!(docs2[0].version, docs3[0].version);
});