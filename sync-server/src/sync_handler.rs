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
    client_id: Option<Uuid>,
    monitoring: Option<MonitoringLayer>,
    app_state: Arc<AppState>,
}

impl SyncHandler {
    pub fn new(db: Arc<ServerDatabase>, tx: mpsc::Sender<ServerMessage>, monitoring: Option<MonitoringLayer>, app_state: Arc<AppState>) -> Self {
        Self {
            db,
            tx,
            user_id: None,
            client_id: None,
            monitoring,
            app_state,
        }
    }
    
    pub fn set_user_id(&mut self, user_id: Uuid) {
        self.user_id = Some(user_id);
    }
    
    pub fn set_client_id(&mut self, client_id: Uuid) {
        self.client_id = Some(client_id);
    }
    
    pub async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let user_id = self.user_id.ok_or("Not authenticated")?;
        
        match msg {
            ClientMessage::CreateDocument { document } => {
                tracing::info!("🔵 Received CreateDocument from user {} for doc {} (rev: {})", 
                             user_id, document.id, document.revision_id);
                
                // Validate ownership
                if document.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot create document for another user").await?;
                    return Ok(());
                }
                
                // Check if document already exists (conflict detection)
                match self.db.get_document(&document.id).await {
                    Ok(existing_doc) => {
                        // Document exists! This is a conflict - handle it
                        tracing::warn!("🔥 CONFLICT DETECTED: Document {} already exists on server", document.id);
                        tracing::warn!("   Server revision: {} | Client revision: {}", 
                                     existing_doc.revision_id, document.revision_id);
                        tracing::warn!("   Server content: {:?}", existing_doc.content);
                        tracing::warn!("   Client content: {:?}", document.content);
                        
                        // Apply last-write-wins strategy (client version replaces server version entirely)
                        // Note: This is NOT a merge - server version is completely overwritten
                        tracing::info!("🔧 Applying last-write-wins: Client version will replace server version");

                        // Use single transaction for atomicity - log conflict AND update document
                        let result = async {
                            let mut tx = self.db.pool.begin().await
                                .map_err(|e| format!("Failed to begin transaction: {}", e))?;

                            // Log server's version as conflict loser (applied=false)
                            let server_content_json = serde_json::to_value(&existing_doc.content)
                                .map_err(|e| format!("Failed to serialize server content: {}", e))?;

                            self.db.log_change_event(
                                &mut tx,
                                crate::database::ChangeEventParams {
                                    document_id: &document.id,
                                    user_id: &user_id,
                                    event_type: sync_core::protocol::ChangeEventType::Create,
                                    revision_id: &existing_doc.revision_id,
                                    forward_patch: Some(&server_content_json),
                                    reverse_patch: None,
                                    applied: false,
                                }
                            ).await
                            .map_err(|e| format!("Failed to log conflict: {}", e))?;

                            tracing::info!("📝 Logged server version as conflict loser (rev: {})", existing_doc.revision_id);

                            // Update document to client version IN SAME TRANSACTION
                            self.db.update_document_in_tx(&mut tx, &document, None).await
                                .map_err(|e| format!("Failed to update document: {}", e))?;

                            // Commit both operations atomically
                            tx.commit().await
                                .map_err(|e| format!("Failed to commit transaction: {}", e))?;

                            Ok::<(), String>(())
                        }.await;

                        match result {
                            Ok(_) => {
                                tracing::info!("✅ Client version applied (server version overwritten)");

                                // Send confirmation to the sender
                                self.tx.send(ServerMessage::DocumentCreatedResponse {
                                    document_id: document.id,
                                    revision_id: document.revision_id.clone(),
                                    success: true,
                                    error: None,
                                }).await?;

                                // Broadcast the client's version to ALL clients for consistency
                                tracing::info!("📡 Broadcasting client's version to all clients");
                                self.broadcast_to_user(
                                    user_id,
                                    ServerMessage::SyncDocument { document: document.clone() }
                                ).await?;
                            }
                            Err(e) => {
                                tracing::error!("❌ Failed to apply conflict resolution: {}", e);
                                self.tx.send(ServerMessage::DocumentCreatedResponse {
                                    document_id: document.id,
                                    revision_id: document.revision_id.clone(),
                                    success: false,
                                    error: Some(e),
                                }).await?;
                            }
                        }
                    }
                    Err(_) => {
                        // Document doesn't exist - this is a true create operation
                        tracing::info!("📝 Creating new document {} ", document.id);
                        
                        match self.db.create_document(&document).await {
                            Ok(_) => {
                                // Send confirmation to the sender
                                self.tx.send(ServerMessage::DocumentCreatedResponse {
                                    document_id: document.id,
                                    revision_id: document.revision_id.clone(),
                                    success: true,
                                    error: None,
                                }).await?;
                                
                                // Broadcast to all OTHER connected clients (exclude sender)
                                tracing::info!("📡 Broadcasting new document to other clients");
                                self.broadcast_to_user_except(
                                    user_id,
                                    self.client_id,
                                    ServerMessage::DocumentCreated { document }
                                ).await?;
                            }
                            Err(e) => {
                                // Send error response to the sender
                                self.tx.send(ServerMessage::DocumentCreatedResponse {
                                    document_id: document.id,
                                    revision_id: document.revision_id.clone(),
                                    success: false,
                                    error: Some(e.to_string()),
                                }).await?;
                            }
                        }
                    }
                }
            }
            
            ClientMessage::UpdateDocument { patch } => {
                tracing::info!("🔵 Received UpdateDocument from client {} for doc {}", 
                             self.client_id.unwrap_or_default(), patch.document_id);
                tracing::info!("   Patch content: {:?}", patch.patch);
                
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

                    // Last-write-wins strategy: Client patch overwrites server state
                    // Note: This applies the patch to current server state, not merging changes
                    tracing::warn!("🔥 CONFLICT DETECTED for document {}", doc.id);
                    tracing::warn!("   Server revision: {} | Client revision: {}", doc.revision_id, patch.revision_id);
                    tracing::warn!("   Server content before: {:?}", doc.content);
                    tracing::warn!("   Client patch: {:?}", patch.patch);

                    // Store the server's current state as conflict loser BEFORE applying client patch
                    let server_content_before = doc.content.clone();
                    let server_revision_before = doc.revision_id.clone();

                    // Apply the client's patch (client wins)
                    apply_patch(&mut doc.content, &patch.patch)?;

                    // Update revision and vector clock
                    let old_revision = doc.revision_id.clone();
                    doc.revision_id = doc.next_revision(&doc.content);
                    doc.vector_clock.increment(&self.user_id.ok_or("Not authenticated")?.to_string());
                    doc.updated_at = chrono::Utc::now();

                    tracing::warn!("   Server content after conflict resolution: {:?}", doc.content);
                    tracing::warn!("   New revision: {} -> {}", old_revision, doc.revision_id);

                    // Use single transaction to log conflict AND update document atomically
                    let result = async {
                        let mut tx = self.db.pool.begin().await
                            .map_err(|e| format!("Failed to begin transaction: {}", e))?;

                        // Log server's pre-conflict state as unapplied (conflict loser)
                        let server_content_json = serde_json::to_value(&server_content_before)
                            .map_err(|e| format!("Failed to serialize server content: {}", e))?;

                        self.db.log_change_event(
                            &mut tx,
                            crate::database::ChangeEventParams {
                                document_id: &doc.id,
                                user_id: &user_id,
                                event_type: sync_core::protocol::ChangeEventType::Update,
                                revision_id: &server_revision_before,
                                forward_patch: Some(&server_content_json),
                                reverse_patch: None,
                                applied: false,
                            }
                        ).await
                        .map_err(|e| format!("Failed to log conflict: {}", e))?;

                        tracing::info!("📝 Logged server state as conflict loser (rev: {})", server_revision_before);

                        // Update document with client's changes IN SAME TRANSACTION
                        self.db.update_document_in_tx(&mut tx, &doc, Some(&patch.patch)).await
                            .map_err(|e| format!("Failed to update document: {}", e))?;

                        // Commit both operations atomically
                        tx.commit().await
                            .map_err(|e| format!("Failed to commit transaction: {}", e))?;

                        Ok::<(), String>(())
                    }.await;

                    match result {
                        Ok(_) => {
                            // Notify client of conflict resolution
                            self.tx.send(ServerMessage::ConflictDetected {
                                document_id: patch.document_id,
                                local_revision: patch.revision_id.clone(),
                                server_revision: doc.revision_id.clone(),
                                resolution_strategy: ConflictResolution::ServerWins,
                            }).await?;
                        }
                        Err(e) => {
                            tracing::error!("❌ Conflict resolution failed: {}", e);
                            // Notify client that the update failed
                            self.tx.send(ServerMessage::ConflictDetected {
                                document_id: patch.document_id,
                                local_revision: patch.revision_id.clone(),
                                server_revision: doc.revision_id.clone(),
                                resolution_strategy: ConflictResolution::ServerWins,
                            }).await?;
                            return Ok(());
                        }
                    }
                    
                    // Broadcast the final state to ALL clients to ensure convergence
                    // This ensures all clients get the authoritative state after conflict resolution
                    tracing::info!("🔸 Broadcasting final document state after conflict resolution");
                    self.broadcast_to_user_except(
                        user_id,
                        self.client_id,
                        ServerMessage::SyncDocument { document: doc.clone() }
                    ).await?;
                    
                    return Ok(());
                }
                
                // Apply patch (no conflict)
                tracing::info!("📝 NORMAL UPDATE for document {}", doc.id);
                tracing::info!("   Revision: {} | Content before: {:?}", doc.revision_id, doc.content);
                tracing::info!("   Patch: {:?}", patch.patch);
                
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
                doc.revision_id = patch.revision_id.clone();
                doc.version += 1;
                doc.vector_clock.merge(&patch.vector_clock);
                doc.updated_at = chrono::Utc::now();
                
                // Save to database
                match self.db.update_document(&doc, Some(&patch.patch)).await {
                    Ok(_) => {
                        tracing::info!("   Content after normal update: {:?}", doc.content);
                        tracing::info!("   New revision: {}", doc.revision_id);
                        
                        // Send confirmation to the sender
                        self.tx.send(ServerMessage::DocumentUpdatedResponse {
                            document_id: doc.id,
                            revision_id: doc.revision_id.clone(),
                            success: true,
                            error: None,
                        }).await?;
                        
                        // Broadcast final state to ALL OTHER clients to ensure convergence
                        tracing::info!("Broadcasting final document state for doc {} to other clients of user {}", doc.id, user_id);
                        self.broadcast_to_user_except(
                            user_id,
                            self.client_id,
                            ServerMessage::SyncDocument { document: doc.clone() }
                        ).await?;
                    }
                    Err(e) => {
                        // Send error response to the sender
                        self.tx.send(ServerMessage::DocumentUpdatedResponse {
                            document_id: patch.document_id,
                            revision_id: patch.revision_id.clone(),
                            success: false,
                            error: Some(e.to_string()),
                        }).await?;
                    }
                }
            }
            
            ClientMessage::DeleteDocument { document_id, revision_id } => {
                let doc = self.db.get_document(&document_id).await?;
                
                if doc.user_id != user_id {
                    self.send_error(ErrorCode::InvalidAuth, "Cannot delete another user's document").await?;
                    return Ok(());
                }
                
                // Soft delete
                match self.db.delete_document(&document_id, &user_id, &revision_id).await {
                    Ok(_) => {
                        // Send confirmation to the sender
                        self.tx.send(ServerMessage::DocumentDeletedResponse {
                            document_id,
                            revision_id: revision_id.clone(),
                            success: true,
                            error: None,
                        }).await?;
                        
                        // Broadcast deletion to all OTHER connected clients
                        self.broadcast_to_user_except(
                            user_id,
                            self.client_id,
                            ServerMessage::DocumentDeleted { document_id, revision_id }
                        ).await?;
                    }
                    Err(e) => {
                        // Send error response to the sender
                        self.tx.send(ServerMessage::DocumentDeletedResponse {
                            document_id,
                            revision_id: revision_id.clone(),
                            success: false,
                            error: Some(e.to_string()),
                        }).await?;
                    }
                }
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
                    tracing::info!("📤 SENDING SyncDocument: {} | Title: {} | Rev: {}", 
                                 doc.id, 
                                 doc.content.get("title").and_then(|v| v.as_str()).unwrap_or("N/A"),
                                 doc.revision_id);
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
            
            ClientMessage::GetChangesSince { .. } => {
                // TODO: Implement sequence-based sync
                self.send_error(ErrorCode::ServerError, "Sequence-based sync not yet implemented").await?;
            }
            
            ClientMessage::AckChanges { .. } => {
                // TODO: Implement change acknowledgment
                self.send_error(ErrorCode::ServerError, "Change acknowledgment not yet implemented").await?;
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
        self.broadcast_to_user_except(user_id, None, message).await
    }
    
    async fn broadcast_to_user_except(
        &self,
        user_id: Uuid,
        exclude_client_id: Option<Uuid>,
        message: ServerMessage,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get all connected client IDs for this user
        if let Some(client_ids) = self.app_state.user_clients.get(&user_id) {
            let total_clients = client_ids.len();
            let excluded = if exclude_client_id.is_some() { 1 } else { 0 };
            tracing::info!("Broadcasting message to {}/{} clients for user {}", total_clients - excluded, total_clients, user_id);
            
            let mut dead_clients = Vec::new();
            let mut successful_sends = 0;
            let mut skipped = 0;
            
            // Send message to all clients for this user except the excluded one
            for client_id in client_ids.iter() {
                // Skip if this is the client to exclude
                if let Some(exclude_id) = exclude_client_id {
                    if *client_id == exclude_id {
                        skipped += 1;
                        tracing::info!("Skipping broadcast to sender client {} for user {}", client_id, user_id);
                        continue;
                    }
                }
                
                if let Some(client_tx) = self.app_state.clients.get(&(user_id, *client_id)) {
                    if client_tx.send(message.clone()).await.is_err() {
                        // Client disconnected, mark for removal
                        dead_clients.push(*client_id);
                        tracing::warn!("Failed to send to client {} for user {}", client_id, user_id);
                    } else {
                        successful_sends += 1;
                        tracing::debug!("Successfully sent message to client {} for user {}", client_id, user_id);
                    }
                } else {
                    // Client not found in registry - this shouldn't happen
                    dead_clients.push(*client_id);
                    tracing::warn!("Client {} not found in registry for user {}", client_id, user_id);
                }
            }
            
            tracing::info!("Successfully sent to {}/{} clients for user {} (skipped {})", 
                         successful_sends, total_clients - skipped, user_id, skipped);
            
            // Remove dead clients
            if !dead_clients.is_empty() {
                drop(client_ids); // Release the read lock
                if let Some(mut client_ids_mut) = self.app_state.user_clients.get_mut(&user_id) {
                    for dead_client_id in &dead_clients {
                        client_ids_mut.remove(dead_client_id);
                        self.app_state.clients.remove(&(user_id, *dead_client_id));
                    }
                    
                    // Remove user entry if no clients left
                    if client_ids_mut.is_empty() {
                        drop(client_ids_mut);
                        self.app_state.user_clients.remove(&user_id);
                    }
                }
            }
        }
        
        Ok(())
    }

}