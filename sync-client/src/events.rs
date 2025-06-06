use std::ffi::{c_char, c_void, CString, CStr};
use std::sync::Mutex;
use uuid::Uuid;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    DocumentCreated = 0,
    DocumentUpdated = 1,
    DocumentDeleted = 2,
    SyncStarted = 3,
    SyncCompleted = 4,
    SyncError = 5,
    ConflictDetected = 6,
    ConnectionStateChanged = 7,
}

#[repr(C)]
#[derive(Debug)]
pub struct EventData {
    pub event_type: EventType,
    pub document_id: *const c_char,
    pub title: *const c_char,
    pub content: *const c_char,
    pub error: *const c_char,
    pub numeric_data: u64,
    pub boolean_data: bool,
}

pub type EventCallback = extern "C" fn(event: *const EventData, context: *mut c_void);

#[derive(Debug)]
struct CallbackEntry {
    callback: EventCallback,
    context: *mut c_void,
    event_filter: Option<EventType>,
}

unsafe impl Send for CallbackEntry {}
unsafe impl Sync for CallbackEntry {}

pub struct EventDispatcher {
    callbacks: Mutex<Vec<CallbackEntry>>,
    temp_strings: Mutex<Vec<CString>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            callbacks: Mutex::new(Vec::new()),
            temp_strings: Mutex::new(Vec::new()),
        }
    }

    pub fn register_callback(
        &self,
        callback: EventCallback,
        context: *mut c_void,
        event_filter: Option<EventType>,
    ) -> Result<(), &'static str> {
        let mut callbacks = self.callbacks.lock()
            .map_err(|_| "Failed to acquire callback lock")?;
        
        callbacks.push(CallbackEntry {
            callback,
            context,
            event_filter,
        });
        
        Ok(())
    }

    pub fn emit_document_created(&self, document_id: &Uuid, title: &str, content: &serde_json::Value) {
        self.emit_event(EventType::DocumentCreated, Some(document_id), Some(title), Some(content), None, 0, false);
    }

    pub fn emit_document_updated(&self, document_id: &Uuid, title: &str, content: &serde_json::Value) {
        self.emit_event(EventType::DocumentUpdated, Some(document_id), Some(title), Some(content), None, 0, false);
    }

    pub fn emit_document_deleted(&self, document_id: &Uuid) {
        self.emit_event(EventType::DocumentDeleted, Some(document_id), None, None, None, 0, false);
    }

    pub fn emit_sync_started(&self) {
        self.emit_event(EventType::SyncStarted, None, None, None, None, 0, false);
    }

    pub fn emit_sync_completed(&self, synced_count: u64) {
        self.emit_event(EventType::SyncCompleted, None, None, None, None, synced_count, false);
    }

    pub fn emit_sync_error(&self, error_message: &str) {
        self.emit_event(EventType::SyncError, None, None, None, Some(error_message), 0, false);
    }

    pub fn emit_conflict_detected(&self, document_id: &Uuid) {
        self.emit_event(EventType::ConflictDetected, Some(document_id), None, None, None, 0, false);
    }

    pub fn emit_connection_state_changed(&self, connected: bool) {
        self.emit_event(EventType::ConnectionStateChanged, None, None, None, None, 0, connected);
    }

    fn emit_event(
        &self,
        event_type: EventType,
        document_id: Option<&Uuid>,
        title: Option<&str>,
        content: Option<&serde_json::Value>,
        error: Option<&str>,
        numeric_data: u64,
        boolean_data: bool,
    ) {
        let callbacks = match self.callbacks.lock() {
            Ok(callbacks) => callbacks,
            Err(_) => {
                tracing::error!("Failed to acquire callback lock for event emission");
                return;
            }
        };

        if callbacks.is_empty() {
            return;
        }

        let mut temp_strings = match self.temp_strings.lock() {
            Ok(temp_strings) => temp_strings,
            Err(_) => {
                tracing::error!("Failed to acquire temp strings lock for event emission");
                return;
            }
        };

        temp_strings.clear();

        let document_id_cstr = document_id.map(|id| {
            let cstr = CString::new(id.to_string()).unwrap_or_else(|_| CString::new("").unwrap());
            let ptr = cstr.as_ptr();
            temp_strings.push(cstr);
            ptr
        });

        let title_cstr = title.map(|t| {
            let cstr = CString::new(t).unwrap_or_else(|_| CString::new("").unwrap());
            let ptr = cstr.as_ptr();
            temp_strings.push(cstr);
            ptr
        });

        let content_cstr = content.map(|c| {
            let content_str = serde_json::to_string(c).unwrap_or_else(|_| "{}".to_string());
            let cstr = CString::new(content_str).unwrap_or_else(|_| CString::new("{}").unwrap());
            let ptr = cstr.as_ptr();
            temp_strings.push(cstr);
            ptr
        });

        let error_cstr = error.map(|e| {
            let cstr = CString::new(e).unwrap_or_else(|_| CString::new("").unwrap());
            let ptr = cstr.as_ptr();
            temp_strings.push(cstr);
            ptr
        });

        let event_data = EventData {
            event_type,
            document_id: document_id_cstr.unwrap_or(std::ptr::null()),
            title: title_cstr.unwrap_or(std::ptr::null()),
            content: content_cstr.unwrap_or(std::ptr::null()),
            error: error_cstr.unwrap_or(std::ptr::null()),
            numeric_data,
            boolean_data,
        };

        for entry in callbacks.iter() {
            if let Some(filter) = entry.event_filter {
                if filter != event_type {
                    continue;
                }
            }

            (entry.callback)(&event_data, entry.context);
        }
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
        assert!(dispatcher.temp_strings.lock().unwrap().is_empty());
    }

    #[test]
    fn test_callback_registration() {
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
    }

    #[test]
    fn test_event_emission() {
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
        
        let doc_id = Uuid::new_v4();
        dispatcher.emit_document_created(&doc_id, "Test", &serde_json::json!({"test": true}));
        
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
        dispatcher.emit_document_created(&doc_id, "Test", &serde_json::json!({}));
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 0);
        
        // Emit updated event
        dispatcher.emit_document_updated(&doc_id, "Test", &serde_json::json!({}));
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 1);
        
        // Emit deleted event (neither should trigger)
        dispatcher.emit_document_deleted(&doc_id);
        assert_eq!(created_count.load(Ordering::SeqCst), 1);
        assert_eq!(updated_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_event_data_fields() {
        let dispatcher = EventDispatcher::new();
        let captured_data = Arc::new(Mutex::new(None));
        let data_clone = captured_data.clone();
        
        extern "C" fn capture_callback(event: *const EventData, context: *mut c_void) {
            let data = unsafe { &*(context as *const Mutex<Option<(EventType, String, String, String)>>) };
            let event = unsafe { &*event };
            
            let doc_id = if event.document_id.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(event.document_id).to_string_lossy().to_string() }
            };
            
            let title = if event.title.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(event.title).to_string_lossy().to_string() }
            };
            
            let content = if event.content.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(event.content).to_string_lossy().to_string() }
            };
            
            *data.lock().unwrap() = Some((event.event_type, doc_id, title, content));
        }
        
        dispatcher.register_callback(
            capture_callback,
            &*data_clone as *const Mutex<Option<(EventType, String, String, String)>> as *mut c_void,
            None,
        ).unwrap();
        
        let doc_id = Uuid::new_v4();
        let content = serde_json::json!({"key": "value"});
        dispatcher.emit_document_created(&doc_id, "Test Title", &content);
        
        let captured = captured_data.lock().unwrap();
        assert!(captured.is_some());
        let (event_type, cap_id, cap_title, cap_content) = captured.as_ref().unwrap();
        assert_eq!(*event_type, EventType::DocumentCreated);
        assert_eq!(*cap_id, doc_id.to_string());
        assert_eq!(*cap_title, "Test Title");
        assert_eq!(*cap_content, content.to_string());
    }

    #[test]
    fn test_multiple_callbacks() {
        let dispatcher = EventDispatcher::new();
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));
        let count3 = Arc::new(AtomicUsize::new(0));
        
        extern "C" fn increment_callback(_event: *const EventData, context: *mut c_void) {
            let count = unsafe { &*(context as *const AtomicUsize) };
            count.fetch_add(1, Ordering::SeqCst);
        }
        
        // Register three callbacks
        dispatcher.register_callback(
            increment_callback,
            &*count1.clone() as *const AtomicUsize as *mut c_void,
            None,
        ).unwrap();
        
        dispatcher.register_callback(
            increment_callback,
            &*count2.clone() as *const AtomicUsize as *mut c_void,
            None,
        ).unwrap();
        
        dispatcher.register_callback(
            increment_callback,
            &*count3.clone() as *const AtomicUsize as *mut c_void,
            None,
        ).unwrap();
        
        // Emit an event
        dispatcher.emit_sync_started();
        
        // All three should have been called
        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
        assert_eq!(count3.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_all_event_types() {
        let dispatcher = EventDispatcher::new();
        let event_types = Arc::new(Mutex::new(Vec::new()));
        let types_clone = event_types.clone();
        
        extern "C" fn record_event_type(event: *const EventData, context: *mut c_void) {
            let types = unsafe { &*(context as *const Mutex<Vec<EventType>>) };
            let event = unsafe { &*event };
            types.lock().unwrap().push(event.event_type);
        }
        
        dispatcher.register_callback(
            record_event_type,
            &*types_clone as *const Mutex<Vec<EventType>> as *mut c_void,
            None,
        ).unwrap();
        
        let doc_id = Uuid::new_v4();
        
        // Emit all event types
        dispatcher.emit_document_created(&doc_id, "Test", &serde_json::json!({}));
        dispatcher.emit_document_updated(&doc_id, "Test", &serde_json::json!({}));
        dispatcher.emit_document_deleted(&doc_id);
        dispatcher.emit_sync_started();
        dispatcher.emit_sync_completed(10);
        dispatcher.emit_sync_error("Test error");
        dispatcher.emit_conflict_detected(&doc_id);
        dispatcher.emit_connection_state_changed(true);
        
        let types = event_types.lock().unwrap();
        assert_eq!(types.len(), 8);
        assert_eq!(types[0], EventType::DocumentCreated);
        assert_eq!(types[1], EventType::DocumentUpdated);
        assert_eq!(types[2], EventType::DocumentDeleted);
        assert_eq!(types[3], EventType::SyncStarted);
        assert_eq!(types[4], EventType::SyncCompleted);
        assert_eq!(types[5], EventType::SyncError);
        assert_eq!(types[6], EventType::ConflictDetected);
        assert_eq!(types[7], EventType::ConnectionStateChanged);
    }

    #[test]
    fn test_numeric_and_boolean_data() {
        let dispatcher = EventDispatcher::new();
        let captured_data = Arc::new(Mutex::new((0u64, false)));
        let data_clone = captured_data.clone();
        
        extern "C" fn capture_numeric_bool(event: *const EventData, context: *mut c_void) {
            let data = unsafe { &*(context as *const Mutex<(u64, bool)>) };
            let event = unsafe { &*event };
            *data.lock().unwrap() = (event.numeric_data, event.boolean_data);
        }
        
        dispatcher.register_callback(
            capture_numeric_bool,
            &*data_clone as *const Mutex<(u64, bool)> as *mut c_void,
            None,
        ).unwrap();
        
        // Test sync completed with count
        dispatcher.emit_sync_completed(42);
        assert_eq!(captured_data.lock().unwrap().0, 42);
        
        // Test connection state changed
        dispatcher.emit_connection_state_changed(true);
        assert_eq!(captured_data.lock().unwrap().1, true);
        
        dispatcher.emit_connection_state_changed(false);
        assert_eq!(captured_data.lock().unwrap().1, false);
    }

    #[test]
    fn test_error_message() {
        let dispatcher = EventDispatcher::new();
        let error_msg = Arc::new(Mutex::new(String::new()));
        let msg_clone = error_msg.clone();
        
        extern "C" fn capture_error(event: *const EventData, context: *mut c_void) {
            let msg = unsafe { &*(context as *const Mutex<String>) };
            let event = unsafe { &*event };
            
            if !event.error.is_null() {
                let error_str = unsafe { CStr::from_ptr(event.error).to_string_lossy().to_string() };
                *msg.lock().unwrap() = error_str;
            }
        }
        
        dispatcher.register_callback(
            capture_error,
            &*msg_clone as *const Mutex<String> as *mut c_void,
            Some(EventType::SyncError),
        ).unwrap();
        
        dispatcher.emit_sync_error("Network connection failed");
        assert_eq!(*error_msg.lock().unwrap(), "Network connection failed");
    }
}