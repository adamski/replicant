use axum::extract::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use uuid::Uuid;
use dashmap::DashMap;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::{AppState, sync_handler::SyncHandler};

type Clients = Arc<DashMap<Uuid, tokio::sync::mpsc::Sender<ServerMessage>>>;

pub async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerMessage>(100);
    
    // Spawn task to forward messages to WebSocket
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });
    
    let mut handler = SyncHandler::new(state.db.clone(), tx.clone());
    let mut authenticated_user_id = None;
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    match client_msg {
                        ClientMessage::Authenticate { user_id, auth_token } => {
                            match state.auth.verify_token(&user_id, &auth_token).await {
                                Ok(true) => {
                                    authenticated_user_id = Some(user_id);
                                    handler.set_user_id(user_id);
                                    
                                    let _ = tx.send(ServerMessage::AuthSuccess {
                                        session_id: Uuid::new_v4(),
                                    }).await;
                                }
                                _ => {
                                    let _ = tx.send(ServerMessage::AuthError {
                                        reason: "Invalid credentials".to_string(),
                                    }).await;
                                    break;
                                }
                            }
                        }
                        _ => {
                            // Require authentication first
                            if authenticated_user_id.is_none() {
                                let _ = tx.send(ServerMessage::AuthError {
                                    reason: "Not authenticated".to_string(),
                                }).await;
                                break;
                            }
                            
                            // Handle other messages
                            if let Err(e) = handler.handle_message(client_msg).await {
                                tracing::error!("Error handling message: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to parse client message: {}", e);
                }
            }
        }
    }
    
    // Clean up on disconnect
    if let Some(user_id) = authenticated_user_id {
        state.db.remove_active_connection(&user_id).await.ok();
    }
}