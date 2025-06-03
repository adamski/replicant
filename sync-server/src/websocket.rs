use axum::extract::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::collections::HashSet;
use uuid::Uuid;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::{AppState, sync_handler::SyncHandler};

pub async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    let connection_id = Uuid::new_v4().to_string();
    
    // Log connection if monitoring is enabled
    if let Some(ref monitoring) = state.monitoring {
        monitoring.log_client_connected(&connection_id).await;
    }
    
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerMessage>(100);
    
    // Spawn task to forward messages to WebSocket
    let monitoring_clone = state.monitoring.clone();
    let connection_id_clone = connection_id.clone();
    tokio::spawn(async move {
        tracing::info!("SERVER: WebSocket sender task started for connection {}", connection_id_clone);
        while let Some(msg) = rx.recv().await {
            // Log outgoing message if monitoring is enabled
            if let Some(ref monitoring) = monitoring_clone {
                monitoring.log_message_sent(&connection_id_clone, msg.clone()).await;
            }
            
            tracing::info!("SERVER: Sending message to connection {}: {:?}", connection_id_clone, std::mem::discriminant(&msg));
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                tracing::error!("SERVER: Failed to send WebSocket message to connection {}", connection_id_clone);
                break;
            } else {
                tracing::info!("SERVER: Successfully sent WebSocket message to connection {}", connection_id_clone);
            }
        }
        tracing::warn!("SERVER: WebSocket sender task terminated for connection {}", connection_id_clone);
    });
    
    let mut handler = SyncHandler::new(state.db.clone(), tx.clone(), state.monitoring.clone(), state.clone());
    let mut authenticated_user_id = None;
    let mut authenticated_client_id = None;
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    // Log incoming message if monitoring is enabled
                    if let Some(ref monitoring) = state.monitoring {
                        monitoring.log_message_received(&connection_id, client_msg.clone()).await;
                    }
                    
                    match client_msg {
                        ClientMessage::Authenticate { user_id, client_id, auth_token } => {
                            match state.auth.verify_token(&user_id, &auth_token).await {
                                Ok(true) => {
                                    authenticated_user_id = Some(user_id);
                                    authenticated_client_id = Some(client_id);
                                    handler.set_user_id(user_id);
                                    handler.set_client_id(client_id);
                                    
                                    // Register client in the registry with both user_id and client_id
                                    state.clients.insert((user_id, client_id), tx.clone());
                                    
                                    // Update user_clients mapping
                                    state.user_clients.entry(user_id)
                                        .and_modify(|clients| {
                                            clients.insert(client_id);
                                        })
                                        .or_insert_with(|| {
                                            let mut set = HashSet::new();
                                            set.insert(client_id);
                                            set
                                        });
                                    
                                    // Log total client count
                                    let client_count = state.user_clients.get(&user_id).map(|c| c.len()).unwrap_or(0);
                                    tracing::info!("User {} now has {} total connected clients", user_id, client_count);
                                    
                                    let _ = tx.send(ServerMessage::AuthSuccess {
                                        session_id: Uuid::new_v4(),
                                        client_id,
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
                                let _ = tx.send(ServerMessage::Error {
                                    code: sync_core::protocol::ErrorCode::ServerError,
                                    message: format!("Failed to process message: {}", e),
                                }).await;
                                if let Some(ref monitoring) = state.monitoring {
                                    monitoring.log_error(format!("Error handling message: {}", e)).await;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to parse client message: {}", e);
                    let _ = tx.send(ServerMessage::Error {
                        code: sync_core::protocol::ErrorCode::InvalidMessage,
                        message: format!("Invalid JSON: {}", e),
                    }).await;
                }
            }
        }
    }
    
    // Clean up on disconnect
    if let (Some(user_id), Some(client_id)) = (authenticated_user_id, authenticated_client_id) {
        tracing::debug!("Client {} disconnecting for user {}", client_id, user_id);
        state.db.remove_active_connection(&user_id).await.ok();
        
        // Remove client from registry
        state.clients.remove(&(user_id, client_id));
        
        // Update user_clients mapping
        if let Some(mut clients) = state.user_clients.get_mut(&user_id) {
            clients.remove(&client_id);
            if clients.is_empty() {
                drop(clients); // Release the lock
                state.user_clients.remove(&user_id);
                tracing::debug!("No more clients for user {}, removed from registry", user_id);
            }
        }
    }
    
    // Log disconnection if monitoring is enabled
    if let Some(ref monitoring) = state.monitoring {
        monitoring.log_client_disconnected(&connection_id).await;
    }
}