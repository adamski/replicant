use crate::integration::helpers::*;
use uuid::Uuid;
use futures_util::{StreamExt, SinkExt};
use tungstenite::Message;
use sync_core::protocol::{SyncMessage, SyncRequest, SyncResponse};
use serde_json::json;

integration_test!(test_websocket_connection_lifecycle, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Connect to WebSocket
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send a ping
    ws.send(Message::Ping(vec![1, 2, 3])).await.unwrap();
    
    // Should receive pong
    if let Some(Ok(msg)) = ws.next().await {
        match msg {
            Message::Pong(data) => assert_eq!(data, vec![1, 2, 3]),
            _ => panic!("Expected pong message"),
        }
    }
    
    // Close connection gracefully
    ws.send(Message::Close(None)).await.unwrap();
});

integration_test!(test_websocket_message_protocol, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send a sync request
    let request = SyncMessage::Request(SyncRequest::GetDocuments);
    let json_msg = serde_json::to_string(&request).unwrap();
    ws.send(Message::Text(json_msg)).await.unwrap();
    
    // Should receive response
    if let Some(Ok(Message::Text(response))) = ws.next().await {
        let msg: SyncMessage = serde_json::from_str(&response).unwrap();
        match msg {
            SyncMessage::Response(SyncResponse::Documents(_)) => {
                // Expected response type
            }
            _ => panic!("Expected Documents response"),
        }
    }
    
    ws.close(None).await.unwrap();
});

integration_test!(test_websocket_reconnection, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create client and add document
    let client = ctx.create_test_client(user_id, token).await;
    let doc = TestContext::create_test_document(user_id, "Reconnection Test");
    client.create_document(doc.clone()).await.unwrap();
    
    // Connect first WebSocket
    let mut ws1 = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Request documents
    let request = SyncMessage::Request(SyncRequest::GetDocuments);
    ws1.send(Message::Text(serde_json::to_string(&request).unwrap())).await.unwrap();
    
    // Verify we get the document
    if let Some(Ok(Message::Text(response))) = ws1.next().await {
        let msg: SyncMessage = serde_json::from_str(&response).unwrap();
        match msg {
            SyncMessage::Response(SyncResponse::Documents(docs)) => {
                assert_eq!(docs.len(), 1);
            }
            _ => panic!("Expected Documents response"),
        }
    }
    
    // Close first connection
    ws1.close(None).await.unwrap();
    
    // Connect second WebSocket (reconnection)
    let mut ws2 = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Should still be able to get documents
    ws2.send(Message::Text(serde_json::to_string(&request).unwrap())).await.unwrap();
    
    if let Some(Ok(Message::Text(response))) = ws2.next().await {
        let msg: SyncMessage = serde_json::from_str(&response).unwrap();
        match msg {
            SyncMessage::Response(SyncResponse::Documents(docs)) => {
                assert_eq!(docs.len(), 1);
                assert_eq!(docs[0].title, "Reconnection Test");
            }
            _ => panic!("Expected Documents response"),
        }
    }
    
    ws2.close(None).await.unwrap();
});

integration_test!(test_websocket_invalid_messages, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send invalid JSON
    ws.send(Message::Text("not valid json".to_string())).await.unwrap();
    
    // Should receive error response
    if let Some(Ok(Message::Text(response))) = ws.next().await {
        let msg: SyncMessage = serde_json::from_str(&response).unwrap();
        match msg {
            SyncMessage::Response(SyncResponse::Error { message, .. }) => {
                assert!(message.contains("Failed to parse") || message.contains("Invalid"));
            }
            _ => panic!("Expected error response for invalid JSON"),
        }
    }
    
    // Send valid JSON but invalid message structure
    ws.send(Message::Text(json!({"invalid": "structure"}).to_string())).await.unwrap();
    
    if let Some(Ok(Message::Text(response))) = ws.next().await {
        let msg: SyncMessage = serde_json::from_str(&response).unwrap();
        match msg {
            SyncMessage::Response(SyncResponse::Error { .. }) => {
                // Expected error
            }
            _ => panic!("Expected error response for invalid message structure"),
        }
    }
    
    // Connection should still be alive after errors
    ws.send(Message::Ping(vec![])).await.unwrap();
    if let Some(Ok(msg)) = ws.next().await {
        assert!(matches!(msg, Message::Pong(_)));
    }
    
    ws.close(None).await.unwrap();
});

integration_test!(test_websocket_rate_limiting, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send many requests rapidly
    let request = SyncMessage::Request(SyncRequest::GetDocuments);
    let json_msg = serde_json::to_string(&request).unwrap();
    
    for _ in 0..100 {
        ws.send(Message::Text(json_msg.clone())).await.unwrap();
    }
    
    // Should handle all requests (or implement rate limiting)
    let mut response_count = 0;
    let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(5));
    tokio::pin!(timeout);
    
    loop {
        tokio::select! {
            msg = ws.next() => {
                match msg {
                    Some(Ok(Message::Text(_))) => {
                        response_count += 1;
                        if response_count >= 100 {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        // Server might close connection due to rate limiting
                        break;
                    }
                    _ => {}
                }
            }
            _ = &mut timeout => {
                // Timeout is acceptable - server might be rate limiting
                break;
            }
        }
    }
    
    // Should have received some responses (exact count depends on rate limiting)
    assert!(response_count > 0, "Should receive at least some responses");
});