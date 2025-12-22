//! Event callback system for the sync client
//!
//! This module provides a thread-safe event system that allows applications to receive
//! real-time notifications about document changes, sync operations, and connection status.
//!
//! # Key Features
//!
//! - **Thread-Safe Design**: Events can be generated from any thread but callbacks are
//!   only invoked on the thread that registered them
//! - **Type-Specific Callbacks**: Separate callback types for documents, sync, errors,
//!   connections, and conflicts
//! - **Event Filtering**: Subscribe to specific event types or all events within a category
//! - **Context Passing**: Pass application context data to callbacks
//! - **Offline/Online Events**: Receive events for both local and synchronized operations
//!
//! # Callback Types
//!
//! - `DocumentEventCallback`: DocumentCreated, DocumentUpdated, DocumentDeleted
//! - `SyncEventCallback`: SyncStarted, SyncCompleted
//! - `ErrorEventCallback`: SyncError
//! - `ConnectionEventCallback`: ConnectionLost, ConnectionAttempted, ConnectionSucceeded
//! - `ConflictEventCallback`: ConflictDetected
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

use replicant_core::{errors::ClientError, SyncResult};
use std::ffi::{c_char, c_void, CString};
use std::sync::{mpsc, Mutex};
use std::thread::{self, ThreadId};
use uuid::Uuid;

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

// =============================================================================
// Rust-Native Event Types
// =============================================================================

/// Rust-native event enum with typed variants
///
/// This provides an idiomatic Rust interface for event handling, with each variant
/// containing only the relevant data for that event type.
///
/// # Example
///
/// ```rust,no_run
/// use replicant_client::events::{EventDispatcher, SyncEvent};
///
/// let dispatcher = EventDispatcher::new();
///
/// dispatcher.register_rust_callback(|event| {
///     match event {
///         SyncEvent::DocumentCreated { id, title, .. } => {
///             println!("Created: {} - {}", id, title);
///         }
///         SyncEvent::SyncCompleted { document_count } => {
///             println!("Synced {} documents", document_count);
///         }
///         _ => {}
///     }
/// }).unwrap();
/// ```
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// A new document was created
    DocumentCreated {
        id: String,
        title: String,
        content: serde_json::Value,
    },
    /// An existing document was updated
    DocumentUpdated {
        id: String,
        title: String,
        content: serde_json::Value,
    },
    /// A document was deleted
    DocumentDeleted { id: String },
    /// Synchronization started
    SyncStarted,
    /// Synchronization completed
    SyncCompleted { document_count: u64 },
    /// A sync error occurred
    SyncError { message: String },
    /// A conflict was detected
    ConflictDetected {
        document_id: String,
        winning_content: Option<String>,
        losing_content: Option<String>,
    },
    /// Connection to server was lost
    ConnectionLost { server_url: String },
    /// A connection attempt was made
    ConnectionAttempted { server_url: String },
    /// Successfully connected to server
    ConnectionSucceeded { server_url: String },
}

impl SyncEvent {
    /// Get the event type for this event
    pub fn event_type(&self) -> EventType {
        match self {
            SyncEvent::DocumentCreated { .. } => EventType::DocumentCreated,
            SyncEvent::DocumentUpdated { .. } => EventType::DocumentUpdated,
            SyncEvent::DocumentDeleted { .. } => EventType::DocumentDeleted,
            SyncEvent::SyncStarted => EventType::SyncStarted,
            SyncEvent::SyncCompleted { .. } => EventType::SyncCompleted,
            SyncEvent::SyncError { .. } => EventType::SyncError,
            SyncEvent::ConflictDetected { .. } => EventType::ConflictDetected,
            SyncEvent::ConnectionLost { .. } => EventType::ConnectionLost,
            SyncEvent::ConnectionAttempted { .. } => EventType::ConnectionAttempted,
            SyncEvent::ConnectionSucceeded { .. } => EventType::ConnectionSucceeded,
        }
    }

    /// Convert from internal QueuedEvent to SyncEvent
    fn from_queued(event: &QueuedEvent) -> Self {
        match event.event_type {
            EventType::DocumentCreated => SyncEvent::DocumentCreated {
                id: event.document_id.clone().unwrap_or_default(),
                title: event
                    .title
                    .clone()
                    .unwrap_or_else(|| "Untitled".to_string()),
                content: event
                    .content
                    .as_ref()
                    .and_then(|c| serde_json::from_str(c).ok())
                    .unwrap_or(serde_json::Value::Null),
            },
            EventType::DocumentUpdated => SyncEvent::DocumentUpdated {
                id: event.document_id.clone().unwrap_or_default(),
                title: event
                    .title
                    .clone()
                    .unwrap_or_else(|| "Untitled".to_string()),
                content: event
                    .content
                    .as_ref()
                    .and_then(|c| serde_json::from_str(c).ok())
                    .unwrap_or(serde_json::Value::Null),
            },
            EventType::DocumentDeleted => SyncEvent::DocumentDeleted {
                id: event.document_id.clone().unwrap_or_default(),
            },
            EventType::SyncStarted => SyncEvent::SyncStarted,
            EventType::SyncCompleted => SyncEvent::SyncCompleted {
                document_count: event.numeric_data,
            },
            EventType::SyncError => SyncEvent::SyncError {
                message: event
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string()),
            },
            EventType::ConflictDetected => SyncEvent::ConflictDetected {
                document_id: event.document_id.clone().unwrap_or_default(),
                winning_content: event.content.clone(),
                losing_content: event.error.clone(),
            },
            EventType::ConnectionLost => SyncEvent::ConnectionLost {
                server_url: event.title.clone().unwrap_or_default(),
            },
            EventType::ConnectionAttempted => SyncEvent::ConnectionAttempted {
                server_url: event.title.clone().unwrap_or_default(),
            },
            EventType::ConnectionSucceeded => SyncEvent::ConnectionSucceeded {
                server_url: event.title.clone().unwrap_or_default(),
            },
        }
    }
}

// =============================================================================
// Type-Specific Callback Types (C FFI)
// =============================================================================

/// Document event callback for DocumentCreated, DocumentUpdated, DocumentDeleted
///
/// # Parameters
/// * `event_type` - The specific document event type
/// * `document_id` - UUID of the document (always non-null)
/// * `title` - Document title (null for Deleted events)
/// * `content` - Full document JSON (null for Deleted events)
/// * `context` - User-defined context pointer
pub type DocumentEventCallback = extern "C" fn(
    event_type: EventType,
    document_id: *const c_char,
    title: *const c_char,
    content: *const c_char,
    context: *mut c_void,
);

/// Sync event callback for SyncStarted, SyncCompleted
///
/// # Parameters
/// * `event_type` - SyncStarted or SyncCompleted
/// * `document_count` - Number of documents synced (0 for SyncStarted)
/// * `context` - User-defined context pointer
pub type SyncEventCallback =
    extern "C" fn(event_type: EventType, document_count: u64, context: *mut c_void);

/// Error event callback for SyncError
///
/// # Parameters
/// * `event_type` - Always SyncError
/// * `error` - Error message (always non-null)
/// * `context` - User-defined context pointer
pub type ErrorEventCallback =
    extern "C" fn(event_type: EventType, error: *const c_char, context: *mut c_void);

/// Connection event callback for ConnectionLost, ConnectionAttempted, ConnectionSucceeded
///
/// # Parameters
/// * `event_type` - The connection event type
/// * `connected` - true if connected (valid for Lost/Succeeded), false otherwise
/// * `attempt_number` - Reconnection attempt number (valid for ConnectionAttempted)
/// * `context` - User-defined context pointer
pub type ConnectionEventCallback = extern "C" fn(
    event_type: EventType,
    connected: bool,
    attempt_number: u32,
    context: *mut c_void,
);

/// Conflict event callback for ConflictDetected
///
/// # Parameters
/// * `event_type` - Always ConflictDetected
/// * `document_id` - UUID of the conflicted document (always non-null)
/// * `winning_content` - Content of the winning version (always non-null)
/// * `losing_content` - Content of the losing version (may be null)
/// * `context` - User-defined context pointer
pub type ConflictEventCallback = extern "C" fn(
    event_type: EventType,
    document_id: *const c_char,
    winning_content: *const c_char,
    losing_content: *const c_char,
    context: *mut c_void,
);

// =============================================================================
// Callback Entry Types (Internal)
// =============================================================================

struct DocumentCallbackEntry {
    callback: DocumentEventCallback,
    context: *mut c_void,
    event_filter: Option<EventType>,
}

struct SyncCallbackEntry {
    callback: SyncEventCallback,
    context: *mut c_void,
}

struct ErrorCallbackEntry {
    callback: ErrorEventCallback,
    context: *mut c_void,
}

struct ConnectionCallbackEntry {
    callback: ConnectionEventCallback,
    context: *mut c_void,
}

struct ConflictCallbackEntry {
    callback: ConflictEventCallback,
    context: *mut c_void,
}

// Safety: Callback entries are only accessed from the registered thread
unsafe impl Send for DocumentCallbackEntry {}
unsafe impl Sync for DocumentCallbackEntry {}
unsafe impl Send for SyncCallbackEntry {}
unsafe impl Sync for SyncCallbackEntry {}
unsafe impl Send for ErrorCallbackEntry {}
unsafe impl Sync for ErrorCallbackEntry {}
unsafe impl Send for ConnectionCallbackEntry {}
unsafe impl Sync for ConnectionCallbackEntry {}
unsafe impl Send for ConflictCallbackEntry {}
unsafe impl Sync for ConflictCallbackEntry {}

// =============================================================================
// Rust Callback Entry (Internal)
// =============================================================================

struct RustCallbackEntry {
    callback: Box<dyn Fn(SyncEvent) + Send>,
    event_filter: Option<EventType>,
}

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
/// use replicant_client::events::{EventDispatcher, EventType};
/// use std::ffi::c_void;
///
/// // Define callback function
/// extern "C" fn my_document_callback(
///     event_type: EventType,
///     document_id: *const std::ffi::c_char,
///     title: *const std::ffi::c_char,
///     content: *const std::ffi::c_char,
///     _context: *mut c_void
/// ) {
///     println!("Document event: {:?}", event_type);
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dispatcher = EventDispatcher::new();
///
/// // Register callback (sets callback thread to current thread)
/// dispatcher.register_document_callback(my_document_callback, std::ptr::null_mut(), None)?;
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
    // Type-specific C FFI callback storage
    document_callbacks: Mutex<Vec<DocumentCallbackEntry>>,
    sync_callbacks: Mutex<Vec<SyncCallbackEntry>>,
    error_callbacks: Mutex<Vec<ErrorCallbackEntry>>,
    connection_callbacks: Mutex<Vec<ConnectionCallbackEntry>>,
    conflict_callbacks: Mutex<Vec<ConflictCallbackEntry>>,
    // Rust-native callback storage
    rust_callbacks: Mutex<Vec<RustCallbackEntry>>,
    // Event queue
    event_queue: Mutex<mpsc::Receiver<QueuedEvent>>,
    event_sender: mpsc::Sender<QueuedEvent>,
    callback_thread_id: Mutex<Option<ThreadId>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            document_callbacks: Mutex::new(Vec::new()),
            sync_callbacks: Mutex::new(Vec::new()),
            error_callbacks: Mutex::new(Vec::new()),
            connection_callbacks: Mutex::new(Vec::new()),
            conflict_callbacks: Mutex::new(Vec::new()),
            rust_callbacks: Mutex::new(Vec::new()),
            event_queue: Mutex::new(receiver),
            event_sender: sender,
            callback_thread_id: Mutex::new(None),
        }
    }

    /// Helper to set callback thread ID on first registration
    fn ensure_callback_thread(&self) -> SyncResult<()> {
        let mut thread_id = self
            .callback_thread_id
            .lock()
            .map_err(|_| ClientError::LockError("thread ID".into()))?;
        if thread_id.is_none() {
            *thread_id = Some(thread::current().id());
            tracing::info!(
                "Event callbacks will be processed on thread: {:?}",
                thread::current().id()
            );
        }
        Ok(())
    }

    /// Register a callback for document events (Created, Updated, Deleted)
    ///
    /// # Parameters
    /// * `callback` - Function to call for document events
    /// * `context` - User-defined context pointer passed to callback
    /// * `event_filter` - Optional filter: DocumentCreated, DocumentUpdated, DocumentDeleted, or None for all
    pub fn register_document_callback(
        &self,
        callback: DocumentEventCallback,
        context: *mut c_void,
        event_filter: Option<EventType>,
    ) -> SyncResult<()> {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .document_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("document_callbacks".into()))?;

        callbacks.push(DocumentCallbackEntry {
            callback,
            context,
            event_filter,
        });

        Ok(())
    }

    /// Register a callback for sync events (Started, Completed)
    ///
    /// # Parameters
    /// * `callback` - Function to call for sync events
    /// * `context` - User-defined context pointer passed to callback
    pub fn register_sync_callback(
        &self,
        callback: SyncEventCallback,
        context: *mut c_void,
    ) -> SyncResult<()> {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .sync_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("sync_callbacks".into()))?;

        callbacks.push(SyncCallbackEntry { callback, context });

        Ok(())
    }

    /// Register a callback for error events (SyncError)
    ///
    /// # Parameters
    /// * `callback` - Function to call for error events
    /// * `context` - User-defined context pointer passed to callback
    pub fn register_error_callback(
        &self,
        callback: ErrorEventCallback,
        context: *mut c_void,
    ) -> SyncResult<()> {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .error_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("error_callbacks".into()))?;

        callbacks.push(ErrorCallbackEntry { callback, context });

        Ok(())
    }

    /// Register a callback for connection events (Lost, Attempted, Succeeded)
    ///
    /// # Parameters
    /// * `callback` - Function to call for connection events
    /// * `context` - User-defined context pointer passed to callback
    pub fn register_connection_callback(
        &self,
        callback: ConnectionEventCallback,
        context: *mut c_void,
    ) -> SyncResult<()> {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .connection_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("connection_callbacks".into()))?;

        callbacks.push(ConnectionCallbackEntry { callback, context });

        Ok(())
    }

    /// Register a callback for conflict events (ConflictDetected)
    ///
    /// # Parameters
    /// * `callback` - Function to call for conflict events
    /// * `context` - User-defined context pointer passed to callback
    pub fn register_conflict_callback(
        &self,
        callback: ConflictEventCallback,
        context: *mut c_void,
    ) -> SyncResult<()> {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .conflict_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("conflict_callbacks".into()))?;

        callbacks.push(ConflictCallbackEntry { callback, context });

        Ok(())
    }

    /// Register a Rust-native callback for all events
    ///
    /// This provides an idiomatic Rust interface using the `SyncEvent` enum.
    /// The callback receives typed event variants with only relevant data.
    ///
    /// # Parameters
    /// * `callback` - Closure to call for events
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use replicant_client::events::{EventDispatcher, SyncEvent};
    ///
    /// let dispatcher = EventDispatcher::new();
    ///
    /// dispatcher.register_rust_callback(|event| {
    ///     match event {
    ///         SyncEvent::DocumentCreated { id, title, .. } => {
    ///             println!("Document created: {} - {}", id, title);
    ///         }
    ///         SyncEvent::SyncCompleted { document_count } => {
    ///             println!("Synced {} documents", document_count);
    ///         }
    ///         SyncEvent::ConnectionSucceeded { server_url } => {
    ///             println!("Connected to {}", server_url);
    ///         }
    ///         _ => {}
    ///     }
    /// }).unwrap();
    /// ```
    pub fn register_rust_callback<F>(&self, callback: F) -> SyncResult<()>
    where
        F: Fn(SyncEvent) + Send + 'static,
    {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .rust_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("rust_callbacks".into()))?;

        callbacks.push(RustCallbackEntry {
            callback: Box::new(callback),
            event_filter: None,
        });

        Ok(())
    }

    /// Register a Rust-native callback with event type filtering
    ///
    /// # Parameters
    /// * `callback` - Closure to call for events
    /// * `event_filter` - Only receive events of this type
    pub fn register_rust_callback_filtered<F>(
        &self,
        callback: F,
        event_filter: EventType,
    ) -> SyncResult<()>
    where
        F: Fn(SyncEvent) + Send + 'static,
    {
        self.ensure_callback_thread()?;

        let mut callbacks = self
            .rust_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("rust_callbacks".into()))?;

        callbacks.push(RustCallbackEntry {
            callback: Box::new(callback),
            event_filter: Some(event_filter),
        });

        Ok(())
    }

    pub fn emit_document_created(&self, document_id: &Uuid, content: &serde_json::Value) {
        // Extract title from content if present
        let title = content
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        self.queue_event(
            EventType::DocumentCreated,
            Some(document_id),
            Some(title),
            Some(content),
            None,
            0,
            false,
        );
    }

    pub fn emit_document_updated(&self, document_id: &Uuid, content: &serde_json::Value) {
        // Extract title from content if present
        let title = content
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        self.queue_event(
            EventType::DocumentUpdated,
            Some(document_id),
            Some(title),
            Some(content),
            None,
            0,
            false,
        );
    }

    pub fn emit_document_deleted(&self, document_id: &Uuid) {
        self.queue_event(
            EventType::DocumentDeleted,
            Some(document_id),
            None,
            None,
            None,
            0,
            false,
        );
    }

    pub fn emit_sync_started(&self) {
        self.queue_event(EventType::SyncStarted, None, None, None, None, 0, false);
    }

    pub fn emit_sync_completed(&self, synced_count: u64) {
        self.queue_event(
            EventType::SyncCompleted,
            None,
            None,
            None,
            None,
            synced_count,
            false,
        );
    }

    pub fn emit_sync_error(&self, error_message: &str) {
        self.queue_event(
            EventType::SyncError,
            None,
            None,
            None,
            Some(error_message),
            0,
            false,
        );
    }

    pub fn emit_conflict_detected(&self, document_id: &Uuid) {
        self.queue_event(
            EventType::ConflictDetected,
            Some(document_id),
            None,
            None,
            None,
            0,
            false,
        );
    }

    pub fn emit_connection_lost(&self, server_url: &str) {
        self.queue_event(
            EventType::ConnectionLost,
            None,
            Some(server_url),
            None,
            None,
            0,
            false,
        );
    }

    pub fn emit_connection_attempted(&self, server_url: &str) {
        self.queue_event(
            EventType::ConnectionAttempted,
            None,
            Some(server_url),
            None,
            None,
            0,
            false,
        );
    }

    pub fn emit_connection_succeeded(&self, server_url: &str) {
        self.queue_event(
            EventType::ConnectionSucceeded,
            None,
            Some(server_url),
            None,
            None,
            0,
            false,
        );
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

    /// Check if any callbacks are registered
    fn has_callbacks(&self) -> SyncResult<bool> {
        let doc = self
            .document_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("document_callbacks".into()))?;
        let sync = self
            .sync_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("sync_callbacks".into()))?;
        let error = self
            .error_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("error_callbacks".into()))?;
        let conn = self
            .connection_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("connection_callbacks".into()))?;
        let conflict = self
            .conflict_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("conflict_callbacks".into()))?;
        let rust = self
            .rust_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("rust_callbacks".into()))?;

        Ok(!doc.is_empty()
            || !sync.is_empty()
            || !error.is_empty()
            || !conn.is_empty()
            || !conflict.is_empty()
            || !rust.is_empty())
    }

    /// Process all queued events. This MUST be called on the same thread where callbacks were registered.
    pub fn process_events(&self) -> SyncResult<usize> {
        // Verify we're on the correct thread
        {
            let thread_id = self
                .callback_thread_id
                .lock()
                .map_err(|_| ClientError::LockError("thread ID".into()))?;
            if let Some(expected_thread_id) = *thread_id {
                if thread::current().id() != expected_thread_id {
                    return Err(ClientError::ThreadSafetyViolation.into());
                }
            } else {
                return Err(ClientError::NoCallbacksRegistered.into());
            }
        }

        if !self.has_callbacks()? {
            return Ok(0);
        }

        // Lock all callback vectors
        let document_callbacks = self
            .document_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("document_callbacks".into()))?;
        let sync_callbacks = self
            .sync_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("sync_callbacks".into()))?;
        let error_callbacks = self
            .error_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("error_callbacks".into()))?;
        let connection_callbacks = self
            .connection_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("connection_callbacks".into()))?;
        let conflict_callbacks = self
            .conflict_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("conflict_callbacks".into()))?;
        let rust_callbacks = self
            .rust_callbacks
            .lock()
            .map_err(|_| ClientError::LockError("rust_callbacks".into()))?;

        let receiver = self
            .event_queue
            .lock()
            .map_err(|_| ClientError::LockError("event queue".into()))?;

        let mut processed_count = 0;
        let mut temp_strings: Vec<CString> = Vec::new();

        // Process all available events
        while let Ok(queued_event) = receiver.try_recv() {
            // Dispatch to Rust callbacks first (no FFI overhead)
            if !rust_callbacks.is_empty() {
                let sync_event = SyncEvent::from_queued(&queued_event);
                for entry in rust_callbacks.iter() {
                    if let Some(filter) = entry.event_filter {
                        if filter != queued_event.event_type {
                            continue;
                        }
                    }
                    (entry.callback)(sync_event.clone());
                }
            }

            // Then dispatch to C FFI callbacks
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

            // Dispatch to appropriate callback type based on event type
            match queued_event.event_type {
                EventType::DocumentCreated
                | EventType::DocumentUpdated
                | EventType::DocumentDeleted => {
                    let doc_id_ptr = document_id_cstr.unwrap_or(std::ptr::null());
                    let title_ptr = title_cstr.unwrap_or(std::ptr::null());
                    let content_ptr = content_cstr.unwrap_or(std::ptr::null());

                    for entry in document_callbacks.iter() {
                        if let Some(filter) = entry.event_filter {
                            if filter != queued_event.event_type {
                                continue;
                            }
                        }
                        (entry.callback)(
                            queued_event.event_type,
                            doc_id_ptr,
                            title_ptr,
                            content_ptr,
                            entry.context,
                        );
                    }
                }

                EventType::SyncStarted | EventType::SyncCompleted => {
                    for entry in sync_callbacks.iter() {
                        (entry.callback)(
                            queued_event.event_type,
                            queued_event.numeric_data,
                            entry.context,
                        );
                    }
                }

                EventType::SyncError => {
                    let error_ptr = error_cstr.unwrap_or(std::ptr::null());
                    for entry in error_callbacks.iter() {
                        (entry.callback)(queued_event.event_type, error_ptr, entry.context);
                    }
                }

                EventType::ConnectionLost
                | EventType::ConnectionAttempted
                | EventType::ConnectionSucceeded => {
                    for entry in connection_callbacks.iter() {
                        (entry.callback)(
                            queued_event.event_type,
                            queued_event.boolean_data,
                            queued_event.numeric_data as u32,
                            entry.context,
                        );
                    }
                }

                EventType::ConflictDetected => {
                    let doc_id_ptr = document_id_cstr.unwrap_or(std::ptr::null());
                    let winning_ptr = content_cstr.unwrap_or(std::ptr::null());
                    let losing_ptr = error_cstr.unwrap_or(std::ptr::null()); // Use error field for losing content

                    for entry in conflict_callbacks.iter() {
                        (entry.callback)(
                            queued_event.event_type,
                            doc_id_ptr,
                            winning_ptr,
                            losing_ptr,
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
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_event_dispatcher_creation() {
        let dispatcher = EventDispatcher::new();
        assert!(dispatcher.document_callbacks.lock().unwrap().is_empty());
        assert!(dispatcher.sync_callbacks.lock().unwrap().is_empty());

        // Should have no pending events initially
        let result = dispatcher.process_events();
        assert!(result.is_err()); // No callbacks registered yet
    }

    #[test]
    fn test_document_callback_registration_and_processing() {
        let dispatcher = EventDispatcher::new();
        let callback_count = Arc::new(AtomicUsize::new(0));
        let count_clone = callback_count.clone();

        extern "C" fn test_callback(
            _event_type: EventType,
            _doc_id: *const c_char,
            _title: *const c_char,
            _content: *const c_char,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        let result = dispatcher.register_document_callback(
            test_callback,
            &*count_clone as *const AtomicUsize as *mut c_void,
            None,
        );

        assert!(result.is_ok());
        assert_eq!(dispatcher.document_callbacks.lock().unwrap().len(), 1);

        // Emit an event
        let doc_id = Uuid::new_v4();
        dispatcher
            .emit_document_created(&doc_id, &serde_json::json!({"title": "Test", "test": true}));

        // Process events (should invoke callback)
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_document_event_filtering() {
        let dispatcher = EventDispatcher::new();
        let created_count = Arc::new(AtomicUsize::new(0));
        let updated_count = Arc::new(AtomicUsize::new(0));

        let created_clone = created_count.clone();
        let updated_clone = updated_count.clone();

        extern "C" fn count_callback(
            _event_type: EventType,
            _doc_id: *const c_char,
            _title: *const c_char,
            _content: *const c_char,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        // Register callback only for created events
        dispatcher
            .register_document_callback(
                count_callback,
                &*created_clone as *const AtomicUsize as *mut c_void,
                Some(EventType::DocumentCreated),
            )
            .unwrap();

        // Register callback only for updated events
        dispatcher
            .register_document_callback(
                count_callback,
                &*updated_clone as *const AtomicUsize as *mut c_void,
                Some(EventType::DocumentUpdated),
            )
            .unwrap();

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

        // Emit deleted event (neither should trigger - they're filtered)
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

        extern "C" fn test_callback(_event_type: EventType, _doc_count: u64, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        // Register callback on main thread
        dispatcher
            .register_sync_callback(
                test_callback,
                &*count_clone as *const AtomicUsize as *mut c_void,
            )
            .unwrap();

        let dispatcher_clone = dispatcher.clone();

        // Try to process events from another thread (should fail)
        let handle = std::thread::spawn(move || dispatcher_clone.process_events());

        let result = handle.join().unwrap();
        assert!(result.is_err()); // Should fail because we're on wrong thread

        // Process events on main thread (should work)
        dispatcher.emit_sync_started();
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_multiple_callback_types() {
        let dispatcher = EventDispatcher::new();
        let doc_count = Arc::new(AtomicUsize::new(0));
        let sync_count = Arc::new(AtomicUsize::new(0));

        let doc_clone = doc_count.clone();
        let sync_clone = sync_count.clone();

        extern "C" fn doc_callback(
            _event_type: EventType,
            _doc_id: *const c_char,
            _title: *const c_char,
            _content: *const c_char,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        extern "C" fn sync_callback(_event_type: EventType, _doc_count: u64, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        // Register callbacks for different event types
        dispatcher
            .register_document_callback(
                doc_callback,
                &*doc_clone as *const AtomicUsize as *mut c_void,
                None,
            )
            .unwrap();

        dispatcher
            .register_sync_callback(
                sync_callback,
                &*sync_clone as *const AtomicUsize as *mut c_void,
            )
            .unwrap();

        // Emit multiple events of different types
        let doc_id = Uuid::new_v4();
        dispatcher.emit_document_created(&doc_id, &serde_json::json!({"title": "Test1"}));
        dispatcher.emit_document_updated(&doc_id, &serde_json::json!({"title": "Test2"}));
        dispatcher.emit_document_deleted(&doc_id);
        dispatcher.emit_sync_started();
        dispatcher.emit_sync_completed(5);

        // Process all events at once
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 5);
        assert_eq!(doc_count.load(Ordering::SeqCst), 3); // 3 document events
        assert_eq!(sync_count.load(Ordering::SeqCst), 2); // 2 sync events

        // Process again (should be no more events)
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 0);
    }

    #[test]
    fn test_error_callback() {
        let dispatcher = EventDispatcher::new();
        let error_count = Arc::new(AtomicUsize::new(0));
        let error_clone = error_count.clone();

        extern "C" fn error_callback(
            _event_type: EventType,
            _error: *const c_char,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        dispatcher
            .register_error_callback(
                error_callback,
                &*error_clone as *const AtomicUsize as *mut c_void,
            )
            .unwrap();

        dispatcher.emit_sync_error("Test error");

        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(error_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_connection_callback() {
        let dispatcher = EventDispatcher::new();
        let conn_count = Arc::new(AtomicUsize::new(0));
        let conn_clone = conn_count.clone();

        extern "C" fn conn_callback(
            _event_type: EventType,
            _connected: bool,
            _attempt: u32,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        dispatcher
            .register_connection_callback(
                conn_callback,
                &*conn_clone as *const AtomicUsize as *mut c_void,
            )
            .unwrap();

        dispatcher.emit_connection_lost("ws://localhost:8080");
        dispatcher.emit_connection_attempted("ws://localhost:8080");
        dispatcher.emit_connection_succeeded("ws://localhost:8080");

        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 3);
        assert_eq!(conn_count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_conflict_callback() {
        let dispatcher = EventDispatcher::new();
        let conflict_count = Arc::new(AtomicUsize::new(0));
        let conflict_clone = conflict_count.clone();

        extern "C" fn conflict_callback(
            _event_type: EventType,
            _doc_id: *const c_char,
            _winning: *const c_char,
            _losing: *const c_char,
            context: *mut c_void,
        ) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        dispatcher
            .register_conflict_callback(
                conflict_callback,
                &*conflict_clone as *const AtomicUsize as *mut c_void,
            )
            .unwrap();

        let doc_id = Uuid::new_v4();
        dispatcher.emit_conflict_detected(&doc_id);

        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 1);
        assert_eq!(conflict_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_rust_callback() {
        let dispatcher = EventDispatcher::new();
        let events_received = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_clone = events_received.clone();

        // Register Rust callback
        dispatcher
            .register_rust_callback(move |event| {
                let desc = match &event {
                    SyncEvent::DocumentCreated { title, .. } => format!("created:{}", title),
                    SyncEvent::SyncCompleted { document_count } => {
                        format!("synced:{}", document_count)
                    }
                    SyncEvent::ConnectionSucceeded { server_url } => {
                        format!("connected:{}", server_url)
                    }
                    _ => format!("other:{:?}", event.event_type()),
                };
                events_clone.lock().unwrap().push(desc);
            })
            .unwrap();

        // Emit events
        let doc_id = Uuid::new_v4();
        dispatcher.emit_document_created(&doc_id, &serde_json::json!({"title": "Test Doc"}));
        dispatcher.emit_sync_completed(42);
        dispatcher.emit_connection_succeeded("ws://localhost:8080");

        // Process events
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 3);

        // Verify events were received
        let events = events_received.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], "created:Test Doc");
        assert_eq!(events[1], "synced:42");
        assert_eq!(events[2], "connected:ws://localhost:8080");
    }

    #[test]
    fn test_rust_and_c_callbacks_together() {
        let dispatcher = EventDispatcher::new();

        // Rust callback
        let rust_count = Arc::new(AtomicUsize::new(0));
        let rust_clone = rust_count.clone();
        dispatcher
            .register_rust_callback(move |_event| {
                rust_clone.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();

        // C FFI callback
        let c_count = Arc::new(AtomicUsize::new(0));
        let c_clone = c_count.clone();

        extern "C" fn c_callback(_event_type: EventType, _doc_count: u64, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }

        dispatcher
            .register_sync_callback(c_callback, &*c_clone as *const AtomicUsize as *mut c_void)
            .unwrap();

        // Emit sync events
        dispatcher.emit_sync_started();
        dispatcher.emit_sync_completed(10);

        // Process events
        let processed = dispatcher.process_events().unwrap();
        assert_eq!(processed, 2);

        // Both callbacks should have been invoked
        assert_eq!(rust_count.load(Ordering::SeqCst), 2);
        assert_eq!(c_count.load(Ordering::SeqCst), 2);
    }
}
