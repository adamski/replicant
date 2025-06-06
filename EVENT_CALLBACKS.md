# Event Callback System

The JSON Database Sync Client provides a comprehensive event callback system that allows applications to receive real-time notifications about document changes, sync operations, and connection status updates. The system is designed to be thread-safe, efficient, and easy to use across Rust, C, and C++ applications.

## Overview

### Key Features

- ✅ **Thread-Safe Design**: No locks or mutexes needed in user code
- ✅ **Cross-Language Support**: Works with Rust, C, and C++
- ✅ **Event Filtering**: Subscribe to specific event types or all events
- ✅ **Offline/Online Events**: Receive events for both local and synchronized operations
- ✅ **Simple Integration**: Easy-to-use callback registration and processing
- ✅ **High Performance**: Efficient event queuing with minimal overhead

### How It Works

The event system uses a **single-thread callback model** inspired by RealmCPP:

1. **Events are generated** from any thread (main thread, sync thread, etc.)
2. **Events are queued** for processing without blocking the generating thread
3. **Callbacks are invoked** only on the thread that registered them
4. **User calls `process_events()`** in their main loop to handle queued events

This design eliminates the need for complex thread synchronization in user code while ensuring all callbacks execute safely on a single, predictable thread.

## Event Types

The system supports the following event types:

| Event Type | Description | When Triggered |
|------------|-------------|----------------|
| `DOCUMENT_CREATED` | A new document was created | Local creation or sync from server |
| `DOCUMENT_UPDATED` | An existing document was modified | Local update or sync from server |
| `DOCUMENT_DELETED` | A document was deleted | Local deletion or sync from server |
| `SYNC_STARTED` | Synchronization process began | When connecting to server |
| `SYNC_COMPLETED` | Synchronization finished successfully | After receiving all server changes |
| `SYNC_ERROR` | An error occurred during sync | Network issues, server errors, etc. |
| `CONFLICT_DETECTED` | A conflict was detected between versions | Concurrent modifications detected |
| `CONNECTION_STATE_CHANGED` | Connection status changed | Connected/disconnected from server |

## Rust API

### Basic Usage

```rust
use sync_client::{SyncEngine, EventDispatcher, EventType};
use std::sync::Arc;

// Create sync engine with event support
let engine = SyncEngine::new(
    "sqlite:client.db?mode=rwc",
    "ws://localhost:8080/ws",
    "demo-token"
).await?;

// Get event dispatcher
let events = engine.event_dispatcher();

// Register callback for all events
events.register_callback(
    |event_type, document_id, title, content, error, numeric_data, boolean_data, context| {
        match event_type {
            EventType::DocumentCreated => {
                println!("📄 Document created: {} - {}", document_id.unwrap_or("unknown"), title.unwrap_or("untitled"));
            },
            EventType::DocumentUpdated => {
                println!("✏️ Document updated: {} - {}", document_id.unwrap_or("unknown"), title.unwrap_or("untitled"));
            },
            EventType::SyncCompleted => {
                println!("🔄 Sync completed: {} documents", numeric_data);
            },
            EventType::ConnectionStateChanged => {
                println!("🔗 Connection {}", if boolean_data { "established" } else { "lost" });
            },
            _ => {
                println!("📨 Event: {:?}", event_type);
            }
        }
    },
    std::ptr::null_mut(), // context (optional)
    None // filter (None = all events)
)?;

// In your main loop
loop {
    // Process any pending events
    let processed_count = events.process_events()?;
    if processed_count > 0 {
        println!("Processed {} events", processed_count);
    }
    
    // Do other work...
    std::thread::sleep(std::time::Duration::from_millis(16)); // ~60 FPS
}
```

### Event Filtering

```rust
// Register callback only for document events
events.register_callback(
    |event_type, document_id, title, content, error, numeric_data, boolean_data, context| {
        println!("Document event: {:?} for {}", event_type, document_id.unwrap_or("unknown"));
    },
    std::ptr::null_mut(),
    Some(EventType::DocumentCreated) // Only document creation events
)?;

// Register separate callbacks for different event types
events.register_callback(error_handler, std::ptr::null_mut(), Some(EventType::SyncError))?;
events.register_callback(sync_handler, std::ptr::null_mut(), Some(EventType::SyncCompleted))?;
```

## C API

### Basic Usage

```c
#include "sync_client_events.h"
#include <stdio.h>

// Event callback function
void my_event_callback(const SyncEventData* event, void* context) {
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
            printf("📄 Document created: %s - %s\n", 
                   event->document_id ? event->document_id : "unknown",
                   event->title ? event->title : "untitled");
            break;
            
        case SYNC_EVENT_DOCUMENT_UPDATED:
            printf("✏️ Document updated: %s - %s\n", 
                   event->document_id ? event->document_id : "unknown",
                   event->title ? event->title : "untitled");
            break;
            
        case SYNC_EVENT_SYNC_COMPLETED:
            printf("🔄 Sync completed: %llu documents\n", 
                   (unsigned long long)event->numeric_data);
            break;
            
        case SYNC_EVENT_CONNECTION_STATE_CHANGED:
            printf("🔗 Connection %s\n", 
                   event->boolean_data ? "established" : "lost");
            break;
            
        case SYNC_EVENT_SYNC_ERROR:
            printf("🚨 Sync error: %s\n", 
                   event->error ? event->error : "unknown error");
            break;
            
        default:
            printf("📨 Event type: %d\n", event->event_type);
            break;
    }
}

int main() {
    // Create sync engine
    struct CSyncEngine* engine = sync_engine_create(
        "sqlite:client.db?mode=rwc",
        "ws://localhost:8080/ws",
        "demo-token"
    );
    
    if (!engine) {
        printf("Failed to create sync engine\n");
        return 1;
    }
    
    // Register event callback for all events
    SyncResult result = sync_engine_register_event_callback(
        engine,
        my_event_callback,
        NULL,        // context (optional)
        -1           // event filter (-1 = all events)
    );
    
    if (result != SYNC_RESULT_SUCCESS) {
        printf("Failed to register callback\n");
        sync_engine_destroy(engine);
        return 1;
    }
    
    printf("Event callback registered successfully!\n");
    
    // Main loop
    int running = 1;
    while (running) {
        // Process pending events
        uint32_t processed_count;
        result = sync_engine_process_events(engine, &processed_count);
        
        if (result == SYNC_RESULT_SUCCESS && processed_count > 0) {
            printf("Processed %u events\n", processed_count);
        }
        
        // Do other work...
        usleep(16000); // ~60 FPS
        
        // Example: exit after some condition
        // running = check_exit_condition();
    }
    
    // Clean up
    sync_engine_destroy(engine);
    return 0;
}
```

### Event Filtering

```c
// Register callback only for document creation events
sync_engine_register_event_callback(
    engine,
    my_document_callback,
    NULL,
    SYNC_EVENT_DOCUMENT_CREATED
);

// Register callback only for sync events
sync_engine_register_event_callback(
    engine,
    my_sync_callback,
    NULL,
    SYNC_EVENT_SYNC_COMPLETED
);

// Register callback only for errors
sync_engine_register_event_callback(
    engine,
    my_error_callback,
    NULL,
    SYNC_EVENT_SYNC_ERROR
);
```

### Using Context Data

```c
// Context structure to pass data to callbacks
struct MyAppContext {
    int total_events;
    int document_events;
    char last_error[256];
};

void context_callback(const SyncEventData* event, void* context) {
    struct MyAppContext* app_context = (struct MyAppContext*)context;
    app_context->total_events++;
    
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
        case SYNC_EVENT_DOCUMENT_UPDATED:
        case SYNC_EVENT_DOCUMENT_DELETED:
            app_context->document_events++;
            break;
            
        case SYNC_EVENT_SYNC_ERROR:
            if (event->error) {
                strncpy(app_context->last_error, event->error, sizeof(app_context->last_error) - 1);
                app_context->last_error[sizeof(app_context->last_error) - 1] = '\0';
            }
            break;
    }
}

int main() {
    struct MyAppContext app_context = {0};
    
    // Register callback with context
    sync_engine_register_event_callback(
        engine,
        context_callback,
        &app_context,  // Pass context to callback
        -1
    );
    
    // ... process events ...
    
    printf("Total events: %d, Document events: %d\n", 
           app_context.total_events, app_context.document_events);
}
```

## C++ API

### Simple Usage

```cpp
#include "sync_client_events.h"
#include <iostream>
#include <string>

// Simple RAII wrapper for sync engine
class SyncEngine {
private:
    CSyncEngine* engine_;
    
public:
    SyncEngine(const std::string& database_url, 
               const std::string& server_url, 
               const std::string& auth_token)
    {
        engine_ = sync_engine_create(database_url.c_str(), server_url.c_str(), auth_token.c_str());
        if (!engine_) {
            throw std::runtime_error("Failed to create sync engine");
        }
    }
    
    ~SyncEngine() {
        if (engine_) {
            sync_engine_destroy(engine_);
        }
    }
    
    CSyncEngine* get() const { return engine_; }
    
    SyncResult register_callback(SyncEventCallback callback, void* context = nullptr, int event_filter = -1)
    {
        return sync_engine_register_event_callback(engine_, callback, context, event_filter);
    }
    
    SyncResult process_events(uint32_t* processed_count = nullptr)
    {
        return sync_engine_process_events(engine_, processed_count);
    }
};

// Simple callback function
void simple_callback(const SyncEventData* event, void* context)
{
    switch (event->event_type)
    {
        case SYNC_EVENT_DOCUMENT_CREATED:
            std::cout << "📄 Document created: " 
                     << (event->document_id ? event->document_id : "unknown")
                     << " - " << (event->title ? event->title : "untitled") << "\n";
            break;
            
        case SYNC_EVENT_DOCUMENT_UPDATED:
            std::cout << "✏️ Document updated: " 
                     << (event->document_id ? event->document_id : "unknown")
                     << " - " << (event->title ? event->title : "untitled") << "\n";
            break;
            
        case SYNC_EVENT_SYNC_COMPLETED:
            std::cout << "🔄 Sync completed: " << event->numeric_data << " documents\n";
            break;
            
        case SYNC_EVENT_CONNECTION_STATE_CHANGED:
            std::cout << "🔗 Connection " << (event->boolean_data ? "established" : "lost") << "\n";
            break;
            
        case SYNC_EVENT_SYNC_ERROR:
            std::cout << "🚨 Sync error: " << (event->error ? event->error : "unknown error") << "\n";
            break;
            
        default:
            std::cout << "📨 Event type: " << event->event_type << "\n";
            break;
    }
}

int main()
{
    try 
    {
        // Create sync engine
        SyncEngine engine("sqlite:client.db?mode=rwc", "ws://localhost:8080/ws", "demo-token");
        
        // Register callback
        auto result = engine.register_callback(simple_callback);
        if (result != SYNC_RESULT_SUCCESS)
        {
            std::cerr << "Failed to register callback\n";
            return 1;
        }
        
        std::cout << "Event callback registered successfully!\n";
        
        // Main loop
        bool running = true;
        while (running)
        {
            // Process pending events
            uint32_t processed_count;
            result = engine.process_events(&processed_count);
            
            if (result == SYNC_RESULT_SUCCESS && processed_count > 0)
            {
                std::cout << "Processed " << processed_count << " events\n";
            }
            
            // Do other work...
            std::this_thread::sleep_for(std::chrono::milliseconds(16)); // ~60 FPS
            
            // Example: exit after some condition
            // running = check_exit_condition();
        }
    }
    catch (const std::exception& e)
    {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
    
    return 0;
}
```

### Advanced C++ with Lambda Callbacks

```cpp
#include "sync_client_events.h"
#include <iostream>
#include <functional>
#include <memory>
#include <vector>

// Advanced callback manager using std::function and lambdas
class CallbackManager
{
private:
    struct CallbackInfo
    {
        std::function<void(const SyncEventData*)> handler;
        SyncEventType filter;
        bool filter_enabled;
        
        CallbackInfo(std::function<void(const SyncEventData*)> h, SyncEventType f = SYNC_EVENT_DOCUMENT_CREATED, bool enabled = false)
            : handler(std::move(h)), filter(f), filter_enabled(enabled) {}
    };
    
    std::vector<std::unique_ptr<CallbackInfo>> callbacks_;
    
    // Static C callback that forwards to C++ handlers
    static void static_callback(const SyncEventData* event, void* context)
    {
        auto* manager = static_cast<CallbackManager*>(context);
        
        for (auto& callback_info : manager->callbacks_)
        {
            if (!callback_info->filter_enabled || callback_info->filter == event->event_type)
            {
                callback_info->handler(event);
            }
        }
    }
    
public:
    template<typename Callable>
    void add_callback(Callable&& callback, SyncEventType filter = SYNC_EVENT_DOCUMENT_CREATED, bool enable_filter = false)
    {
        callbacks_.emplace_back(
            std::make_unique<CallbackInfo>(
                std::forward<Callable>(callback), 
                filter, 
                enable_filter
            )
        );
    }
    
    SyncResult register_with(SyncEngine& engine, int event_filter = -1)
    {
        return engine.register_callback(static_callback, this, event_filter);
    }
};

// Event statistics collector
struct EventStats
{
    int total_events = 0;
    int document_events = 0;
    int sync_events = 0;
    std::vector<std::string> recent_events;
    
    void add_event(const std::string& event_name)
    {
        total_events++;
        recent_events.push_back(event_name);
        if (recent_events.size() > 5)
        {
            recent_events.erase(recent_events.begin());
        }
    }
    
    void print_summary() const
    {
        std::cout << "\n=== Event Summary ===\n";
        std::cout << "Total events: " << total_events << "\n";
        std::cout << "Document events: " << document_events << "\n";
        std::cout << "Sync events: " << sync_events << "\n";
        
        if (!recent_events.empty())
        {
            std::cout << "Recent events: ";
            for (const auto& name : recent_events)
            {
                std::cout << name << " ";
            }
            std::cout << "\n";
        }
        std::cout << "====================\n";
    }
};

int main()
{
    try
    {
        SyncEngine engine("sqlite:client.db?mode=rwc", "ws://localhost:8080/ws", "demo-token");
        CallbackManager callbacks;
        EventStats stats;
        
        // Lambda callback for statistics
        callbacks.add_callback([&stats](const SyncEventData* event)
        {
            std::string event_name;
            switch (event->event_type)
            {
                case SYNC_EVENT_DOCUMENT_CREATED: 
                    event_name = "DocumentCreated"; 
                    stats.document_events++;
                    break;
                case SYNC_EVENT_DOCUMENT_UPDATED: 
                    event_name = "DocumentUpdated"; 
                    stats.document_events++;
                    break;
                case SYNC_EVENT_SYNC_COMPLETED: 
                    event_name = "SyncCompleted"; 
                    stats.sync_events++;
                    break;
                default: 
                    event_name = "Other";
                    break;
            }
            
            stats.add_event(event_name);
            std::cout << "📊 Event: " << event_name << "\n";
        });
        
        // Lambda callback for error handling
        callbacks.add_callback([](const SyncEventData* event)
        {
            if (event->event_type == SYNC_EVENT_SYNC_ERROR)
            {
                std::cerr << "🚨 Sync Error: " << (event->error ? event->error : "unknown") << "\n";
            }
            else if (event->event_type == SYNC_EVENT_CONFLICT_DETECTED)
            {
                std::cout << "⚠️ Conflict detected for document: " 
                         << (event->document_id ? event->document_id : "unknown") << "\n";
            }
        });
        
        // Lambda callback for connection monitoring
        callbacks.add_callback([](const SyncEventData* event)
        {
            if (event->event_type == SYNC_EVENT_CONNECTION_STATE_CHANGED)
            {
                std::cout << "🔗 Connection " << (event->boolean_data ? "established" : "lost") << "\n";
            }
        });
        
        // Register all callbacks
        auto result = callbacks.register_with(engine);
        if (result != SYNC_RESULT_SUCCESS)
        {
            throw std::runtime_error("Failed to register callbacks");
        }
        
        std::cout << "Advanced lambda callbacks registered!\n";
        
        // Main loop
        bool running = true;
        while (running)
        {
            uint32_t processed_count;
            engine.process_events(&processed_count);
            
            if (processed_count > 0)
            {
                std::cout << "Processed " << processed_count << " events\n";
            }
            
            std::this_thread::sleep_for(std::chrono::milliseconds(16));
            
            // Example: exit after some condition
            // running = check_exit_condition();
        }
        
        // Print final statistics
        stats.print_summary();
    }
    catch (const std::exception& e)
    {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
    
    return 0;
}
```

## Compilation Examples

### C Example

```bash
# Build the Rust library first
cd sync-workspace
cargo build --release

# Compile C example
gcc -I./sync-client/include \
    -L./target/release \
    -lsync_client \
    -framework CoreFoundation -framework Security \
    -ldl -lpthread -lm \
    your_c_code.c \
    -o your_program
```

### C++ Example

```bash
# Build the Rust library first
cd sync-workspace
cargo build --release

# Compile C++ example
g++ -std=c++14 \
    -I./sync-client/include \
    -L./target/release \
    -lsync_client \
    -framework CoreFoundation -framework Security \
    -ldl -lpthread -lm \
    your_cpp_code.cpp \
    -o your_program
```

## Best Practices

### Thread Safety

✅ **DO**: Call `process_events()` regularly in your main loop  
✅ **DO**: Register all callbacks on the same thread  
✅ **DO**: Use the callback context parameter to pass application state  
❌ **DON'T**: Call callbacks directly from other threads  
❌ **DON'T**: Access shared state from callbacks without proper synchronization  

### Performance

✅ **DO**: Process events at regular intervals (e.g., 60 FPS)  
✅ **DO**: Keep callback functions fast and non-blocking  
✅ **DO**: Use event filtering to reduce unnecessary callback invocations  
❌ **DON'T**: Perform long-running operations in callbacks  
❌ **DON'T**: Block or sleep in callback functions  

### Error Handling

✅ **DO**: Check return values from callback registration  
✅ **DO**: Handle `SYNC_ERROR` events to detect sync issues  
✅ **DO**: Log callback registration failures  
❌ **DON'T**: Ignore callback registration errors  
❌ **DON'T**: Throw exceptions from C callbacks (undefined behavior)  

### Memory Management

✅ **DO**: Ensure context pointers remain valid for the engine's lifetime  
✅ **DO**: Use RAII patterns in C++ for automatic cleanup  
✅ **DO**: Copy event data if you need to store it beyond the callback  
❌ **DON'T**: Store pointers to event data (they become invalid after the callback)  
❌ **DON'T**: Free or modify event data passed to callbacks  

## Working Examples

The repository includes complete working examples:

- **`sync-client/examples/simple_callback_test.c`**: Basic C callback example
- **`sync-client/examples/simple_cpp_callbacks.cpp`**: Simple C++ callback example  
- **`sync-client/examples/cpp_lambda_callbacks.cpp`**: Advanced C++ with lambdas
- **`sync-client/src/events.rs`**: Rust implementation with tests

Build and run these examples to see the callback system in action:

```bash
# Build and test C example
cargo build --release
gcc -I./sync-client/include -L./target/release -lsync_client \
    sync-client/examples/simple_callback_test.c -o c_test
./c_test

# Build and test C++ examples
g++ -std=c++14 -I./sync-client/include -L./target/release -lsync_client \
    sync-client/examples/simple_cpp_callbacks.cpp -o cpp_simple_test
./cpp_simple_test

g++ -std=c++14 -I./sync-client/include -L./target/release -lsync_client \
    sync-client/examples/cpp_lambda_callbacks.cpp -o cpp_lambda_test
./cpp_lambda_test
```

## Troubleshooting

### Common Issues

**Problem**: Callbacks not being invoked  
**Solution**: Ensure you're calling `process_events()` regularly in your main loop

**Problem**: Events missing or delayed  
**Solution**: Check that `process_events()` is called on the same thread that registered callbacks

**Problem**: Compilation errors with C++  
**Solution**: Use C++14 or later (`-std=c++14`) for modern language features

**Problem**: Linking errors  
**Solution**: Ensure all required libraries are linked (see compilation examples above)

**Problem**: Segmentation fault in callbacks  
**Solution**: Verify context pointers are valid and event data is not accessed after callback returns

### Debug Build Features

Build with debug mode to enable additional test functions:

```bash
cargo build  # Debug mode
g++ -DDEBUG -std=c++14 -I./sync-client/include -L./target/debug -lsync_client \
    your_code.cpp -o your_program
```

Debug builds include:
- `sync_engine_emit_test_event()`: Manually trigger test events
- `sync_engine_emit_test_event_burst()`: Generate multiple test events
- Additional logging and validation

These functions are only available in debug builds and should not be used in production code.