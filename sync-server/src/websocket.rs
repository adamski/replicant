use axum::extract::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use uuid::Uuid;
use dashmap::DashMap;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::{AppState, sync_handler::SyncHandler};

type Clients = Arc<DashMap<Uuid, tokio::sync::mpsc::Sender<ServerMessage>>>;

pub async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let client_id = Uuid::new_v4().to_string();
    
    // Log connection if monitoring is enabled
    if let Some(ref monitoring) = state.monitoring {
        monitoring.log_client_connected(&client_id).await;
    }
    
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerMessage>(100);
    
    // Spawn task to forward messages to WebSocket
    let monitoring_clone = state.monitoring.clone();
    let client_id_clone = client_id.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            // Log outgoing message if monitoring is enabled
            if let Some(ref monitoring) = monitoring_clone {
                monitoring.log_message_sent(&client_id_clone, msg.clone()).await;
            }
            
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });
    
    let mut handler = SyncHandler::new(state.db.clone(), tx.clone(), state.monitoring.clone());
    let mut authenticated_user_id = None;
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    // Log incoming message if monitoring is enabled
                    if let Some(ref monitoring) = state.monitoring {
                        monitoring.log_message_received(&client_id, client_msg.clone()).await;
                    }
                    
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
                                if let Some(ref monitoring) = state.monitoring {
                                    monitoring.log_error(format!("Error handling message: {}", e)).await;
                                }
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
    
    // Log disconnection if monitoring is enabled
    if let Some(ref monitoring) = state.monitoring {
        monitoring.log_client_disconnected(&client_id).await;
    }
}