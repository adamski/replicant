use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::collections::HashMap;
use std::time::Instant;
use std::io::Write;
use tokio::sync::{mpsc, Mutex, Notify};
use uuid::Uuid;
use sqlx::Row;
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

fn debug_log(msg: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/task_list_debug.log")
        .unwrap();
    let timestamp = chrono::Utc::now().format("%H:%M:%S%.3f");
    writeln!(file, "[{}] SYNC: {}", timestamp, msg).unwrap();
}

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
        
        let mut engine = Self {
            db: db.clone(),
            ws_client: Arc::new(Mutex::new(ws_client)),
            user_id,
            client_id,
            node_id,
            message_rx: Some(rx),
            event_dispatcher: event_dispatcher.clone(),
            pending_uploads: Arc::new(Mutex::new(HashMap::new())),
            upload_complete_notifier: Arc::new(Notify::new()),
            sync_protection_mode: Arc::new(AtomicBool::new(false)),
            is_connected: Arc::new(AtomicBool::new(is_connected)),
            server_url: server_url.to_string(),
            auth_token: auth_token.to_string(),
        };
        
        // Automatically start background tasks
        engine.spawn_background_tasks().await?;
        
        Ok(engine)
    }
    
    pub fn event_dispatcher(&self) -> Arc<EventDispatcher> {
        self.event_dispatcher.clone()
    }
    
    async fn spawn_background_tasks(&mut self) -> Result<(), ClientError> {
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
        } else {
            tracing::info!("CLIENT {}: Starting in offline mode - will sync when connection available", self.client_id);
        }
        
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
        
        // Create patch for sync
        let patch = create_patch(&old_content, &new_content)?;
        
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
        
        // Store patch in sync_queue for offline sync
        use sync_core::protocol::ChangeEventType;
        tracing::info!("CLIENT {}: 📋 About to call queue_sync_operation for doc {}", self.client_id, doc.id);
        let queue_result = self.db.queue_sync_operation(&doc.id, ChangeEventType::Update, Some(&patch)).await;
        match &queue_result {
            Ok(_) => tracing::info!("CLIENT {}: 📋 Successfully stored update patch in sync_queue", self.client_id),
            Err(e) => tracing::error!("CLIENT {}: 📋 FAILED to store patch in sync_queue: {}", self.client_id, e),
        }
        queue_result?;
        
        // Verify it was saved correctly and check its sync status
        let saved_doc = self.db.get_document(&id).await?;
        tracing::info!("CLIENT {}: ✅ SAVED: content={:?}, revision={}", 
                     self.client_id, saved_doc.content, saved_doc.revision_id);
        
        // Check sync status after save
        let sync_status_result = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&self.db.pool)
            .await;
        
        match sync_status_result {
            Ok(row) => {
                let sync_status: String = row.try_get("sync_status").unwrap_or_else(|_| "unknown".to_string());
                tracing::info!("CLIENT {}: 📊 Document {} sync_status after save: {}", 
                             self.client_id, id, sync_status);
            }
            Err(e) => {
                tracing::error!("CLIENT {}: Failed to check sync_status: {}", self.client_id, e);
            }
        }
        
        // Emit event
        self.event_dispatcher.emit_document_updated(&doc.id, &doc.content);
        
        // Attempt immediate sync if connected
        tracing::info!("CLIENT {}: 🚀 Attempting immediate sync for updated document {}", self.client_id, doc.id);
        if let Err(e) = self.try_immediate_sync(&doc).await {
            tracing::warn!("CLIENT {}: ⚠️  OFFLINE EDIT - Failed to immediately sync updated document {}: {}. Changes saved locally for later sync.", 
                         self.client_id, doc.id, e);
            // Document stays in "pending" status for next sync attempt
            
            // Double-check sync status after failed immediate sync
            let sync_status_result = sqlx::query("SELECT sync_status FROM documents WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(&self.db.pool)
                .await;
            
            match sync_status_result {
                Ok(row) => {
                    let sync_status: String = row.try_get("sync_status").unwrap_or_else(|_| "unknown".to_string());
                    tracing::warn!("CLIENT {}: 📊 Document {} sync_status after FAILED immediate sync: {}", 
                                 self.client_id, id, sync_status);
                }
                Err(e) => {
                    tracing::error!("CLIENT {}: Failed to check sync_status after failed sync: {}", self.client_id, e);
                }
            }
        } else {
            tracing::info!("CLIENT {}: ✅ Immediate sync successful for document {}", self.client_id, doc.id);
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
        
        // Also check sync_queue for debugging
        let sync_queue_result = sqlx::query("SELECT COUNT(*) as count FROM sync_queue")
            .fetch_one(&self.db.pool)
            .await;
        
        match sync_queue_result {
            Ok(row) => {
                let count: i64 = row.try_get("count").unwrap_or(0);
                tracing::info!("CLIENT {}: 📋 sync_queue contains {} entries", self.client_id, count);
                
                if count > 0 {
                    // Show what's in the sync_queue
                    let queue_entries = sqlx::query("SELECT document_id, operation_type, created_at FROM sync_queue")
                        .fetch_all(&self.db.pool)
                        .await;
                    
                    match queue_entries {
                        Ok(rows) => {
                            for row in rows {
                                let doc_id: String = row.try_get("document_id").unwrap_or_else(|_| "unknown".to_string());
                                let op_type: String = row.try_get("operation_type").unwrap_or_else(|_| "unknown".to_string());
                                let created_at: String = row.try_get("created_at").unwrap_or_else(|_| "unknown".to_string());
                                tracing::info!("CLIENT {}: 📋 sync_queue entry: doc_id={}, op_type={}, created_at={}", 
                                             self.client_id, doc_id, op_type, created_at);
                            }
                        }
                        Err(e) => {
                            tracing::error!("CLIENT {}: Failed to query sync_queue entries: {}", self.client_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("CLIENT {}: Failed to count sync_queue: {}", self.client_id, e);
            }
        }
        
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
                        let last_synced_rev = pending_info.last_synced_revision.as_ref().unwrap();
                        tracing::info!("CLIENT {}: 🔄 OFFLINE UPDATE DETECTED for doc {}", self.client_id, pending_info.id);
                        tracing::info!("CLIENT {}: Current revision: {} | Last synced: {}", 
                                     self.client_id, doc.revision_id, last_synced_rev);
                        tracing::info!("CLIENT {}: Current content: {:?}", self.client_id, doc.content);
                        
                        // Track this upload
                        self.pending_uploads.lock().await.insert(pending_info.id, PendingUpload {
                            document_id: pending_info.id,
                            operation_type: UploadType::Update,
                            sent_at: Instant::now(),
                        });
                        
                        // Check if we have a patch stored in sync_queue
                        match self.db.get_queued_patch(&pending_info.id).await? {
                            Some(stored_patch) => {
                                tracing::info!("CLIENT {}: 📋 Found stored patch in sync_queue for doc {}", 
                                             self.client_id, pending_info.id);
                                
                                // Use the stored patch for UpdateDocument
                                use sync_core::models::DocumentPatch;
                                use sync_core::patches::calculate_checksum;
                                
                                let document_patch = DocumentPatch {
                                    document_id: pending_info.id,
                                    revision_id: doc.revision_id.clone(),
                                    patch: stored_patch,
                                    vector_clock: doc.vector_clock.clone(),
                                    checksum: calculate_checksum(&doc.content),
                                };
                                
                                let ws_client = self.ws_client.lock().await;
                                if let Some(client) = ws_client.as_ref() {
                                    tracing::info!("CLIENT {}: ✅ Sending UpdateDocument with stored patch", self.client_id);
                                    client.send(ClientMessage::UpdateDocument { 
                                        patch: document_patch 
                                    }).await?;
                                } else {
                                    return Err(ClientError::WebSocket("Not connected".to_string()));
                                }
                            }
                            None => {
                                // No stored patch - try to create one or fall back
                                tracing::warn!("CLIENT {}: No stored patch found for doc {}, creating UpdateDocument", 
                                             self.client_id, pending_info.id);
                                
                                match self.create_update_message(&doc, last_synced_rev).await {
                                    Ok(update_message) => {
                                        tracing::info!("CLIENT {}: Created UpdateDocument message with generated patch", self.client_id);
                                        
                                        let ws_client = self.ws_client.lock().await;
                                        if let Some(client) = ws_client.as_ref() {
                                            client.send(update_message).await?;
                                        } else {
                                            return Err(ClientError::WebSocket("Not connected".to_string()));
                                        }
                                    }
                                    Err(e) => {
                                        // Fall back to CreateDocument if we can't create proper patch
                                        tracing::warn!("CLIENT {}: Failed to create UpdateDocument patch: {}. Falling back to CreateDocument", 
                                                     self.client_id, e);
                                        
                                        let ws_client = self.ws_client.lock().await;
                                        if let Some(client) = ws_client.as_ref() {
                                            client.send(ClientMessage::CreateDocument { 
                                                document: doc.clone() 
                                            }).await?;
                                        } else {
                                            return Err(ClientError::WebSocket("Not connected".to_string()));
                                        }
                                    }
                                }
                            }
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
                    // Clean up sync_queue
                    db.remove_from_sync_queue(&document_id).await?;
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
                    // Clean up sync_queue
                    db.remove_from_sync_queue(&document_id).await?;
                    tracing::info!("CLIENT {}: Removed doc {} from sync_queue", client_id, document_id);
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
                    // Clean up sync_queue
                    db.remove_from_sync_queue(&document_id).await?;
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
        let connected = self.is_connected();
        tracing::info!("CLIENT {}: 🔍 Connection status check: connected={}", self.client_id, connected);
        
        if !connected {
            tracing::warn!("CLIENT {}: 📴 OFFLINE - Document {} cannot sync immediately, returning error to mark as pending", 
                         self.client_id, document.id);
            return Err(ClientError::WebSocket("Client is offline - document should remain pending".to_string()));
        }
        
        tracing::info!("CLIENT {}: 🚀 IMMEDIATE SYNC attempt for document {}", self.client_id, document.id);
        tracing::info!("CLIENT {}: Document revision: {}, content: {:?}", 
                     self.client_id, document.revision_id, document.content);
        
        // Determine if this is a create or update based on revision generation
        let generation = document.generation();
        let (operation_type, message) = if generation == 1 {
            // New document (generation 1)
            tracing::info!("CLIENT {}: Sending as CREATE (generation 1)", self.client_id);
            (UploadType::Create, ClientMessage::CreateDocument { 
                document: document.clone() 
            })
        } else {
            // Updated document (generation > 1) - we need to send as update
            // For immediate sync of updates, we'll fall back to CreateDocument for now
            // since we don't have the previous content to create a proper patch
            tracing::warn!("CLIENT {}: Document update (gen {}) - using CreateDocument as fallback", 
                         self.client_id, generation);
            tracing::warn!("CLIENT {}: This may cause conflicts - proper UpdateDocument with patch needed", self.client_id);
            (UploadType::Update, ClientMessage::CreateDocument { 
                document: document.clone() 
            })
        };
        
        // Add to pending uploads for tracking
        {
            let mut uploads = self.pending_uploads.lock().await;
            uploads.insert(document.id, PendingUpload {
                document_id: document.id,
                operation_type,
                sent_at: Instant::now(),
            });
        }
        
        let ws_client = self.ws_client.lock().await;
        match ws_client.as_ref() {
            Some(client) => {
                match client.send(message).await {
                    Ok(_) => {
                        tracing::info!("CLIENT {}: ✅ Immediate sync request sent for document {}", self.client_id, document.id);
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
    
    /// Create an UpdateDocument message with proper patch from last synced revision
    async fn create_update_message(&self, current_doc: &Document, _last_synced_revision: &str) -> Result<ClientMessage, ClientError> {
        use sync_core::models::DocumentPatch;
        use sync_core::patches::{create_patch, calculate_checksum};
        
        // For now, since we don't store the last synced content, we'll construct a simple patch
        // that represents the entire content change. This is not ideal but better than CreateDocument.
        // TODO: Store last synced content to create proper granular patches
        
        tracing::warn!("CLIENT {}: Creating simplified UpdateDocument patch (full content replacement)", self.client_id);
        tracing::warn!("CLIENT {}: For proper patches, need to store last synced content in database", self.client_id);
        
        // Create a patch that replaces the entire content
        // We'll use an empty object as the "from" state since we don't have the last synced content
        let empty_content = serde_json::json!({});
        let patch = create_patch(&empty_content, &current_doc.content)?;
        
        let document_patch = DocumentPatch {
            document_id: current_doc.id,
            revision_id: current_doc.revision_id.clone(),
            patch,
            vector_clock: current_doc.vector_clock.clone(),
            checksum: calculate_checksum(&current_doc.content),
        };
        
        Ok(ClientMessage::UpdateDocument {
            patch: document_patch,
        })
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
        
        tracing::info!("🔄 CLIENT {}: Starting continuous reconnection monitor (5-second intervals)", client_id);
        
        tokio::spawn(async move {
            const RECONNECTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
            let mut connection_attempts = 0;
            
            loop {
                let currently_connected = is_connected.load(Ordering::Relaxed);
                
                if !currently_connected {
                    connection_attempts += 1;
                    tracing::info!("🔌 CLIENT {}: Connection attempt #{} to {}", client_id, connection_attempts, server_url);
                    
                    // Try to connect
                    match WebSocketClient::connect(
                        &server_url,
                        user_id,
                        client_id,
                        &auth_token,
                        Some(event_dispatcher.clone())
                    ).await {
                        Ok((new_client, receiver)) => {
                            tracing::info!("✅ CLIENT {}: Reconnection successful after {} attempts!", client_id, connection_attempts);
                            connection_attempts = 0;
                            
                            // Update the client
                            *ws_client.lock().await = Some(new_client);
                            is_connected.store(true, Ordering::Relaxed);
                            
                            // Emit connection event
                            event_dispatcher.emit_connection_succeeded(&server_url);
                            
                            // Start message receiver forwarding with connection monitoring
                            let (tx, mut rx) = mpsc::channel(100);
                            let receiver_is_connected = is_connected.clone();
                            let receiver_client_id = client_id;
                            tokio::spawn(async move {
                                match receiver.forward_to(tx).await {
                                    Ok(_) => {
                                        tracing::info!("🔌 CLIENT {}: WebSocket receiver completed normally", receiver_client_id);
                                    }
                                    Err(e) => {
                                        tracing::warn!("❌ CLIENT {}: WebSocket receiver error: {} - marking as disconnected", receiver_client_id, e);
                                        receiver_is_connected.store(false, Ordering::Relaxed);
                                    }
                                }
                            });
                            
                            // Process messages in background with connection monitoring
                            let db_clone = db.clone();
                            let event_dispatcher_clone = event_dispatcher.clone();
                            let pending_uploads_clone = pending_uploads.clone();
                            let upload_complete_notifier_clone = upload_complete_notifier.clone();
                            let sync_protection_mode_clone = sync_protection_mode.clone();
                            let handler_is_connected = is_connected.clone();
                            let handler_client_id = client_id;
                            tokio::spawn(async move {
                                while let Some(msg) = rx.recv().await {
                                    if let Err(e) = Self::handle_server_message_with_tracking(
                                        msg,
                                        &db_clone,
                                        handler_client_id,
                                        &event_dispatcher_clone,
                                        &pending_uploads_clone,
                                        &upload_complete_notifier_clone,
                                        &sync_protection_mode_clone
                                    ).await {
                                        tracing::error!("CLIENT {}: Error handling server message: {}", handler_client_id, e);
                                    }
                                }
                                tracing::warn!("📪 CLIENT {}: Message handler terminated - marking as disconnected", handler_client_id);
                                handler_is_connected.store(false, Ordering::Relaxed);
                            });
                            
                            // Sync pending documents after reconnection
                            tracing::info!("📤 CLIENT {}: Checking for pending documents after reconnection", client_id);
                            if let Ok(pending_docs) = db.get_pending_documents().await {
                                if !pending_docs.is_empty() {
                                    tracing::info!("📋 CLIENT {}: Found {} pending documents to sync after reconnection", client_id, pending_docs.len());
                                    
                                    // Trigger upload of pending documents
                                    for pending_info in pending_docs {
                                        if let Ok(doc) = db.get_document(&pending_info.id).await {
                                            if let Some(client) = ws_client.lock().await.as_ref() {
                                                tracing::debug!("📤 CLIENT {}: Uploading pending document: {}", client_id, doc.id);
                                                let _ = client.send(ClientMessage::CreateDocument { 
                                                    document: doc 
                                                }).await;
                                            }
                                        }
                                    }
                                } else {
                                    tracing::info!("✅ CLIENT {}: No pending documents to sync after reconnection", client_id);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!("❌ CLIENT {}: Connection attempt #{} failed: {} - will retry in {}s", client_id, connection_attempts, e, RECONNECTION_INTERVAL.as_secs());
                            event_dispatcher.emit_connection_attempted(&server_url);
                        }
                    }
                } else {
                    // Check if connection is still alive by trying to get the client
                    if ws_client.lock().await.is_none() {
                        tracing::warn!("⚠️ CLIENT {}: Connection flag says connected but no client found - marking as disconnected", client_id);
                        is_connected.store(false, Ordering::Relaxed);
                    }
                }
                
                // Wait before next check/retry
                tokio::time::sleep(RECONNECTION_INTERVAL).await;
            }
        });
    }
}

