use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::{
    protocol::{ClientMessage, ServerMessage, ConflictResolution, ErrorCode},
    patches::{apply_patch, calculate_checksum},
};
use crate::{database::ServerDatabase, monitoring::MonitoringLayer, AppState};

pub struct SyncHandler {
    db: Arc<ServerDatabase>,
    tx: mpsc::Sender<ServerMessage>,
    user_id: Option<Uuid>,
    monitoring: Option<MonitoringLayer>,
    app_state: Arc<AppState>,
}

impl SyncHandler {
    pub fn new(db: Arc<ServerDatabase>, tx: mpsc::Sender<ServerMessage>, monitoring: Option<MonitoringLayer>, app_state: Arc<AppState>) -> Self {
        Self {
            db,
            tx,
            user_id: None,
            monitoring,
            app_state,
        }
    }
    
    pub fn set_user_id(&mut self, user_id: Uuid) {
        self.user_id = Some(user_id);
    }
    
    pub async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let user_id = self.user_id.ok_or("Not authenticated")?;
        
        match msg {
            ClientMessage::CreateDocument { document } => {
                tracing::debug!("Received CreateDocument from user {} for doc {}", user_id, document.id);
                // Validate ownership
                if document.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot create document for another user").await?;
                    return Ok(());
                }
                
                // Save to database
                self.db.create_document(&document).await?;
                
                // Broadcast to all connected clients (including sender)
                tracing::info!("Broadcasting DocumentCreated for doc {} to all clients of user {}", document.id, user_id);
                
                // Log current client count
                if let Some(clients) = self.app_state.clients.get(&user_id) {
                    tracing::info!("User {} has {} connected clients", user_id, clients.len());
                } else {
                    tracing::warn!("User {} has no registered clients!", user_id);
                }
                
                self.broadcast_to_user(
                    user_id,
                    ServerMessage::DocumentCreated { document }
                ).await?;
            }
            
            ClientMessage::UpdateDocument { patch } => {
                // Get current document
                let mut doc = self.db.get_document(&patch.document_id).await?;
                
                // Validate ownership
                if doc.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot update another user's document").await?;
                    return Ok(());
                }
                
                // Check for conflicts
                if doc.vector_clock.is_concurrent(&patch.vector_clock) {
                    // Log conflict if monitoring is enabled
                    if let Some(ref monitoring) = self.monitoring {
                        monitoring.log_conflict_detected(&doc.id.to_string()).await;
                    }
                    
                    // Conflict detected
                    self.tx.send(ServerMessage::ConflictDetected {
                        document_id: patch.document_id,
                        local_revision: patch.revision_id,
                        server_revision: doc.revision_id,
                        resolution_strategy: ConflictResolution::Manual {
                            server_document: doc.clone(),
                            client_patch: patch.clone(),
                        },
                    }).await?;
                    return Ok(());
                }
                
                // Apply patch
                apply_patch(&mut doc.content, &patch.patch)?;
                
                // Log patch applied if monitoring is enabled
                if let Some(ref monitoring) = self.monitoring {
                    let patch_json = serde_json::to_value(&patch.patch).unwrap_or_default();
                    monitoring.log_patch_applied(&doc.id.to_string(), &patch_json).await;
                }
                
                // Verify checksum
                let calculated_checksum = calculate_checksum(&doc.content);
                if calculated_checksum != patch.checksum {
                    self.send_error(ErrorCode::InvalidPatch, "Checksum mismatch").await?;
                    return Ok(());
                }
                
                // Update metadata
                doc.revision_id = patch.revision_id;
                doc.version += 1;
                doc.vector_clock.merge(&patch.vector_clock);
                doc.updated_at = chrono::Utc::now();
                
                // Save to database
                self.db.update_document(&doc).await?;
                self.db.create_revision(&doc, Some(&patch.patch)).await?;
                
                // Broadcast to all connected clients (including sender)
                self.broadcast_to_user(
                    user_id,
                    ServerMessage::DocumentUpdated { patch }
                ).await?;
            }
            
            ClientMessage::DeleteDocument { document_id, revision_id } => {
                let doc = self.db.get_document(&document_id).await?;
                
                if doc.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot delete another user's document").await?;
                    return Ok(());
                }
                
                // Soft delete
                self.db.delete_document(&document_id, &user_id).await?;
                
                // Create revision for the delete operation
                let mut deleted_doc = doc.clone();
                deleted_doc.deleted_at = Some(chrono::Utc::now());
                deleted_doc.revision_id = revision_id;
                self.db.create_revision(&deleted_doc, None).await?;
                
                // Broadcast deletion to all connected clients (including sender)
                self.broadcast_to_user(
                    user_id,
                    ServerMessage::DocumentDeleted { document_id, revision_id }
                ).await?;
            }
            
            ClientMessage::RequestSync { document_ids } => {
                let count = document_ids.len();
                for doc_id in document_ids {
                    if let Ok(doc) = self.db.get_document(&doc_id).await {
                        if doc.user_id == user_id {
                            self.tx.send(ServerMessage::SyncDocument { document: doc }).await?;
                        }
                    }
                }
                
                self.tx.send(ServerMessage::SyncComplete { synced_count: count }).await?;
            }
            
            ClientMessage::RequestFullSync => {
                tracing::debug!("Received RequestFullSync from user {}", user_id);
                let documents = self.db.get_user_documents(&user_id).await?;
                tracing::debug!("Found {} documents for user {}", documents.len(), user_id);
                
                for doc in &documents {
                    tracing::debug!("Sending SyncDocument for doc {}", doc.id);
                    self.tx.send(ServerMessage::SyncDocument { document: doc.clone() }).await?;
                }
                
                self.tx.send(ServerMessage::SyncComplete { synced_count: documents.len() }).await?;
            }
            
            ClientMessage::Ping => {
                self.tx.send(ServerMessage::Pong).await?;
            }
            
            ClientMessage::Authenticate { .. } => {
                // Authentication is handled in the websocket handler
                self.send_error(ErrorCode::InvalidAuth, "Authentication should be handled before this point").await?;
            }
        }
        
        Ok(())
    }
    
    async fn send_error(&self, code: ErrorCode, message: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.tx.send(ServerMessage::Error {
            code,
            message: message.to_string(),
        }).await?;
        Ok(())
    }
    
    async fn broadcast_to_user(
        &self,
        user_id: Uuid,
        message: ServerMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get all connected clients for this user
        if let Some(clients) = self.app_state.clients.get(&user_id) {
            tracing::info!("Broadcasting message to {} clients for user {}", clients.len(), user_id);
            let mut dead_clients = Vec::new();
            let mut successful_sends = 0;
            
            // Send message to all clients for this user
            for (index, client_tx) in clients.iter().enumerate() {
                if client_tx.send(message.clone()).await.is_err() {
                    // Client disconnected, mark for removal
                    dead_clients.push(index);
                    tracing::warn!("Failed to send to client {} for user {}", index, user_id);
                } else {
                    successful_sends += 1;
                }
            }
            
            tracing::info!("Successfully sent to {}/{} clients for user {}", successful_sends, clients.len(), user_id);
            
            // Remove dead clients (in reverse order to maintain indices)
            if !dead_clients.is_empty() {
                drop(clients); // Release the read lock
                if let Some(mut clients_mut) = self.app_state.clients.get_mut(&user_id) {
                    for &index in dead_clients.iter().rev() {
                        if index < clients_mut.len() {
                            clients_mut.remove(index);
                        }
                    }
                    
                    // Remove user entry if no clients left
                    if clients_mut.is_empty() {
                        drop(clients_mut);
                        self.app_state.clients.remove(&user_id);
                    }
                }
            }
        }
        
        Ok(())
    }
}