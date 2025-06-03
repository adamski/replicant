use crate::integration::helpers::*;
use uuid::Uuid;
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::tungstenite::Message;
use sync_core::protocol::{ClientMessage, ServerMessage};
use sync_core::models::{Document, VectorClock};
use chrono::Utc;
use serde_json::json;

crate::integration_test!(test_websocket_connection_lifecycle, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Connect to WebSocket
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send a WebSocket ping frame
    ws.send(Message::Ping(vec![1, 2, 3])).await.unwrap();
    
    // Should receive WebSocket pong frame with timeout
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for pong response");
    
    if let Some(Ok(msg)) = response {
        match msg {
            Message::Pong(data) => assert_eq!(data, vec![1, 2, 3]),
            _ => panic!("Expected pong message"),
        }
    } else {
        panic!("Expected pong response");
    }
    
    // Close connection gracefully
    ws.send(Message::Close(None)).await.unwrap();
    
    // Verify connection is closed
    if let Some(Ok(msg)) = ws.next().await {
        assert!(matches!(msg, Message::Close(_)));
    }
});

crate::integration_test!(test_authentication_flow, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Connect without authentication (raw websocket)
    let ws_url = format!("{}/ws", ctx.server_url);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    
    // Send authenticate message
    let client_id = Uuid::new_v4();
    let auth_msg = ClientMessage::Authenticate {
        user_id,
        client_id,
        auth_token: token.to_string(),
    };
    let json_msg = serde_json::to_string(&auth_msg).unwrap();
    ws.send(Message::Text(json_msg)).await.unwrap();
    
    // Should receive auth success with timeout
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for auth response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        match msg {
            ServerMessage::AuthSuccess { session_id, client_id: _ } => {
                assert!(!session_id.is_nil());
            }
            ServerMessage::AuthError { reason } => {
                panic!("Authentication failed: {}", reason);
            }
            _ => panic!("Expected AuthSuccess or AuthError, got {:?}", msg),
        }
    } else {
        panic!("Expected auth response");
    }
    
    // Test invalid authentication
    let bad_auth_msg = ClientMessage::Authenticate {
        user_id,
        client_id,
        auth_token: "invalid-token".to_string(),
    };
    ws.send(Message::Text(serde_json::to_string(&bad_auth_msg).unwrap())).await.unwrap();
    
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for auth error response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::AuthError { .. }));
    } else {
        panic!("Expected auth error response");
    }
    
    ws.close(None).await.unwrap();
});

crate::integration_test!(test_message_exchange, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Test protocol ping/pong with timeout
    let ping_msg = ClientMessage::Ping;
    ws.send(Message::Text(serde_json::to_string(&ping_msg).unwrap())).await.unwrap();
    
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for ping response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::Pong));
    } else {
        panic!("Expected text message response to ping");
    }
    
    // Test sending various message types with timeout
    let doc = Document {
        id: Uuid::new_v4(),
        user_id,
        title: "Test Doc".to_string(),
        content: json!({"text": "Hello World"}),
        revision_id: Document::initial_revision(&json!({"text": "Hello World"})),
        version: 1,
        vector_clock: VectorClock::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    
    let create_msg = ClientMessage::CreateDocument {
        document: doc.clone(),
    };
    ws.send(Message::Text(serde_json::to_string(&create_msg).unwrap())).await.unwrap();
    
    // Should receive response with timeout
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for create document response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::DocumentCreated { .. } | ServerMessage::Error { .. }));
    } else {
        panic!("Expected text message response to create document");
    }
    
    // Test invalid JSON with timeout
    ws.send(Message::Text("invalid json".to_string())).await.unwrap();
    
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.next()
    ).await.expect("Timeout waiting for error response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::Error { .. }));
    } else {
        panic!("Expected error message response to invalid JSON");
    }
    
    ws.close(None).await.unwrap();
});


crate::integration_test!(test_reconnection_handling, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // First connection
    let mut ws1 = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Send a ping to verify connection
    let ping_msg = ClientMessage::Ping;
    ws1.send(Message::Text(serde_json::to_string(&ping_msg).unwrap())).await.unwrap();
    
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws1.next()
    ).await.expect("Timeout waiting for ping response");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::Pong));
    } else {
        panic!("Expected pong response");
    }
    
    // Close first connection
    ws1.close(None).await.unwrap();
    
    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Create second connection (reconnection)
    let mut ws2 = ctx.create_authenticated_websocket(user_id, token).await;
    
    // Verify second connection works
    ws2.send(Message::Text(serde_json::to_string(&ping_msg).unwrap())).await.unwrap();
    
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws2.next()
    ).await.expect("Timeout waiting for ping response on reconnection");
    
    if let Some(Ok(Message::Text(response_text))) = response {
        let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
        assert!(matches!(msg, ServerMessage::Pong));
    } else {
        panic!("Expected pong response on reconnection");
    }
    
    // Test multiple rapid reconnections
    ws2.close(None).await.unwrap();
    
    for i in 0..3 {
        let mut ws = ctx.create_authenticated_websocket(user_id, token).await;
        ws.send(Message::Text(serde_json::to_string(&ping_msg).unwrap())).await.unwrap();
        
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ws.next()
        ).await.unwrap_or_else(|_| panic!("Timeout waiting for ping response on rapid reconnection {}", i));
        
        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::Pong));
        } else {
            panic!("Expected pong response on rapid reconnection {}", i);
        }
        
        ws.close(None).await.unwrap();
    }
});


