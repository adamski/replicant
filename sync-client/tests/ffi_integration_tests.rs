//! Integration tests for FFI event callbacks
//! 
//! These tests verify that the FFI interface works correctly and that
//! callbacks are properly invoked when events occur.

use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::ptr;

use sync_client::ffi::{SyncEngine, SyncResult, sync_engine_create, sync_engine_destroy,
                      sync_engine_register_event_callback, sync_engine_create_document};
use sync_client::events::{EventData, EventType};

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
        let doc_id = unsafe { CStr::from_ptr(event.document_id).to_string_lossy().to_string() };
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

fn create_test_engine() -> *mut SyncEngine {
    let db_url = CString::new("sqlite::memory:").unwrap();
    let server_url = CString::new("ws://localhost:8080/ws").unwrap();
    let auth_token = CString::new("test-token").unwrap();
    let user_identifier = CString::new("test-user@example.com").unwrap();

    sync_engine_create(
        db_url.as_ptr(),
        server_url.as_ptr(),
        auth_token.as_ptr(),
        user_identifier.as_ptr(),
    )
}

#[test]
fn test_ffi_callback_registration() {
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

#[test]
fn test_ffi_callback_with_document_creation() {
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
    let title = CString::new("Test Document").unwrap();
    let content = CString::new(r#"{"content":"test data","type":"note"}"#).unwrap();
    let mut doc_id = [0u8; 37]; // UUID string + null terminator
    
    let create_result = sync_engine_create_document(
        engine,
        title.as_ptr(),
        content.as_ptr(),
        doc_id.as_mut_ptr() as *mut i8,
    );
    
    assert_eq!(create_result, SyncResult::Success);
    
    // Give a moment for callback to be invoked
    std::thread::sleep(std::time::Duration::from_millis(10));
    
    // Verify callback was called
    assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(*capture.last_event_type.lock().unwrap(), Some(EventType::DocumentCreated));
    
    let captured_title = capture.last_title.lock().unwrap();
    assert_eq!(captured_title.as_ref().unwrap(), "Test Document");
    
    let captured_content = capture.last_content.lock().unwrap();
    assert!(captured_content.is_some());
    assert!(captured_content.as_ref().unwrap().contains("test data"));
    
    sync_engine_destroy(engine);
}

#[test]
fn test_ffi_callback_filtering() {
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
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 0);
        
        sync_engine_emit_test_event(engine, 1); // DocumentUpdated
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 1);
        
        sync_engine_emit_test_event(engine, 2); // DocumentDeleted (neither should trigger)
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        assert_eq!(created_capture.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_capture.call_count.load(Ordering::SeqCst), 1);
    }
    
    sync_engine_destroy(engine);
}

#[test]
fn test_ffi_multiple_callbacks() {
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
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        // All three should have been called
        assert_eq!(capture1.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(capture2.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(capture3.call_count.load(Ordering::SeqCst), 1);
        
        assert_eq!(*capture1.last_event_type.lock().unwrap(), Some(EventType::SyncStarted));
        assert_eq!(*capture2.last_event_type.lock().unwrap(), Some(EventType::SyncStarted));
        assert_eq!(*capture3.last_event_type.lock().unwrap(), Some(EventType::SyncStarted));
    }
    
    sync_engine_destroy(engine);
}

#[test]
#[cfg(debug_assertions)]
fn test_ffi_all_event_types() {
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
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        assert_eq!(capture.call_count.load(Ordering::SeqCst), 1, 
                   "Event type {} was not emitted", event_type);
        
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
        
        assert_eq!(*capture.last_event_type.lock().unwrap(), Some(expected_type));
    }
    
    sync_engine_destroy(engine);
}

#[test]
#[cfg(debug_assertions)]
fn test_ffi_event_data_integrity() {
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
    std::thread::sleep(std::time::Duration::from_millis(10));
    
    assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(*capture.last_event_type.lock().unwrap(), Some(EventType::SyncCompleted));
    assert_eq!(*capture.last_numeric_data.lock().unwrap(), 5); // Test uses fixed value of 5
    
    // Test sync error event (has error message)
    capture.reset();
    sync_engine_emit_test_event(engine, 5); // SyncError
    std::thread::sleep(std::time::Duration::from_millis(10));
    
    assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(*capture.last_event_type.lock().unwrap(), Some(EventType::SyncError));
    let error_msg = capture.last_error.lock().unwrap();
    assert!(error_msg.is_some());
    assert!(error_msg.as_ref().unwrap().contains("Test error message"));
    
    // Test connection state changed (has boolean data)
    capture.reset();
    sync_engine_emit_test_event(engine, 7); // ConnectionLost
    std::thread::sleep(std::time::Duration::from_millis(10));
    
    assert_eq!(capture.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(*capture.last_event_type.lock().unwrap(), Some(EventType::ConnectionLost));
    assert_eq!(*capture.last_boolean_data.lock().unwrap(), true); // Test uses fixed value of true
    
    sync_engine_destroy(engine);
}

#[test]
#[cfg(debug_assertions)]
fn test_ffi_event_burst() {
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
    std::thread::sleep(std::time::Duration::from_millis(50)); // More time for burst
    
    // Should have received all 5 events
    assert_eq!(capture.call_count.load(Ordering::SeqCst), 5);
    assert_eq!(*capture.last_event_type.lock().unwrap(), Some(EventType::DocumentCreated));
    
    // The last event should have sequence 4 in the title
    let last_title = capture.last_title.lock().unwrap();
    assert!(last_title.is_some());
    assert!(last_title.as_ref().unwrap().contains("4")); // Sequence is 0-indexed
    
    sync_engine_destroy(engine);
}

#[test]
fn test_ffi_null_pointer_safety() {
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

#[test]
fn test_ffi_engine_lifecycle() {
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

#[test]
fn test_ffi_callback_thread_safety() {
    use std::thread;
    use std::time::Duration;
    
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
        
        let handles: Vec<_> = (0..3).map(|i| {
            let capture = capture.clone();
            thread::spawn(move || {
                let engine = engine_ptr as *mut SyncEngine;
                for _ in 0..2 {
                    sync_engine_emit_test_event(engine, 3); // SyncStarted
                    thread::sleep(Duration::from_millis(1));
                }
            })
        }).collect();
        
        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Give time for all callbacks to complete
        thread::sleep(Duration::from_millis(50));
        
        // Should have received 6 events total (3 threads × 2 events each)
        assert_eq!(capture.call_count.load(Ordering::SeqCst), 6);
    }
    
    sync_engine_destroy(engine);
}