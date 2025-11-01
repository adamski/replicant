use crate::integration::helpers::*;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use sync_core::models::{Document, VersionVector};
use sync_core::protocol::{ClientMessage, ServerMessage};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

crate::integration_test!(
    test_websocket_connection_lifecycle,
    |ctx: TestContext| async move {
        let email = "alice@test.local";

        // Generate proper HMAC credentials
        let (api_key, _) = ctx
            .generate_test_credentials("test-alice")
            .await
            .expect("Failed to generate credentials");

        // Create user
        let _ = ctx
            .create_test_user(email)
            .await
            .expect("Failed to create user");

        // Connect to WebSocket
        let mut ws = ctx.create_authenticated_websocket(email, &api_key).await;

        // Send a WebSocket ping frame
        ws.send(Message::Ping(vec![1, 2, 3])).await.unwrap();

        // Should receive WebSocket pong frame with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for pong response");

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
    },
    true
);

crate::integration_test!(
    test_authentication_flow,
    |ctx: TestContext| async move {
        let email = "bob@test.local";

        // Generate proper HMAC credentials
        let (api_key, api_secret) = ctx
            .generate_test_credentials("test-bob")
            .await
            .expect("Failed to generate credentials");

        // Create user
        let _ = ctx
            .create_test_user(email)
            .await
            .expect("Failed to create user");

        // Connect without authentication (raw websocket)
        let ws_url = format!("{}/ws", ctx.server_url);
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let signature = create_hmac_signature(&api_secret, now, email, &api_key, "");
        // Send authenticate message
        let client_id = Uuid::new_v4();
        let auth_msg = ClientMessage::Authenticate {
            email: email.to_string(),
            client_id,
            api_key: Some(api_key.clone()),
            signature: Some(signature),
            timestamp: Some(now),
        };
        let json_msg = serde_json::to_string(&auth_msg).unwrap();
        ws.send(Message::Text(json_msg)).await.unwrap();

        // Should receive auth success with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for auth response");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            match msg {
                ServerMessage::AuthSuccess {
                    session_id,
                    client_id: _,
                } => {
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
        let signature = create_hmac_signature(&email, now, email, &api_key, "");
        // Send authenticate message
        let client_id = Uuid::new_v4();
        let bad_auth_msg = ClientMessage::Authenticate {
            email: email.to_string(),
            client_id,
            api_key: Some(api_key.clone()),
            signature: Some(signature),
            timestamp: Some(now),
        };
        ws.send(Message::Text(serde_json::to_string(&bad_auth_msg).unwrap()))
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for auth error response");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::AuthError { .. }));
        } else {
            panic!("Expected auth error response");
        }

        ws.close(None).await.unwrap();
    },
    true
);

crate::integration_test!(
    test_message_exchange,
    |ctx: TestContext| async move {
        let email = "charlie@test.local";

        // Generate proper HMAC credentials
        let (api_key, _) = ctx
            .generate_test_credentials("test-charlie")
            .await
            .expect("Failed to generate credentials");

        // Create user - need user_id for Document creation below
        let user_id = ctx
            .create_test_user(email)
            .await
            .expect("Failed to create user");

        let mut ws = ctx.create_authenticated_websocket(email, &api_key).await;

        // Test protocol ping/pong with timeout
        let ping_msg = ClientMessage::Ping;
        ws.send(Message::Text(serde_json::to_string(&ping_msg).unwrap()))
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for ping response");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::Pong));
        } else {
            panic!("Expected text message response to ping");
        }

        // Test sending various message types with timeout
        let content = json!({"title": "Test Doc", "text": "Hello World"});
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            content: content.clone(),
            version: 1,
            content_hash: None,
            version_vector: VersionVector::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        let create_msg = ClientMessage::CreateDocument {
            document: doc.clone(),
        };
        ws.send(Message::Text(serde_json::to_string(&create_msg).unwrap()))
            .await
            .unwrap();

        // Should receive response with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for create document response");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::DocumentCreatedResponse { .. }));
        } else {
            panic!("Expected text message response to create document");
        }

        // Test invalid JSON with timeout
        ws.send(Message::Text("invalid json".to_string()))
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("Timeout waiting for error response");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::Error { .. }));
        } else {
            panic!("Expected error message response to invalid JSON");
        }

        ws.close(None).await.unwrap();
    },
    true
);

crate::integration_test!(
    test_reconnection_handling,
    |ctx: TestContext| async move {
        let email = "dave@test.local";

        // Generate proper HMAC credentials
        let (api_key, _) = ctx
            .generate_test_credentials("test-dave")
            .await
            .expect("Failed to generate credentials");

        // Create user
        let _ = ctx
            .create_test_user(email)
            .await
            .expect("Failed to create user");

        // First connection
        let mut ws1 = ctx.create_authenticated_websocket(email, &api_key).await;

        // Send a ping to verify connection
        let ping_msg = ClientMessage::Ping;
        ws1.send(Message::Text(serde_json::to_string(&ping_msg).unwrap()))
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws1.next())
            .await
            .expect("Timeout waiting for ping response");

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
        let mut ws2 = ctx.create_authenticated_websocket(email, &api_key).await;

        // Verify second connection works
        ws2.send(Message::Text(serde_json::to_string(&ping_msg).unwrap()))
            .await
            .unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws2.next())
            .await
            .expect("Timeout waiting for ping response on reconnection");

        if let Some(Ok(Message::Text(response_text))) = response {
            let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
            assert!(matches!(msg, ServerMessage::Pong));
        } else {
            panic!("Expected pong response on reconnection");
        }

        // Test multiple rapid reconnections
        ws2.close(None).await.unwrap();

        for i in 0..3 {
            let mut ws = ctx.create_authenticated_websocket(email, &api_key).await;
            ws.send(Message::Text(serde_json::to_string(&ping_msg).unwrap()))
                .await
                .unwrap();

            let response = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "Timeout waiting for ping response on rapid reconnection {}",
                        i
                    )
                });

            if let Some(Ok(Message::Text(response_text))) = response {
                let msg: ServerMessage = serde_json::from_str(&response_text).unwrap();
                assert!(matches!(msg, ServerMessage::Pong));
            } else {
                panic!("Expected pong response on rapid reconnection {}", i);
            }

            ws.close(None).await.unwrap();
        }
    },
    true
);
