use axum::extract::ws::{WebSocket, Message};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use uuid::Uuid;
use sync_core::protocol::{ClientMessage, ServerMessage};
use crate::{AppState, sync_handler::SyncHandler};

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
        tracing::info!("SERVER: WebSocket sender task started for client {}", client_id_clone);
        while let Some(msg) = rx.recv().await {
            // Log outgoing message if monitoring is enabled
            if let Some(ref monitoring) = monitoring_clone {
                monitoring.log_message_sent(&client_id_clone, msg.clone()).await;
            }
            
            tracing::info!("SERVER: Sending message to client {}: {:?}", client_id_clone, std::mem::discriminant(&msg));
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                tracing::error!("SERVER: Failed to send WebSocket message to client {}", client_id_clone);
                break;
            } else {
                tracing::info!("SERVER: Successfully sent WebSocket message to client {}", client_id_clone);
            }
        }
        tracing::warn!("SERVER: WebSocket sender task terminated for client {}", client_id_clone);
    });
    
    let mut handler = SyncHandler::new(state.db.clone(), tx.clone(), state.monitoring.clone(), state.clone());
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
                                    
                                    // Register client in the registry (thread-safe)
                                    state.clients.entry(user_id)
                                        .and_modify(|clients| {
                                            clients.push(tx.clone());
                                            tracing::info!("Added client to existing list, now {} clients for user {}", clients.len(), user_id);
                                        })
                                        .or_insert_with(|| {
                                            tracing::info!("First client for user {}", user_id);
                                            vec![tx.clone()]
                                        });
                                    
                                    // Log total client count
                                    let client_count = state.clients.get(&user_id).map(|c| c.len()).unwrap_or(0);
                                    tracing::info!("User {} now has {} total connected clients", user_id, client_count);
                                    
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
        tracing::debug!("Client disconnecting for user {}", user_id);
        state.db.remove_active_connection(&user_id).await.ok();
        
        // Remove client from registry - properly clean up the disconnected client
        if let Some(mut clients) = state.clients.get_mut(&user_id) {
            let before_count = clients.len();
            // Remove the disconnected client by testing if we can send to it
            clients.retain(|client_tx| {
                // Try to send a ping message - if it fails, the channel is dead
                client_tx.try_send(ServerMessage::Pong).is_ok()
            });
            let after_count = clients.len();
            tracing::debug!("Cleaned up dead clients for user {}: {} -> {} clients", user_id, before_count, after_count);
            
            // If no clients left, remove the user entry entirely
            if clients.is_empty() {
                drop(clients); // Release the lock
                state.clients.remove(&user_id);
                tracing::debug!("No more clients for user {}, removed from registry", user_id);
            }
        }
    }
    
    // Log disconnection if monitoring is enabled
    if let Some(ref monitoring) = state.monitoring {
        monitoring.log_client_disconnected(&client_id).await;
    }
}