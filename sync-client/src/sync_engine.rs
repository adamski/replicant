use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex, Notify};
use uuid::Uuid;
use sync_core::{
    models::{Document, VectorClock, SyncStatus},
    protocol::{ClientMessage, ServerMessage},
    patches::{create_patch, apply_patch},
};
use crate::{
    database::ClientDatabase,
    websocket::WebSocketClient,
    errors::ClientError,
    events::EventDispatcher,
};

#[derive(Debug, Clone)]
struct PendingUpload {
    document_id: Uuid,
    operation_type: UploadType,
    sent_at: Instant,
}

#[derive(Debug, Clone)]
enum UploadType {
    Create,
    Update,
    Delete,
}

pub struct SyncEngine {
    db: Arc<ClientDatabase>,
    ws_client: Arc<Mutex<Option<WebSocketClient>>>,
    user_id: Uuid,
    client_id: Uuid,
    node_id: String,
    message_rx: Option<mpsc::Receiver<ServerMessage>>,
    event_dispatcher: Arc<EventDispatcher>,
    pending_uploads: Arc<Mutex<HashMap<Uuid, PendingUpload>>>,
    upload_complete_notifier: Arc<Notify>,
    sync_protection_mode: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    server_url: String,
    auth_token: String,
}

impl SyncEngine {
    pub async fn new(
        database_url: &str,
        server_url: &str,
        auth_token: &str,
        user_identifier: &str,
    ) -> Result<Self, ClientError> {
        let db = Arc::new(ClientDatabase::new(database_url).await?);
        db.run_migrations().await?;
        
        // Ensure user_config exists with deterministic user ID based on identifier
        db.ensure_user_config_with_identifier(server_url, auth_token, user_identifier).await?;
        
        let (user_id, client_id) = db.get_user_and_client_id().await?;
        let node_id = format!("client_{}", user_id);
        
        // Create the event dispatcher first
        let event_dispatcher = Arc::new(EventDispatcher::new());
        
        // Create a channel for messages
        let (tx, rx) = mpsc::channel(100);
        
        // Try to connect to WebSocket, but don't fail if offline
        let (ws_client, is_connected) = match WebSocketClient::connect(
            server_url, 
            user_id, 
            client_id, 
            auth_token,
            Some(event_dispatcher.clone())
        ).await {
            Ok((client, receiver)) => {
                // Start forwarding WebSocket messages to our channel
                tokio::spawn(async move {
                    if let Err(e) = receiver.forward_to(tx).await {
                        tracing::error!("WebSocket receiver error: {}", e);
                    }
                });
                (Some(client), true)
            }
            Err(e) => {
                tracing::warn!("Failed to connect to server (will retry): {}", e);
                (None, false)
            }
        };
        
        let engine = Self {
            db,
            ws_client: Arc::new(Mutex::new(ws_client)),
            user_id,
            client_id,
            node_id,
            message_rx: Some(rx),
            event_dispatcher,
            pending_uploads: Arc::new(Mutex::new(HashMap::new())),
            upload_complete_notifier: Arc::new(Notify::new()),
            sync_protection_mode: Arc::new(AtomicBool::new(false)),
            is_connected: Arc::new(AtomicBool::new(is_connected)),
            server_url: server_url.to_string(),
            auth_token: auth_token.to_string(),
        };
        
        Ok(engine)
    }
    
    pub fn event_dispatcher(&self) -> Arc<EventDispatcher> {
        self.event_dispatcher.clone()
    }
    
    pub async fn start(&mut self) -> Result<(), ClientError> {
        // Take the receiver - can only start once
        let rx = self.message_rx.take()
            .ok_or_else(|| ClientError::WebSocket("SyncEngine already started".to_string()))?;
        
        let db = self.db.clone();
        let client_id = self.client_id;
        let event_dispatcher = self.event_dispatcher.clone();
        let pending_uploads = self.pending_uploads.clone();
        let upload_complete_notifier = self.upload_complete_notifier.clone();
        let sync_protection_mode = self.sync_protection_mode.clone();
        
        // Start reconnection monitor if not connected
        if !self.is_connected.load(Ordering::Relaxed) {
            self.start_reconnection_loop();
        }
        
        // Spawn message handler with upload tracking
        tokio::spawn(async move {
            let mut rx = rx;
            tracing::info!("CLIENT {}: Message handler started", client_id);
            while let Some(msg) = rx.recv().await {
                tracing::info!("CLIENT {}: Processing server message: {:?}", client_id, std::mem::discriminant(&msg));
                if let Err(e) = Self::handle_server_message_with_tracking(
                    msg, 
                    &db, 
                    client_id, 
                    &event_dispatcher,
                    &pending_uploads,
                    &upload_complete_notifier,
                    &sync_protection_mode
                ).await {
                    tracing::error!("CLIENT {}: Error handling server message: {}", client_id, e);
                } else {
                    tracing::info!("CLIENT {}: Successfully processed server message", client_id);
                }
            }
            tracing::warn!("CLIENT {}: Message handler terminated", client_id);
        });
        
        // Only perform initial sync if connected
        if self.is_connected.load(Ordering::Relaxed) {
            // Upload-first strategy with protection
            self.event_dispatcher.emit_sync_started();
            
            // Enable protection mode during upload phase
            self.sync_protection_mode.store(true, Ordering::Relaxed);
            tracing::info!("CLIENT {}: Protection mode ENABLED - blocking server overwrites during upload", self.client_id);
            
            // First: Upload any pending documents that were created/modified offline
            tracing::info!("CLIENT {}: Starting upload-first sync - uploading pending changes", self.client_id);
            self.sync_pending_documents().await?;
        } else {
            tracing::info!("CLIENT {}: Starting in offline mode - will sync when connection available", self.client_id);
        }
        
        // Wait for upload confirmations with timeout
        if !self.pending_uploads.lock().await.is_empty() {
            let upload_count = self.pending_uploads.lock().await.len();
            tracing::info!("CLIENT {}: Waiting for {} upload confirmations", self.client_id, upload_count);
            
            tokio::select! {
                _ = self.upload_complete_notifier.notified() => {
                    tracing::info!("CLIENT {}: All uploads confirmed successfully", self.client_id);
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                    let remaining = self.pending_uploads.lock().await.len();
                    if remaining > 0 {
                        tracing::warn!("CLIENT {}: Upload timeout - {} uploads still pending", self.client_id, remaining);
                        
                        // Enhanced fallback: Retry failed uploads before proceeding
                        tracing::info!("CLIENT {}: Retrying failed uploads before sync", self.client_id);
                        if let Err(e) = self.retry_failed_uploads().await {
                            tracing::error!("CLIENT {}: Retry failed: {}", self.client_id, e);
                        }
                    } else {
                        tracing::info!("CLIENT {}: Upload timeout but all uploads completed", self.client_id);
                    }
                }
            }
        } else {
            tracing::info!("CLIENT {}: No pending uploads to wait for", self.client_id);
        }
        
        // Disable protection mode - now safe to receive server sync
        self.sync_protection_mode.store(false, Ordering::Relaxed);
        tracing::info!("CLIENT {}: Protection mode DISABLED - server sync now allowed", self.client_id);
        
        // Second: Download current server state (which now includes our uploaded documents)
        tracing::info!("CLIENT {}: Upload phase complete, requesting server state", self.client_id);
        self.sync_all().await?;
        
        Ok(())
    }
    
    pub async fn create_document(&self, content: serde_json::Value) -> Result<Document, ClientError> {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: self.user_id,
            revision_id: Document::initial_revision(&content),
            content,
            version: 1,
            vector_clock: {
                let mut vc = VectorClock::new();
                vc.increment(&self.node_id);
                vc
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        // Save locally first with explicit "pending" status
        tracing::info!("CLIENT {}: Creating document locally: {}", self.client_id, doc.id);
        self.db.save_document_with_status(&doc, Some(SyncStatus::Pending)).await?;
        
        // Emit event
        self.event_dispatcher.emit_document_created(&doc.id, &doc.content);
        
        // Attempt immediate sync if connected
        if let Err(e) = self.try_immediate_sync(&doc).await {
            tracing::warn!("CLIENT {}: Failed to immediately sync new document {}: {}. Will retry later.", 
                         self.client_id, doc.id, e);
            // Document stays in "pending" status for next sync attempt
        }
        
        Ok(doc)
    }
    
    pub async fn update_document(&self, id: Uuid, new_content: serde_json::Value) -> Result<(), ClientError> {
        let mut doc = self.db.get_document(&id).await?;
        let old_content = doc.content.clone();
        let old_revision = doc.revision_id.clone();
        
        tracing::info!("CLIENT {}: 📝 UPDATING DOCUMENT {}", self.client_id, id);
        tracing::info!("CLIENT {}: OLD: content={:?}, revision={}", 
                     self.client_id, old_content, old_revision);
        tracing::info!("CLIENT {}: NEW: content={:?}", self.client_id, new_content);
        
        // Create patch (for potential future use)
        let _patch = create_patch(&old_content, &new_content)?;
        
        // Update document
        doc.revision_id = doc.next_revision(&new_content);
        doc.content = new_content.clone();
        doc.version += 1;
        doc.vector_clock.increment(&self.node_id);
        doc.updated_at = chrono::Utc::now();
        
        tracing::info!("CLIENT {}: 💾 SAVING LOCALLY: revision={}, marking as pending", 
                     self.client_id, doc.revision_id);
        
        // Save locally with explicit "pending" status for sync
        self.db.save_document_with_status(&doc, Some(SyncStatus::Pending)).await?;
        
        // Verify it was saved correctly
        let saved_doc = self.db.get_document(&id).await?;
        tracing::info!("CLIENT {}: ✅ SAVED: content={:?}, revision={}", 
                     self.client_id, saved_doc.content, saved_doc.revision_id);
        
        // Emit event
        self.event_dispatcher.emit_document_updated(&doc.id, &doc.content);
        
        // Attempt immediate sync if connected
        if let Err(e) = self.try_immediate_sync(&doc).await {
            tracing::warn!("CLIENT {}: ⚠️  OFFLINE EDIT - Failed to immediately sync updated document {}: {}. Changes saved locally for later sync.", 
                         self.client_id, doc.id, e);
            // Document stays in "pending" status for next sync attempt
        }
        
        Ok(())
    }
    
    pub async fn delete_document(&self, id: Uuid) -> Result<(), ClientError> {
        let doc = self.db.get_document(&id).await?;
        
        // Mark as deleted locally first
        self.db.delete_document(&id).await?;
        
        // Emit event
        self.event_dispatcher.emit_document_deleted(&id);
        
        // Try to send delete to server if connected
        let ws_client = self.ws_client.lock().await;
        if let Some(client) = ws_client.as_ref() {
            if let Err(e) = client.send(ClientMessage::DeleteDocument {
                document_id: id,
                revision_id: doc.revision_id.clone(),
            }).await {
                tracing::warn!("CLIENT {}: Failed to send delete to server: {}. Will sync later.", self.client_id, e);
                self.is_connected.store(false, Ordering::Relaxed);
                drop(ws_client);
                self.start_reconnection_loop();
            }
        } else {
            tracing::info!("CLIENT {}: Offline - delete will sync when connection available", self.client_id);
        }
        
        Ok(())
    }
    
    pub async fn get_all_documents(&self) -> Result<Vec<Document>, ClientError> {
        let docs = self.db.get_all_documents().await?;
        tracing::info!("CLIENT: get_all_documents() returning {} documents", docs.len());
        for doc in &docs {
            tracing::info!("CLIENT:   - Document: {} (updated: {})", doc.id, doc.updated_at);
        }
        Ok(docs)
    }
    
    async fn sync_pending_documents(&self) -> Result<(), ClientError> {
        let pending_docs = self.db.get_pending_documents().await?;
        
        if pending_docs.is_empty() {
            tracing::info!("CLIENT {}: No pending documents to sync", self.client_id);
            return Ok(());
        }
        
        tracing::info!("CLIENT {}: 📤 UPLOADING {} PENDING DOCUMENTS", self.client_id, pending_docs.len());
        
        // Show details of each pending document
        for (i, pending_info) in pending_docs.iter().enumerate() {
            if let Ok(doc) = self.db.get_document(&pending_info.id).await {
                tracing::info!("CLIENT {}: PENDING {}/{}: doc_id={}, content={:?}, revision={}", 
                             self.client_id, i+1, pending_docs.len(), 
                             pending_info.id, doc.content, doc.revision_id);
            }
        }
        
        for pending_info in pending_docs {
            match self.db.get_document(&pending_info.id).await {
                Ok(doc) => {
                    let upload_type = if pending_info.is_deleted {
                        // Handle pending delete
                        tracing::info!("CLIENT {}: Uploading pending delete for doc {}", self.client_id, pending_info.id);
                        
                        // Track this upload
                        self.pending_uploads.lock().await.insert(pending_info.id, PendingUpload {
                            document_id: pending_info.id,
                            operation_type: UploadType::Delete,
                            sent_at: Instant::now(),
                        });
                        
                        let ws_client = self.ws_client.lock().await;
                        if let Some(client) = ws_client.as_ref() {
                            client.send(ClientMessage::DeleteDocument {
                                document_id: pending_info.id,
                                revision_id: doc.revision_id.clone(),
                            }).await?;
                        } else {
                            return Err(ClientError::WebSocket("Not connected".to_string()));
                        }
                        
                        UploadType::Delete
                    } else if pending_info.last_synced_revision.is_none() {
                        // No previous sync revision = new document created offline
                        tracing::info!("CLIENT {}: Uploading new document {} created offline", self.client_id, pending_info.id);
                        
                        // Track this upload
                        self.pending_uploads.lock().await.insert(pending_info.id, PendingUpload {
                            document_id: pending_info.id,
                            operation_type: UploadType::Create,
                            sent_at: Instant::now(),
                        });
                        
                        let ws_client = self.ws_client.lock().await;
                        if let Some(client) = ws_client.as_ref() {
                            client.send(ClientMessage::CreateDocument { 
                                document: doc.clone() 
                            }).await?;
                        } else {
                            return Err(ClientError::WebSocket("Not connected".to_string()));
                        }
                        
                        UploadType::Create
                    } else {
                        // Has previous sync revision = document was updated offline
                        tracing::info!("CLIENT {}: Uploading document {} updated offline (previous rev: {})", 
                                     self.client_id, pending_info.id, 
                                     pending_info.last_synced_revision.as_ref().unwrap());
                        
                        // Track this upload
                        self.pending_uploads.lock().await.insert(pending_info.id, PendingUpload {
                            document_id: pending_info.id,
                            operation_type: UploadType::Update,
                            sent_at: Instant::now(),
                        });
                        
                        // Create a patch from the last synced revision to current state
                        // For now, we'll send the full document as CreateDocument 
                        // TODO: Implement proper patch generation for updates
                        tracing::warn!("CLIENT {}: Sending updated document as CreateDocument (patch generation not yet implemented)", self.client_id);
                        let ws_client = self.ws_client.lock().await;
                        if let Some(client) = ws_client.as_ref() {
                            client.send(ClientMessage::CreateDocument { 
                                document: doc.clone() 
                            }).await?;
                        } else {
                            return Err(ClientError::WebSocket("Not connected".to_string()));
                        }
                        
                        UploadType::Update
                    };
                    
                    tracing::debug!("CLIENT {}: Tracked upload for document {} ({:?})", 
                                  self.client_id, pending_info.id, upload_type);
                }
                Err(e) => {
                    tracing::error!("CLIENT {}: Failed to get pending document {}: {}", self.client_id, pending_info.id, e);
                }
            }
        }
        
        tracing::info!("CLIENT {}: Upload tracking: {} operations pending confirmation", 
                      self.client_id, self.pending_uploads.lock().await.len());
        Ok(())
    }
    
    // Enhanced message handler with upload tracking and protection
    async fn handle_server_message_with_tracking(
        msg: ServerMessage, 
        db: &Arc<ClientDatabase>, 
        client_id: Uuid, 
        event_dispatcher: &Arc<EventDispatcher>,
        pending_uploads: &Arc<Mutex<HashMap<Uuid, PendingUpload>>>,
        upload_complete_notifier: &Arc<Notify>,
        sync_protection_mode: &Arc<AtomicBool>
    ) -> Result<(), ClientError> {
        match &msg {
            // Handle upload confirmations first
            ServerMessage::DocumentCreatedResponse { document_id, success, .. } |
            ServerMessage::DocumentUpdatedResponse { document_id, success, .. } |
            ServerMessage::DocumentDeletedResponse { document_id, success, .. } => {
                if *success {
                    // Remove from pending uploads
                    let mut uploads = pending_uploads.lock().await;
                    if let Some(upload) = uploads.remove(document_id) {
                        let elapsed = upload.sent_at.elapsed();
                        tracing::info!("CLIENT {}: Upload confirmed for {} ({:?}) in {:?}", 
                                     client_id, document_id, upload.operation_type, elapsed);
                        
                        // If this was the last pending upload, notify
                        if uploads.is_empty() {
                            tracing::info!("CLIENT {}: All uploads confirmed - notifying completion", client_id);
                            upload_complete_notifier.notify_one();
                        }
                    }
                } else {
                    tracing::error!("CLIENT {}: Upload failed for document {}", client_id, document_id);
                }
                
                // Continue with normal processing
                return Self::handle_server_message(msg, db, client_id, event_dispatcher).await;
            }
            
            // Apply protection for sync messages during upload phase
            ServerMessage::SyncDocument { document } => {
                // Check if we're in protection mode
                if sync_protection_mode.load(Ordering::Relaxed) {
                    tracing::warn!("CLIENT {}: PROTECTED - Upload phase active, deferring sync for document {}", 
                                  client_id, document.id);
                    return Ok(()); // Defer this message
                }
                
                // Check if document has pending changes
                if let Ok(_local_doc) = db.get_document(&document.id).await {
                    // Check if this document has an active upload in progress
                    // This is our primary protection mechanism
                    if Self::has_pending_upload(pending_uploads, &document.id).await {
                        tracing::warn!("CLIENT {}: PROTECTED - Document {} has active upload, refusing server overwrite", 
                                      client_id, document.id);
                        
                        event_dispatcher.emit_sync_error(&format!(
                            "Document {} protected from overwrite (active upload in progress)", document.id
                        ));
                        return Ok(());
                    }
                }
                
                // Safe to proceed with sync
                return Self::handle_server_message(msg, db, client_id, event_dispatcher).await;
            }
            
            _ => {
                // For all other messages, use normal handling
                return Self::handle_server_message(msg, db, client_id, event_dispatcher).await;
            }
        }
    }

    // Check if a document has an active upload in progress
    async fn has_pending_upload(pending_uploads: &Arc<Mutex<HashMap<Uuid, PendingUpload>>>, document_id: &Uuid) -> bool {
        let uploads = pending_uploads.lock().await;
        uploads.contains_key(document_id)
    }

    async fn handle_server_message(msg: ServerMessage, db: &Arc<ClientDatabase>, client_id: Uuid, event_dispatcher: &Arc<EventDispatcher>) -> Result<(), ClientError> {
        match msg {
            ServerMessage::DocumentUpdated { patch } => {
                // Apply patch from server
                tracing::info!("CLIENT {}: Received DocumentUpdated for doc {}", client_id, patch.document_id);
                let mut doc = db.get_document(&patch.document_id).await?;
                
                tracing::info!("CLIENT {}: Document content before patch: {:?}", client_id, doc.content);
                tracing::info!("CLIENT {}: Patch to apply: {:?}", client_id, patch.patch);
                
                // Check for conflicts
                if doc.vector_clock.is_concurrent(&patch.vector_clock) {
                    tracing::warn!("CLIENT {}: Conflict detected for document {}", client_id, patch.document_id);
                    // Handle conflict - for now, server wins
                }
                
                // Apply patch
                apply_patch(&mut doc.content, &patch.patch)?;
                doc.revision_id = patch.revision_id;
                doc.vector_clock.merge(&patch.vector_clock);
                doc.updated_at = chrono::Utc::now();
                
                tracing::info!("CLIENT {}: Document content after patch: {:?}", client_id, doc.content);
                
                db.save_document(&doc).await?;
                db.mark_synced(&doc.id, &doc.revision_id).await?;
                
                // Emit event for updated document
                event_dispatcher.emit_document_updated(&doc.id, &doc.content);
            }
            ServerMessage::DocumentCreated { document } => {
                // New document from server - check if we already have it to avoid duplicates
                tracing::info!("CLIENT: Received DocumentCreated from server: {}", document.id);
                
                // Check if we already have this document (e.g., if we were the creator)
                match db.get_document(&document.id).await {
                    Ok(existing_doc) => {
                        // We already have this document - just ensure it's marked as synced
                        if existing_doc.revision_id == document.revision_id {
                            tracing::info!("CLIENT: Document {} already exists locally with same revision, marking as synced", document.id);
                            db.mark_synced(&document.id, &document.revision_id).await?;
                        } else {
                            // Different revision - update it
                            tracing::info!("CLIENT: Document {} exists locally but has different revision, updating", document.id);
                            db.save_document_with_status(&document, Some(SyncStatus::Synced)).await?;
                            
                            // Emit event for updated document
                            event_dispatcher.emit_document_updated(&document.id, &document.content);
                        }
                    }
                    Err(_) => {
                        // Document doesn't exist locally - save it
                        tracing::info!("CLIENT: Document {} is new, saving to local database", document.id);
                        db.save_document_with_status(&document, Some(SyncStatus::Synced)).await?;
                        
                        // Emit event for new document from server
                        event_dispatcher.emit_document_created(&document.id, &document.content);
                    }
                }
            }
            ServerMessage::DocumentDeleted { document_id, revision_id } => {
                // Document deleted from server - we need to delete it locally
                tracing::info!("CLIENT {}: Received DocumentDeleted for doc {} with revision {}", client_id, document_id, revision_id);
                
                // Delete the document locally (soft delete)
                db.delete_document(&document_id).await?;
                
                // Mark it as synced so we don't try to sync the delete again
                db.mark_synced(&document_id, &revision_id).await?;
                
                // Emit event for deleted document
                event_dispatcher.emit_document_deleted(&document_id);
            }
            ServerMessage::ConflictDetected { document_id, .. } => {
                tracing::warn!("Conflict detected for document {}", document_id);
                
                // Emit conflict event
                event_dispatcher.emit_conflict_detected(&document_id);
            }
            ServerMessage::SyncDocument { document } => {
                // Document sync - check if it's newer than what we have
                tracing::info!("CLIENT {}: 📥 RECEIVED SyncDocument: {} (rev: {})", client_id, document.id, document.revision_id);
                
                match db.get_document(&document.id).await {
                    Ok(local_doc) => {
                        tracing::info!("CLIENT {}: LOCAL  DOCUMENT: content={:?}, revision={}", 
                                     client_id, local_doc.content, local_doc.revision_id);
                        tracing::info!("CLIENT {}: SERVER DOCUMENT: content={:?}, revision={}", 
                                     client_id, document.content, document.revision_id);
                        
                        // Compare revisions
                        let local_rev_parts: Vec<&str> = local_doc.revision_id.split('-').collect();
                        let sync_rev_parts: Vec<&str> = document.revision_id.split('-').collect();
                        
                        if local_rev_parts.len() == 2 && sync_rev_parts.len() == 2 {
                            let local_gen: u32 = local_rev_parts[0].parse().unwrap_or(0);
                            let sync_gen: u32 = sync_rev_parts[0].parse().unwrap_or(0);
                            
                            if sync_gen >= local_gen {
                                // Check if this might be overwriting local changes by comparing content
                                if local_doc.content != document.content {
                                    tracing::warn!("CLIENT {}: ⚠️  SERVER OVERWRITING LOCAL CHANGES!", client_id);
                                    tracing::warn!("CLIENT {}: LOCAL: {:?} → SERVER: {:?}", 
                                                 client_id, local_doc.content, document.content);
                                }
                                
                                tracing::info!("CLIENT {}: 🔄 Updating to newer version (gen {} -> {})", 
                                             client_id, local_gen, sync_gen);
                                db.save_document_with_status(&document, Some(SyncStatus::Synced)).await?;
                                
                                // Emit event for updated document
                                event_dispatcher.emit_document_updated(&document.id, &document.content);
                            } else {
                                tracing::info!("CLIENT {}: Skipping older sync (local gen {} > sync gen {})", 
                                             client_id, local_gen, sync_gen);
                            }
                        } else {
                            // Can't compare, accept the sync
                            db.save_document_with_status(&document, Some(SyncStatus::Synced)).await?;
                            
                            // Emit event for updated document
                            event_dispatcher.emit_document_updated(&document.id, &document.content);
                        }
                    }
                    Err(_) => {
                        // Document doesn't exist locally - save it
                        tracing::info!("CLIENT {}: Document {} is new, saving", client_id, document.id);
                        db.save_document_with_status(&document, Some(SyncStatus::Synced)).await?;
                        
                        // Emit event for new document
                        event_dispatcher.emit_document_created(&document.id, &document.content);
                    }
                }
            }
            ServerMessage::SyncComplete { synced_count } => {
                tracing::debug!("Sync complete, received {} documents", synced_count);
                
                // Emit sync completed event
                event_dispatcher.emit_sync_completed(synced_count as u64);
            }
            
            // Handle document operation confirmations
            ServerMessage::DocumentCreatedResponse { document_id, revision_id, success, error } => {
                if success {
                    tracing::info!("CLIENT {}: Document creation confirmed by server: {}", client_id, document_id);
                    db.mark_synced(&document_id, &revision_id).await?;
                } else {
                    tracing::error!("CLIENT {}: Document creation failed on server: {} - {}", 
                                  client_id, document_id, error.as_deref().unwrap_or("unknown error"));
                    // Could emit an error event here
                    event_dispatcher.emit_sync_error(&format!("Create failed: {}", error.as_deref().unwrap_or("unknown")));
                }
            }
            
            ServerMessage::DocumentUpdatedResponse { document_id, revision_id, success, error } => {
                if success {
                    tracing::info!("CLIENT {}: Document update confirmed by server: {}", client_id, document_id);
                    db.mark_synced(&document_id, &revision_id).await?;
                } else {
                    tracing::error!("CLIENT {}: Document update failed on server: {} - {}", 
                                  client_id, document_id, error.as_deref().unwrap_or("unknown error"));
                    event_dispatcher.emit_sync_error(&format!("Update failed: {}", error.as_deref().unwrap_or("unknown")));
                }
            }
            
            ServerMessage::DocumentDeletedResponse { document_id, revision_id, success, error } => {
                if success {
                    tracing::info!("CLIENT {}: Document deletion confirmed by server: {}", client_id, document_id);
                    db.mark_synced(&document_id, &revision_id).await?;
                } else {
                    tracing::error!("CLIENT {}: Document deletion failed on server: {} - {}", 
                                  client_id, document_id, error.as_deref().unwrap_or("unknown error"));
                    event_dispatcher.emit_sync_error(&format!("Delete failed: {}", error.as_deref().unwrap_or("unknown")));
                }
            }
            
            _ => {}
        }
        
        Ok(())
    }
    
    // Retry failed uploads by re-checking pending documents
    async fn retry_failed_uploads(&self) -> Result<(), ClientError> {
        tracing::info!("CLIENT {}: Starting upload retry for failed operations", self.client_id);
        
        // Get current pending uploads (these are the ones that timed out)
        let timed_out_uploads: Vec<Uuid> = {
            let uploads = self.pending_uploads.lock().await;
            uploads.keys().cloned().collect()
        };
        
        if timed_out_uploads.is_empty() {
            tracing::info!("CLIENT {}: No timed out uploads to retry", self.client_id);
            return Ok(());
        }
        
        tracing::info!("CLIENT {}: Retrying {} timed out uploads", self.client_id, timed_out_uploads.len());
        
        // Clear the pending uploads (we'll re-add them during retry)
        self.pending_uploads.lock().await.clear();
        
        // Re-run sync_pending_documents to retry uploads
        // This will re-query the database for documents with pending status
        // and re-upload them with fresh tracking
        self.sync_pending_documents().await?;
        
        // Quick wait for the retry confirmations (shorter timeout)
        if !self.pending_uploads.lock().await.is_empty() {
            let retry_count = self.pending_uploads.lock().await.len();
            tracing::info!("CLIENT {}: Waiting for {} retry confirmations (short timeout)", self.client_id, retry_count);
            
            tokio::select! {
                _ = self.upload_complete_notifier.notified() => {
                    tracing::info!("CLIENT {}: All retry uploads confirmed", self.client_id);
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {
                    let remaining = self.pending_uploads.lock().await.len();
                    tracing::warn!("CLIENT {}: Retry timeout - {} uploads still failing", self.client_id, remaining);
                    // Don't retry again - proceed with partial failure
                }
            }
        }
        
        Ok(())
    }
    
    pub async fn sync_all(&self) -> Result<(), ClientError> {
        // Request full sync on startup to get all documents
        tracing::debug!("Requesting full sync from server");
        
        let ws_client = self.ws_client.lock().await;
        if let Some(client) = ws_client.as_ref() {
            client.send(ClientMessage::RequestFullSync).await?;
        } else {
            tracing::warn!("CLIENT {}: Cannot sync - not connected", self.client_id);
            return Err(ClientError::WebSocket("Not connected".to_string()));
        }
        
        Ok(())
    }
    
    /// Check if the WebSocket connection is active
    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }
    
    /// Attempt to sync a single document immediately if connected
    async fn try_immediate_sync(&self, document: &Document) -> Result<(), ClientError> {
        if !self.is_connected() {
            tracing::debug!("CLIENT {}: Offline, document {} will sync later", self.client_id, document.id);
            return Ok(());
        }
        
        tracing::info!("CLIENT {}: Attempting immediate sync for document {}", self.client_id, document.id);
        
        // Add to pending uploads for tracking
        {
            let mut uploads = self.pending_uploads.lock().await;
            uploads.insert(document.id, PendingUpload {
                document_id: document.id,
                operation_type: UploadType::Create, // Server treats all as creates
                sent_at: Instant::now(),
            });
        }
        
        // Send to server (using CreateDocument for both creates and updates)
        let ws_client = self.ws_client.lock().await;
        match ws_client.as_ref() {
            Some(client) => {
                match client.send(ClientMessage::CreateDocument { 
                    document: document.clone() 
                }).await {
                    Ok(_) => {
                        tracing::debug!("CLIENT {}: Immediate sync request sent for document {}", self.client_id, document.id);
                        Ok(())
                    }
                    Err(e) => {
                        // Connection failed - mark as disconnected and remove from pending uploads
                        self.is_connected.store(false, Ordering::Relaxed);
                        {
                            let mut uploads = self.pending_uploads.lock().await;
                            uploads.remove(&document.id);
                        }
                        tracing::warn!("CLIENT {}: WebSocket send failed, marked as disconnected", self.client_id);
                        // Start reconnection loop if not already running
                        drop(ws_client); // Release lock before starting reconnection
                        self.start_reconnection_loop();
                        Err(e)
                    }
                }
            }
            None => {
                tracing::warn!("CLIENT {}: No WebSocket connection available for immediate sync", self.client_id);
                {
                    let mut uploads = self.pending_uploads.lock().await;
                    uploads.remove(&document.id);
                }
                Err(ClientError::WebSocket("Not connected".to_string()))
            }
        }
    }
    
    /// Start the reconnection loop if not already running
    fn start_reconnection_loop(&self) {
        let is_connected = self.is_connected.clone();
        let ws_client = self.ws_client.clone();
        let server_url = self.server_url.clone();
        let auth_token = self.auth_token.clone();
        let user_id = self.user_id;
        let client_id = self.client_id;
        let event_dispatcher = self.event_dispatcher.clone();
        let db = self.db.clone();
        let pending_uploads = self.pending_uploads.clone();
        let upload_complete_notifier = self.upload_complete_notifier.clone();
        let sync_protection_mode = self.sync_protection_mode.clone();
        
        tokio::spawn(async move {
            let mut retry_delay = std::time::Duration::from_secs(1);
            const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(30);
            
            loop {
                if is_connected.load(Ordering::Relaxed) {
                    // Already connected, exit loop
                    break;
                }
                
                tracing::info!("CLIENT {}: Attempting to reconnect...", client_id);
                
                // Try to connect
                match WebSocketClient::connect(
                    &server_url,
                    user_id,
                    client_id,
                    &auth_token,
                    Some(event_dispatcher.clone())
                ).await {
                    Ok((new_client, receiver)) => {
                        tracing::info!("CLIENT {}: Reconnection successful!", client_id);
                        
                        // Update the client
                        *ws_client.lock().await = Some(new_client);
                        is_connected.store(true, Ordering::Relaxed);
                        
                        // Start message receiver forwarding
                        let (tx, mut rx) = mpsc::channel(100);
                        tokio::spawn(async move {
                            if let Err(e) = receiver.forward_to(tx).await {
                                tracing::error!("WebSocket receiver error: {}", e);
                            }
                        });
                        
                        // Process messages in background
                        let db_clone = db.clone();
                        let event_dispatcher_clone = event_dispatcher.clone();
                        let pending_uploads_clone = pending_uploads.clone();
                        let upload_complete_notifier_clone = upload_complete_notifier.clone();
                        let sync_protection_mode_clone = sync_protection_mode.clone();
                        tokio::spawn(async move {
                            while let Some(msg) = rx.recv().await {
                                if let Err(e) = Self::handle_server_message_with_tracking(
                                    msg,
                                    &db_clone,
                                    client_id,
                                    &event_dispatcher_clone,
                                    &pending_uploads_clone,
                                    &upload_complete_notifier_clone,
                                    &sync_protection_mode_clone
                                ).await {
                                    tracing::error!("CLIENT {}: Error handling server message: {}", client_id, e);
                                }
                            }
                        });
                        
                        // Sync pending documents
                        tracing::info!("CLIENT {}: Syncing pending documents after reconnection", client_id);
                        if let Ok(pending_docs) = db.get_pending_documents().await {
                            if !pending_docs.is_empty() {
                                tracing::info!("CLIENT {}: Found {} pending documents to sync", client_id, pending_docs.len());
                                
                                // Trigger upload of pending documents
                                // We can't call sync_pending_documents directly, so we'll send the documents manually
                                for pending_info in pending_docs {
                                    if let Ok(doc) = db.get_document(&pending_info.id).await {
                                        if let Some(client) = ws_client.lock().await.as_ref() {
                                            let _ = client.send(ClientMessage::CreateDocument { 
                                                document: doc 
                                            }).await;
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Reset retry delay
                        retry_delay = std::time::Duration::from_secs(1);
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("CLIENT {}: Reconnection failed: {}. Retrying in {:?}", client_id, e, retry_delay);
                        
                        // Wait before next retry
                        tokio::time::sleep(retry_delay).await;
                        
                        // Exponential backoff
                        retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                    }
                }
            }
        });
    }
}

