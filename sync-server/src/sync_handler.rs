use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::{
    protocol::{ClientMessage, ServerMessage, ConflictResolution, ErrorCode},
    patches::{apply_patch, calculate_checksum},
};
use crate::database::ServerDatabase;

pub struct SyncHandler {
    db: Arc<ServerDatabase>,
    tx: mpsc::Sender<ServerMessage>,
    user_id: Option<Uuid>,
}

impl SyncHandler {
    pub fn new(db: Arc<ServerDatabase>, tx: mpsc::Sender<ServerMessage>) -> Self {
        Self {
            db,
            tx,
            user_id: None,
        }
    }
    
    pub fn set_user_id(&mut self, user_id: Uuid) {
        self.user_id = Some(user_id);
    }
    
    pub async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let user_id = self.user_id.ok_or("Not authenticated")?;
        
        match msg {
            ClientMessage::CreateDocument { document } => {
                // Validate ownership
                if document.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot create document for another user").await?;
                    return Ok(());
                }
                
                // Save to database
                self.db.create_document(&document).await?;
                
                // Broadcast to other connected clients
                self.broadcast_to_user(
                    user_id,
                    ServerMessage::DocumentCreated { document: document.clone() }
                ).await?;
                
                // Confirm to sender
                self.tx.send(ServerMessage::DocumentCreated { document }).await?;
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
                
                // Broadcast to other clients
                self.broadcast_to_user(
                    user_id,
                    ServerMessage::DocumentUpdated { patch: patch.clone() }
                ).await?;
                
                // Confirm to sender
                self.tx.send(ServerMessage::DocumentUpdated { patch }).await?;
            }
            
            ClientMessage::DeleteDocument { document_id, revision_id } => {
                let mut doc = self.db.get_document(&document_id).await?;
                
                if doc.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot delete another user's document").await?;
                    return Ok(());
                }
                
                // Soft delete
                doc.deleted_at = Some(chrono::Utc::now());
                doc.revision_id = revision_id;
                self.db.update_document(&doc).await?;
                
                // Broadcast deletion
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
                let documents = self.db.get_user_documents(&user_id).await?;
                
                for doc in &documents {
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
        _user_id: Uuid,
        _message: ServerMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // In a real implementation, you'd maintain a registry of connected clients
        // and broadcast to all connections for this user except the sender
        Ok(())
    }
}