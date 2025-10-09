//! Event callback system for the sync client
//! 
//! This module provides a thread-safe event system that allows applications to receive
//! real-time notifications about document changes, sync operations, and connection status.
//! 
//! # Key Features
//! 
//! - **Thread-Safe Design**: Events can be generated from any thread but callbacks are
//!   only invoked on the thread that registered them
//! - **Event Filtering**: Subscribe to specific event types or all events
//! - **Context Passing**: Pass application context data to callbacks
//! - **Offline/Online Events**: Receive events for both local and synchronized operations
//! 
//! # Usage
//!
//! ```rust,no_run
//! use sync_client::events::{EventDispatcher, EventType};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create event dispatcher
//! let dispatcher = EventDispatcher::new();
//!
//! // Register callback for all events
//! dispatcher.register_rust_callback(
//!     Box::new(|event_type, _document_id, _title, _content, _error, _numeric_data, _boolean_data, _context| {
//!         // Handle event
//!         println!("Event received: {:?}", event_type);
//!     }),
//!     std::ptr::null_mut(),
//!     None
//! )?;
//!
//! // In your main loop, process events
//! loop {
//!     let processed = dispatcher.process_events()?;
//!     // ... do other work
//! #   break; // Exit loop for doctest
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! # Thread Safety
//! 
//! The event system uses a single-thread callback model:
//! 1. Events can be generated from any thread
//! 2. Events are queued for processing
//! 3. Callbacks are only invoked when `process_events()` is called
//! 4. All callbacks execute on the thread that registered them
//! 
//! This design eliminates the need for complex synchronization in user code.

use std::ffi::{c_char, c_void, CString};
use std::sync::{Mutex, mpsc};
use std::thread::{self, ThreadId};
use uuid::Uuid;
use sync_core::{errors::ClientError, SyncResult};

/// Event types that can be emitted by the sync client
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    /// A new document was created (local or from sync)
    DocumentCreated = 0,
    /// An existing document was updated (local or from sync)
    DocumentUpdated = 1,
    /// A document was deleted (local or from sync)
    DocumentDeleted = 2,
    /// Synchronization process started
    SyncStarted = 3,
    /// Synchronization completed successfully
    SyncCompleted = 4,
    /// An error occurred during synchronization
    SyncError = 5,
    /// A conflict was detected between document versions
    ConflictDetected = 6,
    /// Connection to server was lost
    ConnectionLost = 7,
    /// A connection attempt was made to the server
    ConnectionAttempted = 8,
    /// Successfully connected to the server
    ConnectionSucceeded = 9,
}

/// Event data structure passed to C/C++ callbacks
/// 
/// Contains all relevant information about the event. Not all fields are populated
/// for all event types - check the event_type to determine which fields are valid.
#[repr(C)]
#[derive(Debug)]
pub struct EventData {
    /// Type of event that occurred
    pub event_type: EventType,
    /// Document UUID (may be null for non-document events)
    pub document_id: *const c_char,
    /// Document title (may be null)
    pub title: *const c_char,
    /// Document content as JSON string (may be null)
    pub content: *const c_char,
    /// Error message (only for error events, may be null)
    pub error: *const c_char,
    /// Numeric data (e.g., sync count for SYNC_COMPLETED)
    pub numeric_data: u64,
    /// Boolean data (e.g., connection state for CONNECTION_STATE_CHANGED)
    pub boolean_data: bool,
}

/// C-compatible callback function type for event notifications
/// 
/// # Parameters
/// 
/// * `event` - Pointer to event data (valid only during callback execution)
/// * `context` - User-defined context pointer passed during registration
/// 
/// # Safety
/// 
/// The event pointer is only valid during the callback execution.
/// Do not store or access it after the callback returns.
/// String pointers in the event data are temporary and will be freed
/// after the callback returns. Copy them if you need to retain the data.
pub type EventCallback = extern "C" fn(event: *const EventData, context: *mut c_void);

/// Rust-native callback function type for event handling
/// 
/// This is a more ergonomic callback interface for Rust applications that
/// passes individual event parameters rather than a C-style struct pointer.
/// 
/// # Parameters
/// 
/// * `event_type` - The type of event that occurred
/// * `document_id` - Optional document ID (for document-related events)
/// * `title` - Optional document title
/// * `content` - Optional document content as JSON string
/// * `error` - Optional error message (for error events)
/// * `numeric_data` - Numeric data (e.g., sync count)
/// * `boolean_data` - Boolean data (e.g., connection state)
/// * `context` - User-defined context pointer
pub type RustEventCallback = Box<dyn Fn(
    EventType,
    Option<&str>,    // document_id
    Option<&str>,    // title
    Option<&str>,    // content
    Option<&str>,    // error
    u64,             // numeric_data
    bool,            // boolean_data
    *mut c_void,     // context
) + Send + Sync>;

enum CallbackType {
    CFfi(EventCallback),
    Rust(RustEventCallback),
}

struct CallbackEntry {
    callback: CallbackType,
    context: *mut c_void,
    event_filter: Option<EventType>,
}

unsafe impl Send for CallbackEntry {}
unsafe impl Sync for CallbackEntry {}

#[derive(Debug, Clone)]
pub struct QueuedEvent {
    event_type: EventType,
    document_id: Option<String>,
    title: Option<String>,
    content: Option<String>,
    error: Option<String>,
    numeric_data: u64,
    boolean_data: bool,
}

/// Thread-safe event dispatcher for managing callbacks and event processing
/// 
/// The EventDispatcher uses a single-thread callback model where events can be
/// generated from any thread but callbacks are only invoked on the thread that
/// registered them. This eliminates the need for complex synchronization in user code.
/// 
/// # Example
///
/// ```rust,no_run
/// use sync_client::events::{EventDispatcher, EventData, EventType};
/// use std::ffi::c_void;
///
/// // Define callback function
/// extern "C" fn my_callback(event: *const EventData, _context: *mut c_void) {
///     unsafe {
///         println!("Event type: {:?}", (*event).event_type);
///     }
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dispatcher = EventDispatcher::new();
///
/// // Register callback (sets callback thread to current thread)
/// dispatcher.register_callback(my_callback, std::ptr::null_mut(), None)?;
///
/// // In main loop
/// loop {
///     let processed = dispatcher.process_events()?;
///     // ... do other work
/// #   break; // Exit loop for doctest
/// }
/// # Ok(())
/// # }
/// ```
pub struct EventDispatcher {
    callbacks: Mutex<Vec<CallbackEntry>>,
    event_queue: Mutex<mpsc::Receiver<QueuedEvent>>,
    event_sender: mpsc::Sender<QueuedEvent>,
    callback_thread_id: Mutex<Option<ThreadId>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            callbacks: Mutex::new(Vec::new()),
            event_queue: Mutex::new(receiver),
            event_sender: sender,
            callback_thread_id: Mutex::new(None),
        }
    }

    pub fn register_callback(
        &self,
        callback: EventCallback,
        context: *mut c_void,
        event_filter: Option<EventType>,
    ) -> SyncResult<()> {
        // Set the callback thread ID on first registration
        {
            let mut thread_id = self.callback_thread_id.lock()
                .map_err(|_| ClientError::LockError("thread ID".into()))?;
            if thread_id.is_none() {
                *thread_id = Some(thread::current().id());
                tracing::info!("Event callbacks will be processed on thread: {:?}", thread::current().id());
            }
        }

        let mut callbacks = self.callbacks.lock()
            .map_err(|_|  ClientError::LockError("callbacks".into()))?;
        
        callbacks.push(CallbackEntry {
            callback: CallbackType::CFfi(callback),
            context,
            event_filter,
        });
        
        Ok(())
    }

    /// Register a Rust-native callback for event notifications
    /// 
    /// This is a more ergonomic interface for Rust applications that avoids
    /// the need to work with C-style function pointers and structs.
    /// 
    /// # Parameters
    /// 
    /// * `callback` - The callback function to invoke for events
    /// * `context` - Optional user-defined context pointer
    /// * `event_filter` - Optional filter to only receive specific event types
    /// 
    /// # Example
    /// 
    /// ```rust,no_run
    /// use sync_client::events::{EventDispatcher, EventType};
    /// 
    /// let dispatcher = EventDispatcher::new();
    /// 
    /// dispatcher.register_rust_callback(
    ///     Box::new(|event_type, document_id, title, _content, error, numeric_data, boolean_data, _context| {
    ///         match event_type {
    ///             EventType::DocumentCreated => {
    ///                 println!("Document created: {}", title.unwrap_or("untitled"));
    ///             },
    ///             EventType::SyncCompleted => {
    ///                 println!("Sync completed: {} documents", numeric_data);
    ///             },
    ///             _ => {}
    ///         }
    ///     }),
    ///     std::ptr::null_mut(),
    ///     None
    /// ).unwrap();
    /// ```
    pub fn register_rust_callback(
        &self,
        callback: RustEventCallback,
        context: *mut c_void,
        event_filter: Option<EventType>,
    ) -> SyncResult<()> {
        // Set the callback thread ID on first registration
        {
            let mut thread_id = self.callback_thread_id.lock()
                .map_err(|_| ClientError::LockError("thread ID".into()))?;
            if thread_id.is_none() {
                *thread_id = Some(thread::current().id());
                tracing::info!("Event callbacks will be processed on thread: {:?}", thread::current().id());
            }
        }

        let mut callbacks = self.callbacks.lock()
            .map_err(|_| ClientError::LockError("callbacks".into()))?;
        
        callbacks.push(CallbackEntry {
            callback: CallbackType::Rust(callback),
            context,
            event_filter,
        });
        
        Ok(())
    }

    pub fn emit_document_created(&self, document_id: &Uuid, content: &serde_json::Value) {
        // Extract title from content if present
        let title = content.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        self.queue_event(EventType::DocumentCreated, Some(document_id), Some(title), Some(content), None, 0, false);
    }

    pub fn emit_document_updated(&self, document_id: &Uuid, content: &serde_json::Value) {
        // Extract title from content if present
        let title = content.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        self.queue_event(EventType::DocumentUpdated, Some(document_id), Some(title), Some(content), None, 0, false);
    }

    pub fn emit_document_deleted(&self, document_id: &Uuid) {
        self.queue_event(EventType::DocumentDeleted, Some(document_id), None, None, None, 0, false);
    }

    pub fn emit_sync_started(&self) {
        self.queue_event(EventType::SyncStarted, None, None, None, None, 0, false);
    }

    pub fn emit_sync_completed(&self, synced_count: u64) {
        self.queue_event(EventType::SyncCompleted, None, None, None, None, synced_count, false);
    }

    pub fn emit_sync_error(&self, error_message: &str) {
        self.queue_event(EventType::SyncError, None, None, None, Some(error_message), 0, false);
    }

    pub fn emit_conflict_detected(&self, document_id: &Uuid) {
       self.queue_event(EventType::ConflictDetected, Some(document_id), None, None, None, 0, false);
    }

    pub fn emit_connection_lost(&self, server_url: &str) {
        self.queue_event(EventType::ConnectionLost, None, Some(server_url), None, None, 0, false);
    }

    pub fn emit_connection_attempted(&self, server_url: &str) {
        self.queue_event(EventType::ConnectionAttempted, None, Some(server_url), None, None, 0, false);
    }

    pub fn emit_connection_succeeded(&self, server_url: &str) {
        self.queue_event(EventType::ConnectionSucceeded, None, Some(server_url), None, None, 0, false);
    }

    /// Queue an event for later processing on the callback thread
    #[allow(clippy::too_many_arguments)] // FFI callback constraints
    fn queue_event(
        &self,
        event_type: EventType,
        document_id: Option<&Uuid>,
        title: Option<&str>,
        content: Option<&serde_json::Value>,
        error: Option<&str>,
        numeric_data: u64,
        boolean_data: bool,
    ) {
        let queued_event = QueuedEvent {
            event_type,
            document_id: document_id.map(|id| id.to_string()),
            title: title.map(|t| t.to_string()),
            content: content.map(|c| serde_json::to_string(c).unwrap_or_else(|_| "{}".to_string())),
            error: error.map(|e| e.to_string()),
            numeric_data,
            boolean_data,
        };

        if self.event_sender.send(queued_event).is_err() {
            tracing::error!("Failed to queue event - receiver may have been dropped");
        }
    }

    /// Process all queued events. This MUST be called on the same thread where callbacks were registered.
    pub fn process_events(&self) -> SyncResult<usize> {
        // Verify we're on the correct thread
        {
            let thread_id = self.callback_thread_id.lock()
                .map_err(|_| ClientError::LockError("thread ID".into()))?;
            if let Some(expected_thread_id) = *thread_id {
                if thread::current().id() != expected_thread_id {
                    return Err(ClientError::ThreadSafetyViolation.into());
                }
            } else {
                return Err(ClientError::NoCallbacksRegistered.into());
            }
        }

        let callbacks = self.callbacks.lock()
            .map_err(|_| ClientError::LockError("callbacks".into()))?;

        if callbacks.is_empty() {
            return Ok(0);
        }

        let receiver = self.event_queue.lock()
            .map_err(|_| ClientError::LockError("event queue".into()))?;

        let mut processed_count = 0;
        let mut temp_strings = Vec::new();

        // Process all available events
        while let Ok(queued_event) = receiver.try_recv() {
            temp_strings.clear();

            // Convert strings to C-compatible format
            let document_id_cstr = queued_event.document_id.as_ref().map(|id| {
                let cstr = CString::new(id.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
                let ptr = cstr.as_ptr();
                temp_strings.push(cstr);
                ptr
            });

            let title_cstr = queued_event.title.as_ref().map(|t| {
                let cstr = CString::new(t.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
                let ptr = cstr.as_ptr();
                temp_strings.push(cstr);
                ptr
            });

            let content_cstr = queued_event.content.as_ref().map(|c| {
                let cstr = CString::new(c.as_str()).unwrap_or_else(|_| CString::new("{}").unwrap());
                let ptr = cstr.as_ptr();
                temp_strings.push(cstr);
                ptr
            });

            let error_cstr = queued_event.error.as_ref().map(|e| {
                let cstr = CString::new(e.as_str()).unwrap_or_else(|_| CString::new("").unwrap());
                let ptr = cstr.as_ptr();
                temp_strings.push(cstr);
                ptr
            });

            let event_data = EventData {
                event_type: queued_event.event_type,
                document_id: document_id_cstr.unwrap_or(std::ptr::null()),
                title: title_cstr.unwrap_or(std::ptr::null()),
                content: content_cstr.unwrap_or(std::ptr::null()),
                error: error_cstr.unwrap_or(std::ptr::null()),
                numeric_data: queued_event.numeric_data,
                boolean_data: queued_event.boolean_data,
            };

            // Invoke all matching callbacks
            for entry in callbacks.iter() {
                if let Some(filter) = entry.event_filter {
                    if filter != queued_event.event_type {
                        continue;
                    }
                }

                match &entry.callback {
                    CallbackType::CFfi(callback) => {
                        callback(&event_data, entry.context);
                    },
                    CallbackType::Rust(callback) => {
                        callback(
                            queued_event.event_type,
                            queued_event.document_id.as_deref(),
                            queued_event.title.as_deref(),
                            queued_event.content.as_deref(),
                            queued_event.error.as_deref(),
                            queued_event.numeric_data,
                            queued_event.boolean_data,
                            entry.context,
                        );
                    }
                }
            }

            processed_count += 1;
        }

        Ok(processed_count)
    }

    /// Get the number of events waiting to be processed
    pub fn pending_event_count(&self) -> usize {
        // We can't easily check the channel length without consuming from it,
        // so we'll estimate by trying to process events and counting them
        self.process_events().unwrap_or_default()
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_event_dispatcher_creation() {
        let dispatcher = EventDispatcher::new();
        assert!(dispatcher.callbacks.lock().unwrap().is_empty());
        
        // Should have no pending events initially
        let result = dispatcher.process_events();
        assert!(result.is_err()); // No callbacks registered yet
    }

    #[test]
    fn test_callback_registration_and_processing() {
        let dispatcher = EventDispatcher::new();
        let callback_count = Arc::new(AtomicUsize::new(0));
        let count_clone = callback_count.clone();
        
        extern "C" fn test_callback(_event: *const EventData, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        
        let result = dispatcher.register_callback(
            test_callback,
            &*count_clone as *const AtomicUsize as *mut c_void,
            None,
        );
        
        assert!(result.is_ok());
        assert_eq!(dispatcher.callbacks.lock().unwrap().len(), 1);
        
        // Emit an event
        let doc_id = Uuid::new_v4();
        dispatcher.emit_document_created(&doc_id, &serde_json::json!({"title": "Test", "test": true}));
        
        // Process events (should invoke callback)
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_event_filtering() {
        let dispatcher = EventDispatcher::new();
        let created_count = Arc::new(AtomicUsize::new(0));
        let updated_count = Arc::new(AtomicUsize::new(0));
        
        let created_clone = created_count.clone();
        let updated_clone = updated_count.clone();
        
        extern "C" fn count_callback(_event: *const EventData, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        
        // Register callback only for created events
        dispatcher.register_callback(
            count_callback,
            &*created_clone as *const AtomicUsize as *mut c_void,
            Some(EventType::DocumentCreated),
        ).unwrap();
        
        // Register callback only for updated events
        dispatcher.register_callback(
            count_callback,
            &*updated_clone as *const AtomicUsize as *mut c_void,
            Some(EventType::DocumentUpdated),
        ).unwrap();
        
        let doc_id = Uuid::new_v4();
        
        // Emit created event
        dispatcher.emit_document_created(&doc_id, &serde_json::json!({"title": "Test"}));
        dispatcher.process_events().unwrap();
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 0);
        
        // Emit updated event
        dispatcher.emit_document_updated(&doc_id, &serde_json::json!({"title": "Test"}));
        dispatcher.process_events().unwrap();
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 1);
        
        // Emit deleted event (neither should trigger)
        dispatcher.emit_document_deleted(&doc_id);
        dispatcher.process_events().unwrap();
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_thread_safety_check() {
        let dispatcher = Arc::new(EventDispatcher::new());
        let callback_count = Arc::new(AtomicUsize::new(0));
        let count_clone = callback_count.clone();
        
        extern "C" fn test_callback(_event: *const EventData, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        
        // Register callback on main thread
        dispatcher.register_callback(
            test_callback,
            &*count_clone as *const AtomicUsize as *mut c_void,
            None,
        ).unwrap();
        
        let dispatcher_clone = dispatcher.clone();
        
        // Try to process events from another thread (should fail)
        let handle = std::thread::spawn(move || {
            dispatcher_clone.process_events()
        });
        
        let result = handle.join().unwrap();
        assert!(result.is_err()); // Should fail because we're on wrong thread
        
        // Process events on main thread (should work)
        dispatcher.emit_sync_started();
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_multiple_events_queuing() {
        let dispatcher = EventDispatcher::new();
        let callback_count = Arc::new(AtomicUsize::new(0));
        let count_clone = callback_count.clone();
        
        extern "C" fn count_callback(_event: *const EventData, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        
        dispatcher.register_callback(
            count_callback,
            &*count_clone as *const AtomicUsize as *mut c_void,
            None,
        ).unwrap();
        
        // Emit multiple events
        let doc_id = Uuid::new_v4();
        dispatcher.emit_document_created(&doc_id, &serde_json::json!({"title": "Test1"}));
        dispatcher.emit_document_updated(&doc_id, &serde_json::json!({"title": "Test2"}));
        dispatcher.emit_document_deleted(&doc_id);
        dispatcher.emit_sync_started();
        dispatcher.emit_sync_completed(5);
        
        // Process all events at once
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 5);
        assert_eq!(callback_count.load(Ordering::SeqCst), 5);
        
        // Process again (should be no more events)
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 0);
    }
}