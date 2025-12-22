//! C FFI interface for the sync client
//!
//! This module provides C-compatible functions for using the sync client from C/C++.
//! The generated header file will be available after building.

use serde_json::Value;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Arc;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::events::{
    ConflictEventCallback, ConnectionEventCallback, DocumentEventCallback, ErrorEventCallback,
    EventDispatcher, EventType, SyncEventCallback,
};
use crate::{ClientDatabase, SyncEngine as CoreSyncEngine};

/// Opaque handle to a SyncEngine instance
pub struct SyncEngine {
    engine: Option<CoreSyncEngine>,
    database: Arc<ClientDatabase>,
    runtime: Runtime,
    pub(crate) event_dispatcher: Arc<EventDispatcher>,
}

/// Result codes for C API functions
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncResult {
    Success = 0,
    ErrorInvalidInput = -1,
    ErrorConnection = -2,
    ErrorDatabase = -3,
    ErrorSerialization = -4,
    ErrorUnknown = -99,
}

/// Document structure for C API
#[repr(C)]
pub struct Document {
    pub id: *mut c_char,
    pub title: *mut c_char,
    pub content: *mut c_char,
    pub sync_revision: i64,
}

/// Create a new sync engine instance
///
/// # Arguments
/// * `database_url` - SQLite database URL (e.g., "sqlite:client.db?mode=rwc")
/// * `server_url` - WebSocket server URL (e.g., "ws://localhost:8080/ws")
/// * `email` - User email address
/// * `api_key` - Application API key (rpa_ prefix)
/// * `api_secret` - Application API secret (rps_ prefix)
///
/// # Returns
/// * Pointer to SyncEngine on success, null on failure
///
/// # Safety
/// Caller must ensure all pointers are valid, non-null C strings
#[no_mangle]
pub unsafe extern "C" fn sync_engine_create(
    database_url: *const c_char,
    server_url: *const c_char,
    email: *const c_char,
    api_key: *const c_char,
    api_secret: *const c_char,
) -> *mut SyncEngine {
    if database_url.is_null()
        || server_url.is_null()
        || email.is_null()
        || api_key.is_null()
        || api_secret.is_null()
    {
        return ptr::null_mut();
    }

    let database_url = match CStr::from_ptr(database_url).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let server_url = match CStr::from_ptr(server_url).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let email = match CStr::from_ptr(email).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let api_key = match CStr::from_ptr(api_key).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let api_secret = match CStr::from_ptr(api_secret).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return ptr::null_mut(),
    };

    let database = match runtime.block_on(async { ClientDatabase::new(database_url).await }) {
        Ok(db) => Arc::new(db),
        Err(_) => return ptr::null_mut(),
    };

    // Run migrations
    if runtime
        .block_on(async { database.run_migrations().await })
        .is_err()
    {
        return ptr::null_mut();
    }

    let event_dispatcher = Arc::new(EventDispatcher::new());

    // Try to create sync engine (optional - can work offline)
    let engine = runtime.block_on(async {
        let sync_engine = CoreSyncEngine::new(database_url, server_url, email, api_key, api_secret)
            .await
            .ok()?;
        // We can't easily replace the event dispatcher in an existing SyncEngine,
        // so we'll use separate dispatchers for now. In a production system,
        // you'd want to refactor to share the same dispatcher.
        Some(sync_engine)
    });

    Box::into_raw(Box::new(SyncEngine {
        engine,
        database,
        runtime,
        event_dispatcher,
    }))
}

/// Destroy a sync engine instance and free memory
///
/// # Safety
/// Caller must ensure engine pointer was created by sync_engine_create and hasn't been freed
#[no_mangle]
pub unsafe extern "C" fn sync_engine_destroy(engine: *mut SyncEngine) {
    if !engine.is_null() {
        let _ = Box::from_raw(engine);
    }
}

/// Create a new document
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `content_json` - Document content as JSON string (should include any title as part of the JSON)
/// * `out_document_id` - Output buffer for document ID (must be at least 37 chars)
///
/// # Returns
/// * CSyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, content_json is a valid C string, and out_document_id has space for 37 bytes
#[no_mangle]
pub unsafe extern "C" fn sync_engine_create_document(
    engine: *mut SyncEngine,
    content_json: *const c_char,
    out_document_id: *mut c_char,
) -> SyncResult {
    if engine.is_null() || content_json.is_null() || out_document_id.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &mut *engine;

    let content_json = match CStr::from_ptr(content_json).to_str() {
        Ok(s) => s,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let content: Value = match serde_json::from_str(content_json) {
        Ok(c) => c,
        Err(_) => return SyncResult::ErrorSerialization,
    };

    let doc_id = if let Some(ref sync_engine) = engine.engine {
        // Online mode - use sync engine
        match engine
            .runtime
            .block_on(async { sync_engine.create_document(content.clone()).await })
        {
            Ok(doc) => {
                // Emit event to FFI event dispatcher
                engine
                    .event_dispatcher
                    .emit_document_created(&doc.id, &content);
                doc.id
            }
            Err(_) => return SyncResult::ErrorConnection,
        }
    } else {
        // Offline mode - create locally
        let doc_id = Uuid::new_v4();
        let user_id = match engine
            .runtime
            .block_on(async { engine.database.get_user_id().await })
        {
            Ok(id) => id,
            Err(_) => return SyncResult::ErrorDatabase,
        };

        let doc = sync_core::models::Document {
            id: doc_id,
            user_id,
            content: content.clone(),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        if engine
            .runtime
            .block_on(async { engine.database.save_document(&doc).await })
            .is_err()
        {
            return SyncResult::ErrorDatabase;
        }

        // Emit event for offline document creation
        engine
            .event_dispatcher
            .emit_document_created(&doc_id, &content);

        doc_id
    };

    // Copy document ID to output buffer
    let id_string = doc_id.to_string();
    let id_bytes = id_string.as_bytes();
    if id_bytes.len() >= 36 {
        unsafe {
            ptr::copy_nonoverlapping(id_bytes.as_ptr(), out_document_id as *mut u8, 36);
            out_document_id.add(36).write(0); // null terminator
        }
    }

    SyncResult::Success
}

/// Update an existing document
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `document_id` - Document ID to update
/// * `content_json` - New document content as JSON string
///
/// # Returns
/// * CSyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid and both document_id and content_json are valid C strings
#[no_mangle]
pub unsafe extern "C" fn sync_engine_update_document(
    engine: *mut SyncEngine,
    document_id: *const c_char,
    content_json: *const c_char,
) -> SyncResult {
    if engine.is_null() || document_id.is_null() || content_json.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &mut *engine;

    let document_id = match CStr::from_ptr(document_id).to_str() {
        Ok(s) => s,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let doc_uuid = match Uuid::parse_str(document_id) {
        Ok(id) => id,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let content_json = match CStr::from_ptr(content_json).to_str() {
        Ok(s) => s,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let content: Value = match serde_json::from_str(content_json) {
        Ok(c) => c,
        Err(_) => return SyncResult::ErrorSerialization,
    };

    if let Some(ref sync_engine) = engine.engine {
        // Online mode
        match engine
            .runtime
            .block_on(async { sync_engine.update_document(doc_uuid, content).await })
        {
            Ok(_) => SyncResult::Success,
            Err(_) => SyncResult::ErrorConnection,
        }
    } else {
        // Offline mode - update locally
        let doc = match engine
            .runtime
            .block_on(async { engine.database.get_document(&doc_uuid).await })
        {
            Ok(d) => d,
            Err(_) => return SyncResult::ErrorDatabase,
        };

        let mut updated_doc = doc;
        updated_doc.content = content;
        updated_doc.sync_revision += 1;
        updated_doc.content_hash = None; // Will be recalculated on server
        updated_doc.updated_at = chrono::Utc::now();

        match engine
            .runtime
            .block_on(async { engine.database.save_document(&updated_doc).await })
        {
            Ok(_) => {
                // Emit event for offline document update
                engine
                    .event_dispatcher
                    .emit_document_updated(&doc_uuid, &updated_doc.content);
                SyncResult::Success
            }
            Err(_) => SyncResult::ErrorDatabase,
        }
    }
}

/// Delete a document
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `document_id` - Document ID to delete
///
/// # Returns
/// * CSyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid and document_id is a valid C string
#[no_mangle]
pub unsafe extern "C" fn sync_engine_delete_document(
    engine: *mut SyncEngine,
    document_id: *const c_char,
) -> SyncResult {
    if engine.is_null() || document_id.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &mut *engine;

    let document_id = match CStr::from_ptr(document_id).to_str() {
        Ok(s) => s,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let doc_uuid = match Uuid::parse_str(document_id) {
        Ok(id) => id,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    if let Some(ref sync_engine) = engine.engine {
        // Online mode
        match engine
            .runtime
            .block_on(async { sync_engine.delete_document(doc_uuid).await })
        {
            Ok(_) => SyncResult::Success,
            Err(_) => SyncResult::ErrorConnection,
        }
    } else {
        // Offline mode
        match engine
            .runtime
            .block_on(async { engine.database.delete_document(&doc_uuid).await })
        {
            Ok(_) => {
                // Emit event for offline document deletion
                engine.event_dispatcher.emit_document_deleted(&doc_uuid);
                SyncResult::Success
            }
            Err(_) => SyncResult::ErrorDatabase,
        }
    }
}

/// Free a C string allocated by this library
///
/// # Safety
/// Caller must ensure the string was allocated by this library and hasn't been freed
#[no_mangle]
pub unsafe extern "C" fn sync_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = CString::from_raw(s);
    }
}

/// Get library version string
#[no_mangle]
pub extern "C" fn sync_get_version() -> *mut c_char {
    let version = env!("CARGO_PKG_VERSION");
    match CString::new(version) {
        Ok(s) => s.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Register a callback for document events (Created, Updated, Deleted)
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `callback` - C callback function to invoke for document events
/// * `context` - User-defined context pointer passed to callback
/// * `event_filter` - Optional filter: 0=Created, 1=Updated, 2=Deleted, -1=all document events
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, callback is a valid function pointer, and context pointer outlives the callback registration
#[no_mangle]
pub unsafe extern "C" fn sync_engine_register_document_callback(
    engine: *mut SyncEngine,
    callback: DocumentEventCallback,
    context: *mut c_void,
    event_filter: i32,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    let filter = if event_filter >= 0 {
        match event_filter {
            0 => Some(EventType::DocumentCreated),
            1 => Some(EventType::DocumentUpdated),
            2 => Some(EventType::DocumentDeleted),
            _ => return SyncResult::ErrorInvalidInput,
        }
    } else {
        None
    };

    match engine
        .event_dispatcher
        .register_document_callback(callback, context, filter)
    {
        Ok(_) => SyncResult::Success,
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Register a callback for sync events (Started, Completed)
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `callback` - C callback function to invoke for sync events
/// * `context` - User-defined context pointer passed to callback
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, callback is a valid function pointer, and context pointer outlives the callback registration
#[no_mangle]
pub unsafe extern "C" fn sync_engine_register_sync_callback(
    engine: *mut SyncEngine,
    callback: SyncEventCallback,
    context: *mut c_void,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match engine
        .event_dispatcher
        .register_sync_callback(callback, context)
    {
        Ok(_) => SyncResult::Success,
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Register a callback for error events (SyncError)
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `callback` - C callback function to invoke for error events
/// * `context` - User-defined context pointer passed to callback
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, callback is a valid function pointer, and context pointer outlives the callback registration
#[no_mangle]
pub unsafe extern "C" fn sync_engine_register_error_callback(
    engine: *mut SyncEngine,
    callback: ErrorEventCallback,
    context: *mut c_void,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match engine
        .event_dispatcher
        .register_error_callback(callback, context)
    {
        Ok(_) => SyncResult::Success,
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Register a callback for connection events (Lost, Attempted, Succeeded)
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `callback` - C callback function to invoke for connection events
/// * `context` - User-defined context pointer passed to callback
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, callback is a valid function pointer, and context pointer outlives the callback registration
#[no_mangle]
pub unsafe extern "C" fn sync_engine_register_connection_callback(
    engine: *mut SyncEngine,
    callback: ConnectionEventCallback,
    context: *mut c_void,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match engine
        .event_dispatcher
        .register_connection_callback(callback, context)
    {
        Ok(_) => SyncResult::Success,
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Register a callback for conflict events (ConflictDetected)
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `callback` - C callback function to invoke for conflict events
/// * `context` - User-defined context pointer passed to callback
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is valid, callback is a valid function pointer, and context pointer outlives the callback registration
#[no_mangle]
pub unsafe extern "C" fn sync_engine_register_conflict_callback(
    engine: *mut SyncEngine,
    callback: ConflictEventCallback,
    context: *mut c_void,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match engine
        .event_dispatcher
        .register_conflict_callback(callback, context)
    {
        Ok(_) => SyncResult::Success,
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Process all queued events on the current thread
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `out_processed_count` - Output pointer for number of events processed (optional)
///
/// # Returns
/// * CSyncResult indicating success or failure
///
/// # Important
/// This function MUST be called on the same thread where callbacks were registered.
/// Events are queued from any thread but only processed on the callback thread.
///
/// # Safety
/// Caller must ensure engine is valid and out_processed_count points to valid memory (if not null)
#[no_mangle]
pub unsafe extern "C" fn sync_engine_process_events(
    engine: *mut SyncEngine,
    out_processed_count: *mut u32,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match engine.event_dispatcher.process_events() {
        Ok(count) => {
            if !out_processed_count.is_null() {
                out_processed_count.write(count as u32);
            }
            SyncResult::Success
        }
        Err(_) => SyncResult::ErrorUnknown,
    }
}

/// Get a document by ID
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `document_id` - Document ID as UUID string
/// * `out_content` - Output pointer for document JSON content (caller must free with sync_string_free)
///
/// # Returns
/// * SyncResult::Success if document found and content returned
/// * SyncResult::ErrorInvalidInput if document not found or invalid ID
///
/// # Safety
/// Caller must ensure engine is valid, document_id is a valid C string, and out_content is a valid pointer
#[no_mangle]
pub unsafe extern "C" fn sync_engine_get_document(
    engine: *mut SyncEngine,
    document_id: *const c_char,
    out_content: *mut *mut c_char,
) -> SyncResult {
    if engine.is_null() || document_id.is_null() || out_content.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    let document_id = match CStr::from_ptr(document_id).to_str() {
        Ok(s) => s,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let doc_uuid = match Uuid::parse_str(document_id) {
        Ok(id) => id,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    let doc = match engine
        .runtime
        .block_on(async { engine.database.get_document(&doc_uuid).await })
    {
        Ok(d) => d,
        Err(_) => return SyncResult::ErrorInvalidInput,
    };

    // Serialize document to JSON
    let json = match serde_json::to_string(&doc) {
        Ok(j) => j,
        Err(_) => return SyncResult::ErrorSerialization,
    };

    match CString::new(json) {
        Ok(c_str) => {
            *out_content = c_str.into_raw();
            SyncResult::Success
        }
        Err(_) => SyncResult::ErrorSerialization,
    }
}

/// Get all documents as a JSON array
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `out_documents` - Output pointer for JSON array of documents (caller must free with sync_string_free)
///
/// # Returns
/// * SyncResult::Success with JSON array (empty array [] if no documents)
///
/// # Safety
/// Caller must ensure engine is valid and out_documents is a valid pointer
#[no_mangle]
pub unsafe extern "C" fn sync_engine_get_all_documents(
    engine: *mut SyncEngine,
    out_documents: *mut *mut c_char,
) -> SyncResult {
    if engine.is_null() || out_documents.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    let docs = match engine
        .runtime
        .block_on(async { engine.database.get_all_documents().await })
    {
        Ok(d) => d,
        Err(_) => return SyncResult::ErrorDatabase,
    };

    // Serialize documents array to JSON
    let json = match serde_json::to_string(&docs) {
        Ok(j) => j,
        Err(_) => return SyncResult::ErrorSerialization,
    };

    match CString::new(json) {
        Ok(c_str) => {
            *out_documents = c_str.into_raw();
            SyncResult::Success
        }
        Err(_) => SyncResult::ErrorSerialization,
    }
}

/// Get the count of local documents
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `out_count` - Output pointer for document count
///
/// # Returns
/// * SyncResult::Success with count written to out_count
///
/// # Safety
/// Caller must ensure engine is valid and out_count is a valid pointer
#[no_mangle]
pub unsafe extern "C" fn sync_engine_count_documents(
    engine: *mut SyncEngine,
    out_count: *mut u64,
) -> SyncResult {
    if engine.is_null() || out_count.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    let count = match engine
        .runtime
        .block_on(async { engine.database.count_documents().await })
    {
        Ok(d) => d,
        Err(_) => return SyncResult::ErrorDatabase,
    };

    *out_count = count as u64;
    SyncResult::Success
}

/// Check if the sync engine is connected to the server
///
/// # Arguments
/// * `engine` - Sync engine instance
///
/// # Returns
/// * true if connected, false if disconnected or engine is null
///
/// # Safety
/// Caller must ensure engine was created by sync_engine_create
#[no_mangle]
pub unsafe extern "C" fn sync_engine_is_connected(engine: *mut SyncEngine) -> bool {
    if engine.is_null() {
        return false;
    }

    let engine = &*engine;

    match &engine.engine {
        Some(sync_engine) => sync_engine.is_connected(),
        None => false,
    }
}

/// Get the count of documents pending sync to server
///
/// # Arguments
/// * `engine` - Sync engine instance
/// * `out_count` - Output pointer for pending document count
///
/// # Returns
/// * SyncResult::Success with count written to out_count
///
/// # Safety
/// Caller must ensure engine is valid and out_count is a valid pointer
#[no_mangle]
pub unsafe extern "C" fn sync_engine_count_pending_sync(
    engine: *mut SyncEngine,
    out_count: *mut u64,
) -> SyncResult {
    if engine.is_null() || out_count.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    // If we have a sync engine, use it; otherwise check database directly
    let count = if let Some(ref sync_engine) = engine.engine {
        match engine
            .runtime
            .block_on(async { sync_engine.count_pending_sync().await })
        {
            Ok(c) => c,
            Err(_) => return SyncResult::ErrorDatabase,
        }
    } else {
        // Offline mode - check pending documents in database
        match engine
            .runtime
            .block_on(async { engine.database.get_pending_documents().await })
        {
            Ok(docs) => docs.len(),
            Err(_) => return SyncResult::ErrorDatabase,
        }
    };

    *out_count = count as u64;
    SyncResult::Success
}
