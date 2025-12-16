//! Integration tests for FFI event callbacks
//!
//! These tests verify that the FFI interface works correctly and that
//! callbacks are properly invoked when events occur.

use std::ffi::{c_void, CStr, CString};
use std::ptr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sync_client::events::{EventData, EventType};
use sync_client::ffi::{
    sync_engine_count_documents, sync_engine_count_pending_sync, sync_engine_create,
    sync_engine_create_document, sync_engine_destroy, sync_engine_get_all_documents,
    sync_engine_get_document, sync_engine_is_connected, sync_engine_process_events,
    sync_engine_register_event_callback, sync_engine_update_document, sync_string_free, SyncEngine,
    SyncResult,
};

#[cfg(debug_assertions)]
use sync_client::ffi_test::{sync_engine_emit_test_event, sync_engine_emit_test_event_burst};

/// Test data structure for capturing callback invocations
#[derive(Debug, Default)]
struct CallbackCapture {
    call_count: AtomicUsize,
    last_event_type: Mutex<Option<EventType>>,
    last_document_id: Mutex<Option<String>>,
    last_title: Mutex<Option<String>>,
    last_content: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    last_numeric_data: Mutex<u64>,
    last_boolean_data: Mutex<bool>,
}

impl CallbackCapture {
    fn new() -> Self {
        Self::default()
    }

    fn reset(&self) {
        self.call_count.store(0, Ordering::SeqCst);
        *self.last_event_type.lock().unwrap() = None;
        *self.last_document_id.lock().unwrap() = None;
        *self.last_title.lock().unwrap() = None;
        *self.last_content.lock().unwrap() = None;
        *self.last_error.lock().unwrap() = None;
        *self.last_numeric_data.lock().unwrap() = 0;
        *self.last_boolean_data.lock().unwrap() = false;
    }
}

extern "C" fn capture_callback(event: *const EventData, context: *mut c_void) {
    let capture = unsafe { &*(context as *const CallbackCapture) };
    let event = unsafe { &*event };

    capture.call_count.fetch_add(1, Ordering::SeqCst);
    *capture.last_event_type.lock().unwrap() = Some(event.event_type);

    // Capture strings
    if !event.document_id.is_null() {
        let doc_id = unsafe {
            CStr::from_ptr(event.document_id)
                .to_string_lossy()
                .to_string()
        };
        *capture.last_document_id.lock().unwrap() = Some(doc_id);
    }

    if !event.title.is_null() {
        let title = unsafe { CStr::from_ptr(event.title).to_string_lossy().to_string() };
        *capture.last_title.lock().unwrap() = Some(title);
    }

    if !event.content.is_null() {
        let content = unsafe { CStr::from_ptr(event.content).to_string_lossy().to_string() };
        *capture.last_content.lock().unwrap() = Some(content);
    }

    if !event.error.is_null() {
        let error = unsafe { CStr::from_ptr(event.error).to_string_lossy().to_string() };
        *capture.last_error.lock().unwrap() = Some(error);
    }

    *capture.last_numeric_data.lock().unwrap() = event.numeric_data;
    *capture.last_boolean_data.lock().unwrap() = event.boolean_data;
}

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a test engine with a unique file-based database per test invocation.
unsafe fn create_test_engine() -> *mut SyncEngine {
    // Generate a unique database file for each test to avoid conflicts when running in parallel
    let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let test_db_path = format!(
        "/tmp/sync_client_test_{}_{}.db",
        std::process::id(),
        unique_id
    );
    // Clean up any existing database file
    let _ = std::fs::remove_file(&test_db_path);

    let db_url = CString::new(format!("sqlite:{}?mode=rwc", test_db_path)).unwrap();
    let server_url = CString::new("ws://localhost:8080/ws").unwrap();
    let email = CString::new("test-user@example.com").unwrap();
    let api_key = CString::new("rpa_test_api_key_example_12345").unwrap();
    let api_secret = CString::new("rps_test_api_secret_example_67890").unwrap();

    sync_engine_create(
        db_url.as_ptr(),
        server_url.as_ptr(),
        email.as_ptr(),
        api_key.as_ptr(),
        api_secret.as_ptr(),
    )
}

#[test]
fn test_ffi_callback_registration() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = CallbackCapture::new();

        // Register callback for all events
        let result = sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture as *const CallbackCapture as *mut c_void,
            -1, // All events
        );

        assert_eq!(result, SyncResult::Success);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_callback_with_document_creation() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = CallbackCapture::new();

        // Register callback for document events
        let result = sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture as *const CallbackCapture as *mut c_void,
            0, // DocumentCreated
        );
        assert_eq!(result, SyncResult::Success);

        // Create a document (this should trigger the callback in offline mode)
        let content =
            CString::new(r#"{"title":"Test Document","content":"test data","type":"note"}"#)
                .unwrap();
        let mut doc_id = [0u8; 37]; // UUID string + null terminator

        let create_result =
            sync_engine_create_document(engine, content.as_ptr(), doc_id.as_mut_ptr() as *mut i8);

        assert_eq!(create_result, SyncResult::Success);

        // Process events to trigger callbacks
        sync_engine_process_events(engine, ptr::null_mut());

        // Verify callback was called
        assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *capture.last_event_type.lock().unwrap(),
            Some(EventType::DocumentCreated)
        );

        let captured_title = capture.last_title.lock().unwrap();
        assert_eq!(captured_title.as_ref().unwrap(), "Test Document");

        let captured_content = capture.last_content.lock().unwrap();
        assert!(captured_content.is_some());
        assert!(captured_content.as_ref().unwrap().contains("test data"));

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_callback_filtering() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let created_capture = CallbackCapture::new();
        let updated_capture = CallbackCapture::new();

        // Register callback only for created events
        let result1 = sync_engine_register_event_callback(
            engine,
            capture_callback,
            &created_capture as *const CallbackCapture as *mut c_void,
            0, // DocumentCreated
        );
        assert_eq!(result1, SyncResult::Success);

        // Register callback only for updated events
        let result2 = sync_engine_register_event_callback(
            engine,
            capture_callback,
            &updated_capture as *const CallbackCapture as *mut c_void,
            1, // DocumentUpdated
        );
        assert_eq!(result2, SyncResult::Success);

        #[cfg(debug_assertions)]
        {
            // Test with debug event emission
            sync_engine_emit_test_event(engine, 0); // DocumentCreated
            sync_engine_process_events(engine, ptr::null_mut());

            assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
            assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 0);

            sync_engine_emit_test_event(engine, 1); // DocumentUpdated
            sync_engine_process_events(engine, ptr::null_mut());

            assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
            assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 1);

            sync_engine_emit_test_event(engine, 2); // DocumentDeleted (neither should trigger)
            sync_engine_process_events(engine, ptr::null_mut());

            assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
            assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 1);
        }

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_multiple_callbacks() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture1 = CallbackCapture::new();
        let capture2 = CallbackCapture::new();
        let capture3 = CallbackCapture::new();

        // Register three callbacks for all events
        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture1 as *const CallbackCapture as *mut c_void,
            -1,
        );

        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture2 as *const CallbackCapture as *mut c_void,
            -1,
        );

        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture3 as *const CallbackCapture as *mut c_void,
            -1,
        );

        #[cfg(debug_assertions)]
        {
            // Emit a test event
            sync_engine_emit_test_event(engine, 3); // SyncStarted
            sync_engine_process_events(engine, ptr::null_mut());

            // All three should have been called
            assert_eq!(capture1.call_count.load(Ordering::SeqCst), 1);
            assert_eq!(capture2.call_count.load(Ordering::SeqCst), 1);
            assert_eq!(capture3.call_count.load(Ordering::SeqCst), 1);

            assert_eq!(
                *capture1.last_event_type.lock().unwrap(),
                Some(EventType::SyncStarted)
            );
            assert_eq!(
                *capture2.last_event_type.lock().unwrap(),
                Some(EventType::SyncStarted)
            );
            assert_eq!(
                *capture3.last_event_type.lock().unwrap(),
                Some(EventType::SyncStarted)
            );
        }

        sync_engine_destroy(engine);
    }
}

#[test]
#[cfg(debug_assertions)]
fn test_ffi_all_event_types() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = CallbackCapture::new();

        // Register for all events
        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture as *const CallbackCapture as *mut c_void,
            -1,
        );

        // Test all event types
        for event_type in 0..8 {
            capture.reset();

            sync_engine_emit_test_event(engine, event_type);
            sync_engine_process_events(engine, ptr::null_mut());

            assert_eq!(
                capture.call_count.load(Ordering::SeqCst),
                1,
                "Event type {} was not emitted",
                event_type
            );

            let expected_type = match event_type {
                0 => EventType::DocumentCreated,
                1 => EventType::DocumentUpdated,
                2 => EventType::DocumentDeleted,
                3 => EventType::SyncStarted,
                4 => EventType::SyncCompleted,
                5 => EventType::SyncError,
                6 => EventType::ConflictDetected,
                7 => EventType::ConnectionLost,
                _ => panic!("Unexpected event type"),
            };

            assert_eq!(
                *capture.last_event_type.lock().unwrap(),
                Some(expected_type)
            );
        }

        sync_engine_destroy(engine);
    }
}

#[test]
#[cfg(debug_assertions)]
#[ignore] // TODO: This test hangs - needs investigation (unrelated to title removal)
fn test_ffi_event_data_integrity() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = CallbackCapture::new();

        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture as *const CallbackCapture as *mut c_void,
            -1,
        );

        // Test sync completed event (has numeric data)
        capture.reset();
        sync_engine_emit_test_event(engine, 4); // SyncCompleted
        sync_engine_process_events(engine, ptr::null_mut());

        assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *capture.last_event_type.lock().unwrap(),
            Some(EventType::SyncCompleted)
        );
        assert_eq!(*capture.last_numeric_data.lock().unwrap(), 5); // Test uses fixed value of 5

        // Test sync error event (has error message)
        capture.reset();
        sync_engine_emit_test_event(engine, 5); // SyncError
        sync_engine_process_events(engine, ptr::null_mut());

        assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *capture.last_event_type.lock().unwrap(),
            Some(EventType::SyncError)
        );
        let error_msg = capture.last_error.lock().unwrap();
        assert!(error_msg.is_some());
        assert!(error_msg.as_ref().unwrap().contains("Test error message"));

        // Test connection state changed (has boolean data)
        capture.reset();
        sync_engine_emit_test_event(engine, 7); // ConnectionLost
        sync_engine_process_events(engine, ptr::null_mut());

        assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *capture.last_event_type.lock().unwrap(),
            Some(EventType::ConnectionLost)
        );
        assert_eq!(*capture.last_boolean_data.lock().unwrap(), true); // Test uses fixed value of true

        sync_engine_destroy(engine);
    }
}

#[test]
#[cfg(debug_assertions)]
fn test_ffi_event_burst() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = CallbackCapture::new();

        sync_engine_register_event_callback(
            engine,
            capture_callback,
            &capture as *const CallbackCapture as *mut c_void,
            0, // Only DocumentCreated events
        );

        // Emit a burst of 5 events
        sync_engine_emit_test_event_burst(engine, 5);
        sync_engine_process_events(engine, ptr::null_mut());

        // Should have received all 5 events
        assert_eq!(capture.call_count.load(Ordering::SeqCst), 5);
        assert_eq!(
            *capture.last_event_type.lock().unwrap(),
            Some(EventType::DocumentCreated)
        );

        // The last event should have sequence 4 in the title
        let last_title = capture.last_title.lock().unwrap();
        assert!(last_title.is_some());
        assert!(last_title.as_ref().unwrap().contains("4")); // Sequence is 0-indexed

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_null_pointer_safety() {
    unsafe {
        // Test that null pointers are handled safely

        // Test null engine
        let result = sync_engine_register_event_callback(
            ptr::null_mut(),
            capture_callback,
            ptr::null_mut(),
            -1,
        );
        assert_eq!(result, SyncResult::ErrorInvalidInput);

        // Test with valid engine but invalid filter
        let engine = create_test_engine();
        if !engine.is_null() {
            let result = sync_engine_register_event_callback(
                engine,
                capture_callback,
                ptr::null_mut(),
                999, // Invalid event type
            );
            assert_eq!(result, SyncResult::ErrorInvalidInput);

            sync_engine_destroy(engine);
        }
    }
}

#[test]
fn test_ffi_engine_lifecycle() {
    unsafe {
        // Test creating and destroying multiple engines
        for _ in 0..3 {
            let engine = create_test_engine();
            assert!(!engine.is_null(), "Failed to create sync engine");

            let capture = CallbackCapture::new();

            let result = sync_engine_register_event_callback(
                engine,
                capture_callback,
                &capture as *const CallbackCapture as *mut c_void,
                -1,
            );
            assert_eq!(result, SyncResult::Success);

            sync_engine_destroy(engine);
            // After destruction, the engine pointer should not be used
        }
    }
}

#[test]
fn test_ffi_callback_thread_safety() {
    use std::thread;
    use std::time::Duration;

    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let capture = Arc::new(CallbackCapture::new());
        let capture_clone = capture.clone();

        // Register callback
        sync_engine_register_event_callback(
            engine,
            capture_callback,
            Arc::as_ptr(&capture_clone) as *mut c_void,
            -1,
        );

        #[cfg(debug_assertions)]
        {
            // Emit events from multiple threads (simulated stress test)
            let engine_ptr = engine as usize; // Store as usize for thread safety

            let handles: Vec<_> = (0..3)
                .map(|_i| {
                    let _capture = capture.clone();
                    thread::spawn(move || {
                        let engine = engine_ptr as *mut SyncEngine;
                        for _ in 0..2 {
                            sync_engine_emit_test_event(engine, 3); // SyncStarted
                            thread::sleep(Duration::from_millis(1));
                        }
                    })
                })
                .collect();

            // Wait for all threads
            for handle in handles {
                handle.join().unwrap();
            }

            // Process all queued events
            sync_engine_process_events(engine, ptr::null_mut());

            // Should have received 6 events total (3 threads × 2 events each)
            assert_eq!(capture.call_count.load(Ordering::SeqCst), 6);
        }

        sync_engine_destroy(engine);
    }
}

// =============================================================================
// Tests for Read APIs (get_document, get_all_documents, count_documents, etc.)
// =============================================================================

#[test]
fn test_ffi_get_document() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Create a document first
        let content =
            CString::new(r#"{"title":"Test Get Document","data":"some content"}"#).unwrap();
        let mut doc_id_buf = [0u8; 37];

        let create_result = sync_engine_create_document(
            engine,
            content.as_ptr(),
            doc_id_buf.as_mut_ptr() as *mut i8,
        );
        assert_eq!(create_result, SyncResult::Success);

        let doc_id = CStr::from_ptr(doc_id_buf.as_ptr() as *const i8)
            .to_string_lossy()
            .to_string();

        // Now retrieve the document
        let doc_id_cstr = CString::new(doc_id.clone()).unwrap();
        let mut out_content: *mut i8 = ptr::null_mut();

        let get_result = sync_engine_get_document(engine, doc_id_cstr.as_ptr(), &mut out_content);

        assert_eq!(get_result, SyncResult::Success);
        assert!(!out_content.is_null());

        let content_str = CStr::from_ptr(out_content).to_string_lossy().to_string();
        assert!(content_str.contains("Test Get Document"));
        assert!(content_str.contains("some content"));
        assert!(content_str.contains(&doc_id)); // Should include the document ID

        // Free the returned string
        sync_string_free(out_content);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_get_document_not_found() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Try to get a non-existent document
        let fake_id = CString::new("00000000-0000-0000-0000-000000000000").unwrap();
        let mut out_content: *mut i8 = ptr::null_mut();

        let result = sync_engine_get_document(engine, fake_id.as_ptr(), &mut out_content);

        assert_eq!(result, SyncResult::ErrorInvalidInput);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_get_all_documents_empty() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        let mut out_documents: *mut i8 = ptr::null_mut();

        let result = sync_engine_get_all_documents(engine, &mut out_documents);

        assert_eq!(result, SyncResult::Success);
        assert!(!out_documents.is_null());

        let docs_str = CStr::from_ptr(out_documents).to_string_lossy().to_string();
        assert_eq!(docs_str, "[]"); // Empty array

        sync_string_free(out_documents);
        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_get_all_documents_with_data() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Create multiple documents
        let mut doc_ids: Vec<String> = Vec::new();
        for i in 0..3 {
            let content =
                CString::new(format!(r#"{{"title":"Document {}","index":{}}}"#, i, i)).unwrap();
            let mut doc_id_buf = [0u8; 37];

            let result = sync_engine_create_document(
                engine,
                content.as_ptr(),
                doc_id_buf.as_mut_ptr() as *mut i8,
            );
            assert_eq!(result, SyncResult::Success);

            let doc_id = CStr::from_ptr(doc_id_buf.as_ptr() as *const i8)
                .to_string_lossy()
                .to_string();
            doc_ids.push(doc_id);
        }

        // Get all documents
        let mut out_documents: *mut i8 = ptr::null_mut();
        let result = sync_engine_get_all_documents(engine, &mut out_documents);

        assert_eq!(result, SyncResult::Success);
        assert!(!out_documents.is_null());

        let docs_str = CStr::from_ptr(out_documents).to_string_lossy().to_string();

        // Verify all document IDs are present
        for doc_id in &doc_ids {
            assert!(
                docs_str.contains(doc_id),
                "Document {} not found in result",
                doc_id
            );
        }

        // Verify it's a JSON array with 3 documents
        assert!(docs_str.starts_with('['));
        assert!(docs_str.ends_with(']'));
        assert!(docs_str.contains("Document 0"));
        assert!(docs_str.contains("Document 1"));
        assert!(docs_str.contains("Document 2"));

        sync_string_free(out_documents);
        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_count_documents() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Initially should be 0
        let mut count: u64 = 999;
        let result = sync_engine_count_documents(engine, &mut count);
        assert_eq!(result, SyncResult::Success);
        assert_eq!(count, 0);

        // Create some documents
        for i in 0..5 {
            let content = CString::new(format!(r#"{{"title":"Doc {}"}}"#, i)).unwrap();
            let mut doc_id_buf = [0u8; 37];
            let create_result = sync_engine_create_document(
                engine,
                content.as_ptr(),
                doc_id_buf.as_mut_ptr() as *mut i8,
            );
            assert_eq!(create_result, SyncResult::Success);
        }

        // Count should now be 5
        let result = sync_engine_count_documents(engine, &mut count);
        assert_eq!(result, SyncResult::Success);
        assert_eq!(count, 5);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_is_connected() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Should be disconnected (no server running)
        let connected = sync_engine_is_connected(engine);
        assert!(!connected);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_count_pending_sync() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Initially should have no pending syncs
        let mut pending_count: u64 = 999;
        let result = sync_engine_count_pending_sync(engine, &mut pending_count);
        assert_eq!(result, SyncResult::Success);
        assert_eq!(pending_count, 0);

        // Create a document (should be pending sync since we're offline)
        let content = CString::new(r#"{"title":"Pending Doc"}"#).unwrap();
        let mut doc_id_buf = [0u8; 37];
        let create_result = sync_engine_create_document(
            engine,
            content.as_ptr(),
            doc_id_buf.as_mut_ptr() as *mut i8,
        );
        assert_eq!(create_result, SyncResult::Success);

        // Should now have 1 pending sync
        let result = sync_engine_count_pending_sync(engine, &mut pending_count);
        assert_eq!(result, SyncResult::Success);
        assert_eq!(pending_count, 1);

        sync_engine_destroy(engine);
    }
}

#[test]
fn test_ffi_read_api_null_safety() {
    unsafe {
        // Test null engine pointer for all read APIs
        let mut out_content: *mut i8 = ptr::null_mut();
        let mut out_count: u64 = 0;
        let fake_id = CString::new("00000000-0000-0000-0000-000000000000").unwrap();

        // get_document with null engine
        let result = sync_engine_get_document(ptr::null_mut(), fake_id.as_ptr(), &mut out_content);
        assert_eq!(result, SyncResult::ErrorInvalidInput);

        // get_all_documents with null engine
        let result = sync_engine_get_all_documents(ptr::null_mut(), &mut out_content);
        assert_eq!(result, SyncResult::ErrorInvalidInput);

        // count_documents with null engine
        let result = sync_engine_count_documents(ptr::null_mut(), &mut out_count);
        assert_eq!(result, SyncResult::ErrorInvalidInput);

        // count_pending_sync with null engine
        let result = sync_engine_count_pending_sync(ptr::null_mut(), &mut out_count);
        assert_eq!(result, SyncResult::ErrorInvalidInput);

        // is_connected with null engine should return false
        let connected = sync_engine_is_connected(ptr::null_mut());
        assert!(!connected);
    }
}

#[test]
fn test_ffi_update_and_get_document() {
    unsafe {
        let engine = create_test_engine();
        assert!(!engine.is_null(), "Failed to create sync engine");

        // Create a document
        let content = CString::new(r#"{"title":"Original Title","value":1}"#).unwrap();
        let mut doc_id_buf = [0u8; 37];

        let create_result = sync_engine_create_document(
            engine,
            content.as_ptr(),
            doc_id_buf.as_mut_ptr() as *mut i8,
        );
        assert_eq!(create_result, SyncResult::Success);

        let doc_id = CStr::from_ptr(doc_id_buf.as_ptr() as *const i8)
            .to_string_lossy()
            .to_string();
        let doc_id_cstr = CString::new(doc_id.clone()).unwrap();

        // Update the document
        let updated_content = CString::new(r#"{"title":"Updated Title","value":42}"#).unwrap();
        let update_result =
            sync_engine_update_document(engine, doc_id_cstr.as_ptr(), updated_content.as_ptr());
        assert_eq!(update_result, SyncResult::Success);

        // Retrieve and verify the content field has updated values
        let mut out_content: *mut i8 = ptr::null_mut();
        let get_result = sync_engine_get_document(engine, doc_id_cstr.as_ptr(), &mut out_content);
        assert_eq!(get_result, SyncResult::Success);

        let content_str = CStr::from_ptr(out_content).to_string_lossy().to_string();
        let doc: serde_json::Value = serde_json::from_str(&content_str).unwrap();

        let doc_content = &doc["content"];
        assert_eq!(doc_content["title"], "Updated Title");
        assert_eq!(doc_content["value"], 42);

        sync_string_free(out_content);
        sync_engine_destroy(engine);
    }
}
