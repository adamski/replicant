use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;
use sync_core::{
    models::{Document, DocumentPatch, VectorClock},
    protocol::{ClientMessage, ServerMessage},
    patches::{create_patch, apply_patch, calculate_checksum},
};
use crate::{
    database::ClientDatabase,
    websocket::WebSocketClient,
    errors::ClientError,
};

pub struct SyncEngine {
    db: Arc<ClientDatabase>,
    ws_client: Arc<WebSocketClient>,
    user_id: Uuid,
    client_id: Uuid,
    node_id: String,
    message_rx: Option<mpsc::Receiver<ServerMessage>>,
}

impl SyncEngine {
    pub async fn new(
        database_url: &str,
        server_url: &str,
        auth_token: &str,
    ) -> Result<Self, ClientError> {
        let db = Arc::new(ClientDatabase::new(database_url).await?);
        db.run_migrations().await?;
        
        let (user_id, client_id) = db.get_user_and_client_id().await?;
        let node_id = format!("client_{}", user_id);
        
        let (ws_client, ws_receiver) = WebSocketClient::connect(server_url, user_id, client_id, auth_token).await?;
        
        // Create a channel for messages
        let (tx, rx) = mpsc::channel(100);
        
        // Start forwarding WebSocket messages to our channel
        tokio::spawn(async move {
            if let Err(e) = ws_receiver.forward_to(tx).await {
                tracing::error!("WebSocket receiver error: {}", e);
            }
        });
        
        let engine = Self {
            db,
            ws_client: Arc::new(ws_client),
            user_id,
            client_id,
            node_id,
            message_rx: Some(rx),
        };
        
        Ok(engine)
    }
    
    pub async fn start(&mut self) -> Result<(), ClientError> {
        // Take the receiver - can only start once
        let rx = self.message_rx.take()
            .ok_or_else(|| ClientError::WebSocket("SyncEngine already started".to_string()))?;
        
        let db = self.db.clone();
        let client_id = self.client_id;
        
        // Spawn message handler
        tokio::spawn(async move {
            let mut rx = rx;
            tracing::info!("CLIENT {}: Message handler started", client_id);
            while let Some(msg) = rx.recv().await {
                tracing::info!("CLIENT {}: Processing server message: {:?}", client_id, std::mem::discriminant(&msg));
                if let Err(e) = Self::handle_server_message(msg, &db, client_id).await {
                    tracing::error!("CLIENT {}: Error handling server message: {}", client_id, e);
                } else {
                    tracing::info!("CLIENT {}: Successfully processed server message", client_id);
                }
            }
            tracing::warn!("CLIENT {}: Message handler terminated", client_id);
        });
        
        // Initial sync
        self.sync_all().await?;
        
        Ok(())
    }
    
    pub async fn create_document(&self, title: String, content: serde_json::Value) -> Result<Document, ClientError> {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: self.user_id,
            title: title.clone(),
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
        
        // Save locally first
        tracing::info!("CLIENT: Creating document locally: {} ({})", doc.id, title);
        self.db.save_document(&doc).await?;
        
        // Send to server
        tracing::info!("CLIENT: Sending CreateDocument to server: {} ({})", doc.id, title);
        self.ws_client.send(ClientMessage::CreateDocument { document: doc.clone() }).await?;
        tracing::info!("CLIENT: Successfully sent CreateDocument to server: {} ({})", doc.id, title);
        
        Ok(doc)
    }
    
    pub async fn update_document(&self, id: Uuid, new_content: serde_json::Value) -> Result<(), ClientError> {
        let mut doc = self.db.get_document(&id).await?;
        let old_content = doc.content.clone();
        
        tracing::info!("CLIENT {}: Updating document {} - old content: {:?}, new content: {:?}", 
                     self.client_id, id, old_content, new_content);
        
        // Create patch
        let patch = create_patch(&old_content, &new_content)?;
        
        // Update document
        doc.revision_id = doc.next_revision(&new_content);
        doc.content = new_content;
        doc.version += 1;
        doc.vector_clock.increment(&self.node_id);
        doc.updated_at = chrono::Utc::now();
        
        // Save locally
        self.db.save_document(&doc).await?;
        
        // Create patch message
        let patch_msg = DocumentPatch {
            document_id: id,
            revision_id: doc.revision_id.clone(),
            patch,
            vector_clock: doc.vector_clock.clone(),
            checksum: calculate_checksum(&doc.content),
        };
        
        // Send to server
        tracing::info!("CLIENT {}: Sending update to server for doc {}", self.client_id, id);
        self.ws_client.send(ClientMessage::UpdateDocument { patch: patch_msg }).await?;
        tracing::info!("CLIENT {}: Update sent successfully", self.client_id);
        
        Ok(())
    }
    
    pub async fn delete_document(&self, id: Uuid) -> Result<(), ClientError> {
        let doc = self.db.get_document(&id).await?;
        
        // Send delete to server
        self.ws_client.send(ClientMessage::DeleteDocument {
            document_id: id,
            revision_id: doc.revision_id,
        }).await?;
        
        // Mark as deleted locally
        self.db.delete_document(&id).await?;
        
        Ok(())
    }
    
    pub async fn get_all_documents(&self) -> Result<Vec<Document>, ClientError> {
        let docs = self.db.get_all_documents().await?;
        tracing::info!("CLIENT: get_all_documents() returning {} documents", docs.len());
        for doc in &docs {
            tracing::info!("CLIENT:   - Document: {} ({})", doc.id, doc.title);
        }
        Ok(docs)
    }
    
    async fn handle_server_message(msg: ServerMessage, db: &Arc<ClientDatabase>, client_id: Uuid) -> Result<(), ClientError> {
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
            }
            ServerMessage::DocumentCreated { document } => {
                // New document from server - check if we already have it to avoid duplicates
                tracing::info!("CLIENT: Received DocumentCreated from server: {} ({})", document.id, document.title);
                
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
                            db.save_document(&document).await?;
                            db.mark_synced(&document.id, &document.revision_id).await?;
                        }
                    }
                    Err(_) => {
                        // Document doesn't exist locally - save it
                        tracing::info!("CLIENT: Document {} is new, saving to local database", document.id);
                        db.save_document(&document).await?;
                        db.mark_synced(&document.id, &document.revision_id).await?;
                    }
                }
            }
            ServerMessage::DocumentDeleted { document_id, .. } => {
                // Document deleted from server
                db.delete_document(&document_id).await?;
            }
            ServerMessage::ConflictDetected { document_id, .. } => {
                tracing::warn!("Conflict detected for document {}", document_id);
                // Implement conflict resolution UI callback
            }
            ServerMessage::SyncDocument { document } => {
                // Document sync - check if it's newer than what we have
                tracing::info!("CLIENT {}: Received SyncDocument: {} (rev: {})", client_id, document.id, document.revision_id);
                
                match db.get_document(&document.id).await {
                    Ok(local_doc) => {
                        // Compare revisions
                        let local_rev_parts: Vec<&str> = local_doc.revision_id.split('-').collect();
                        let sync_rev_parts: Vec<&str> = document.revision_id.split('-').collect();
                        
                        if local_rev_parts.len() == 2 && sync_rev_parts.len() == 2 {
                            let local_gen: u32 = local_rev_parts[0].parse().unwrap_or(0);
                            let sync_gen: u32 = sync_rev_parts[0].parse().unwrap_or(0);
                            
                            if sync_gen >= local_gen {
                                tracing::info!("CLIENT {}: Updating to newer version (gen {} -> {})", 
                                             client_id, local_gen, sync_gen);
                                db.save_document(&document).await?;
                                db.mark_synced(&document.id, &document.revision_id).await?;
                            } else {
                                tracing::info!("CLIENT {}: Skipping older sync (local gen {} > sync gen {})", 
                                             client_id, local_gen, sync_gen);
                            }
                        } else {
                            // Can't compare, accept the sync
                            db.save_document(&document).await?;
                            db.mark_synced(&document.id, &document.revision_id).await?;
                        }
                    }
                    Err(_) => {
                        // Document doesn't exist locally - save it
                        tracing::info!("CLIENT {}: Document {} is new, saving", client_id, document.id);
                        db.save_document(&document).await?;
                        db.mark_synced(&document.id, &document.revision_id).await?;
                    }
                }
            }
            ServerMessage::SyncComplete { synced_count } => {
                tracing::debug!("Sync complete, received {} documents", synced_count);
            }
            _ => {}
        }
        
        Ok(())
    }
    
    pub async fn sync_all(&self) -> Result<(), ClientError> {
        // Request full sync on startup to get all documents
        tracing::debug!("Requesting full sync from server");
        self.ws_client.send(ClientMessage::RequestFullSync).await?;
        
        Ok(())
    }
}

// C FFI exports for C++ integration
#[no_mangle]
pub extern "C" fn sync_engine_create(
    database_path: *const std::os::raw::c_char,
    server_url: *const std::os::raw::c_char,
    auth_token: *const std::os::raw::c_char,
) -> *mut SyncEngine {
    use std::ffi::CStr;
    
    let database_path = unsafe { CStr::from_ptr(database_path).to_string_lossy() };
    let server_url = unsafe { CStr::from_ptr(server_url).to_string_lossy() };
    let auth_token = unsafe { CStr::from_ptr(auth_token).to_string_lossy() };
    
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let engine = runtime.block_on(async {
        SyncEngine::new(&database_path, &server_url, &auth_token).await.ok()
    });
    
    match engine {
        Some(e) => Box::into_raw(Box::new(e)),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn sync_engine_destroy(engine: *mut SyncEngine) {
    if !engine.is_null() {
        unsafe { drop(Box::from_raw(engine)); }
    }
}