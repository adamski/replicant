use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
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
    ws_client: Arc<Mutex<WebSocketClient>>,
    user_id: Uuid,
    node_id: String,
}

impl SyncEngine {
    pub async fn new(
        database_url: &str,
        server_url: &str,
        auth_token: &str,
    ) -> Result<Self, ClientError> {
        let db = Arc::new(ClientDatabase::new(database_url).await?);
        db.run_migrations().await?;
        
        let user_id = db.get_user_id().await?;
        let node_id = format!("client_{}", user_id);
        
        let ws_client = WebSocketClient::connect(server_url, user_id, auth_token).await?;
        
        Ok(Self {
            db,
            ws_client: Arc::new(Mutex::new(ws_client)),
            user_id,
            node_id,
        })
    }
    
    pub async fn start(&self) -> Result<(), ClientError> {
        // Start WebSocket message handler
        let (tx, mut rx) = mpsc::channel::<ServerMessage>(100);
        
        let _ws_client = self.ws_client.clone();
        let db = self.db.clone();
        
        // Spawn message handler
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = Self::handle_server_message(msg, &db).await {
                    tracing::error!("Error handling server message: {}", e);
                }
            }
        });
        
        // Start WebSocket reader
        let mut ws = self.ws_client.lock().await;
        ws.start_reading(tx).await?;
        
        // Initial sync
        self.sync_all().await?;
        
        Ok(())
    }
    
    pub async fn create_document(&self, title: String, content: serde_json::Value) -> Result<Document, ClientError> {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: self.user_id,
            title,
            content,
            revision_id: Uuid::new_v4(),
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
        
        // Save locally
        self.db.save_document(&doc).await?;
        
        // Send to server
        let mut ws = self.ws_client.lock().await;
        ws.send(ClientMessage::CreateDocument { document: doc.clone() }).await?;
        
        Ok(doc)
    }
    
    pub async fn update_document(&self, id: Uuid, new_content: serde_json::Value) -> Result<(), ClientError> {
        let mut doc = self.db.get_document(&id).await?;
        let old_content = doc.content.clone();
        
        // Create patch
        let patch = create_patch(&old_content, &new_content)?;
        
        // Update document
        doc.content = new_content;
        doc.revision_id = Uuid::new_v4();
        doc.version += 1;
        doc.vector_clock.increment(&self.node_id);
        doc.updated_at = chrono::Utc::now();
        
        // Save locally
        self.db.save_document(&doc).await?;
        
        // Create patch message
        let patch_msg = DocumentPatch {
            document_id: id,
            revision_id: doc.revision_id,
            patch,
            vector_clock: doc.vector_clock.clone(),
            checksum: calculate_checksum(&doc.content),
        };
        
        // Send to server
        let mut ws = self.ws_client.lock().await;
        ws.send(ClientMessage::UpdateDocument { patch: patch_msg }).await?;
        
        Ok(())
    }
    
    async fn handle_server_message(msg: ServerMessage, db: &Arc<ClientDatabase>) -> Result<(), ClientError> {
        match msg {
            ServerMessage::DocumentUpdated { patch } => {
                // Apply patch from server
                let mut doc = db.get_document(&patch.document_id).await?;
                
                // Check for conflicts
                if doc.vector_clock.is_concurrent(&patch.vector_clock) {
                    tracing::warn!("Conflict detected for document {}", patch.document_id);
                    // Handle conflict - for now, server wins
                }
                
                // Apply patch
                apply_patch(&mut doc.content, &patch.patch)?;
                doc.revision_id = patch.revision_id;
                doc.vector_clock.merge(&patch.vector_clock);
                doc.updated_at = chrono::Utc::now();
                
                db.save_document(&doc).await?;
                db.mark_synced(&doc.id, &doc.revision_id).await?;
            }
            ServerMessage::DocumentCreated { document } => {
                // New document from server
                db.save_document(&document).await?;
                db.mark_synced(&document.id, &document.revision_id).await?;
            }
            ServerMessage::ConflictDetected { document_id, .. } => {
                tracing::warn!("Conflict detected for document {}", document_id);
                // Implement conflict resolution UI callback
            }
            _ => {}
        }
        
        Ok(())
    }
    
    async fn sync_all(&self) -> Result<(), ClientError> {
        let pending = self.db.get_pending_documents().await?;
        
        let mut ws = self.ws_client.lock().await;
        ws.send(ClientMessage::RequestSync { document_ids: pending }).await?;
        
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