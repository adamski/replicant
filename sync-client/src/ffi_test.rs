//! C FFI test functions for the sync client
//! 
//! This module provides test-only C-compatible functions for development and testing.
//! These functions are only available in debug builds.

use uuid::Uuid;
use crate::ffi::{CSyncEngine, CSyncResult};

/// Trigger a test event (for development/testing purposes)
/// 
/// # Arguments
/// * `engine` - Sync engine instance
/// * `event_type` - Event type to emit (0-7)
/// 
/// # Returns
/// * CSyncResult indicating success or failure
/// 
/// # Event Types
/// * 0 - DocumentCreated
/// * 1 - DocumentUpdated  
/// * 2 - DocumentDeleted
/// * 3 - SyncStarted
/// * 4 - SyncCompleted
/// * 5 - SyncError
/// * 6 - ConflictDetected
/// * 7 - ConnectionStateChanged
#[cfg(debug_assertions)]
#[no_mangle]
pub extern "C" fn sync_engine_emit_test_event(
    engine: *mut CSyncEngine,
    event_type: i32,
) -> CSyncResult {
    if engine.is_null() {
        return CSyncResult::ErrorInvalidInput;
    }

    let engine = unsafe { &*engine };
    
    match event_type {
        0 => {
            let test_id = Uuid::new_v4();
            let test_content = serde_json::json!({"test": "data", "created_at": chrono::Utc::now()});
            engine.event_dispatcher.emit_document_created(&test_id, "Test Document (Created)", &test_content);
        },
        1 => {
            let test_id = Uuid::new_v4();
            let test_content = serde_json::json!({"test": "updated", "updated_at": chrono::Utc::now()});
            engine.event_dispatcher.emit_document_updated(&test_id, "Test Document (Updated)", &test_content);
        },
        2 => {
            let test_id = Uuid::new_v4();
            engine.event_dispatcher.emit_document_deleted(&test_id);
        },
        3 => engine.event_dispatcher.emit_sync_started(),
        4 => engine.event_dispatcher.emit_sync_completed(5),
        5 => engine.event_dispatcher.emit_sync_error("Test error message from sync_engine_emit_test_event"),
        6 => {
            let test_id = Uuid::new_v4();
            engine.event_dispatcher.emit_conflict_detected(&test_id);
        },
        7 => engine.event_dispatcher.emit_connection_state_changed(true),
        _ => return CSyncResult::ErrorInvalidInput,
    }

    CSyncResult::Success
}

/// Trigger multiple test events in sequence (for stress testing callbacks)
/// 
/// # Arguments
/// * `engine` - Sync engine instance
/// * `count` - Number of events to emit (1-100)
/// 
/// # Returns
/// * CSyncResult indicating success or failure
#[cfg(debug_assertions)]
#[no_mangle]
pub extern "C" fn sync_engine_emit_test_event_burst(
    engine: *mut CSyncEngine,
    count: i32,
) -> CSyncResult {
    if engine.is_null() || count < 1 || count > 100 {
        return CSyncResult::ErrorInvalidInput;
    }

    let engine = unsafe { &*engine };
    
    for i in 0..count {
        let test_id = Uuid::new_v4();
        let test_content = serde_json::json!({
            "test": "burst_event",
            "sequence": i,
            "timestamp": chrono::Utc::now()
        });
        engine.event_dispatcher.emit_document_created(&test_id, &format!("Burst Test Document {}", i), &test_content);
    }

    CSyncResult::Success
}