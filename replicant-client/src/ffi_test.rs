//! C FFI test functions for the Replicant client
//!
//! This module provides test-only C-compatible functions for development and testing.
//! These functions are only available in debug builds.

use crate::ffi::{Replicant, SyncResult};
use uuid::Uuid;

/// Trigger a test event (for development/testing purposes)
///
/// # Arguments
/// * `engine` - Replicant client instance
/// * `event_type` - Event type to emit (0-7)
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Event Types
/// * 0 - DocumentCreated
/// * 1 - DocumentUpdated
/// * 2 - DocumentDeleted
/// * 3 - SyncStarted
/// * 4 - SyncCompleted
/// * 5 - SyncError
/// * 6 - ConflictDetected
/// * 7 - ConnectionLost
/// * 8 - ConnectionAttempted
/// * 9 - ConnectionSucceeded
///
/// # Safety
/// Caller must ensure engine is a valid pointer
#[cfg(debug_assertions)]
#[no_mangle]
pub unsafe extern "C" fn replicant_emit_test_event(
    engine: *mut Replicant,
    event_type: i32,
) -> SyncResult {
    if engine.is_null() {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    match event_type {
        0 => {
            let test_id = Uuid::new_v4();
            let test_content = serde_json::json!({"title": "Test Document (Created)", "test": "data", "created_at": chrono::Utc::now()});
            engine
                .event_dispatcher
                .emit_document_created(&test_id, &test_content);
        }
        1 => {
            let test_id = Uuid::new_v4();
            let test_content = serde_json::json!({"title": "Test Document (Updated)", "test": "updated", "updated_at": chrono::Utc::now()});
            engine
                .event_dispatcher
                .emit_document_updated(&test_id, &test_content);
        }
        2 => {
            let test_id = Uuid::new_v4();
            engine.event_dispatcher.emit_document_deleted(&test_id);
        }
        3 => engine.event_dispatcher.emit_sync_started(),
        4 => engine.event_dispatcher.emit_sync_completed(5),
        5 => engine
            .event_dispatcher
            .emit_sync_error("Test error message from replicant_emit_test_event"),
        6 => {
            let test_id = Uuid::new_v4();
            engine.event_dispatcher.emit_conflict_detected(&test_id);
        }
        7 => engine.event_dispatcher.emit_connection_lost("test-server"),
        8 => engine
            .event_dispatcher
            .emit_connection_attempted("test-server"),
        9 => engine
            .event_dispatcher
            .emit_connection_succeeded("test-server"),
        _ => return SyncResult::ErrorInvalidInput,
    }

    SyncResult::Success
}

/// Trigger multiple test events in sequence (for stress testing callbacks)
///
/// # Arguments
/// * `engine` - Replicant client instance
/// * `count` - Number of events to emit (1-100)
///
/// # Returns
/// * SyncResult indicating success or failure
///
/// # Safety
/// Caller must ensure engine is a valid pointer
#[cfg(debug_assertions)]
#[no_mangle]
pub unsafe extern "C" fn replicant_emit_test_event_burst(
    engine: *mut Replicant,
    count: i32,
) -> SyncResult {
    if engine.is_null() || !(1..=100).contains(&count) {
        return SyncResult::ErrorInvalidInput;
    }

    let engine = &*engine;

    for i in 0..count {
        let test_id = Uuid::new_v4();
        let test_content = serde_json::json!({
            "title": format!("Burst Test Document {}", i),
            "test": "burst_event",
            "sequence": i,
            "timestamp": chrono::Utc::now()
        });
        engine
            .event_dispatcher
            .emit_document_created(&test_id, &test_content);
    }

    SyncResult::Success
}
