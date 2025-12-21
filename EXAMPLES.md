# Interactive Examples

This project includes interactive examples that demonstrate the sync system in action.

## Event Callback Examples

The sync client provides real-time event callbacks for document changes, sync operations, and connection status. These examples show practical usage patterns.

### Real-Time UI Updates

Monitor document changes to update your user interface in real-time:

```c
// C example for UI updates
struct AppContext {
    int total_documents;
    char status_message[256];
};

void ui_update_callback(const SyncEventData* event, void* context) {
    struct AppContext* app = (struct AppContext*)context;
    
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
            app->total_documents++;
            snprintf(app->status_message, sizeof(app->status_message), 
                    "Document created: %s", event->title);
            update_ui_status(app->status_message);
            refresh_document_list();
            break;
            
        case SYNC_EVENT_SYNC_COMPLETED:
            snprintf(app->status_message, sizeof(app->status_message),
                    "Sync complete: %llu documents updated", event->numeric_data);
            update_ui_status(app->status_message);
            break;
            
        case SYNC_EVENT_CONNECTION_STATE_CHANGED:
            if (event->boolean_data) {
                strcpy(app->status_message, "✅ Connected to server");
                enable_sync_features();
            } else {
                strcpy(app->status_message, "❌ Disconnected - working offline");
                disable_sync_features();
            }
            update_ui_status(app->status_message);
            break;
    }
}

// In your application initialization
struct AppContext app_context = {0};
sync_engine_register_event_callback(engine, ui_update_callback, &app_context, -1);
```

### Sync Progress Monitoring

Track synchronization progress and handle errors:

```cpp
// C++ example for sync monitoring
class SyncMonitor {
private:
    std::chrono::steady_clock::time_point sync_start_time;
    int sync_progress = 0;
    
public:
    void handle_event(const SyncEventData* event) {
        switch (event->event_type) {
            case SYNC_EVENT_SYNC_STARTED:
                sync_start_time = std::chrono::steady_clock::now();
                sync_progress = 0;
                std::cout << "🔄 Starting sync..." << std::endl;
                show_progress_bar(true);
                break;
                
            case SYNC_EVENT_DOCUMENT_UPDATED:
                sync_progress++;
                update_progress_bar(sync_progress);
                std::cout << "📄 Synced: " << event->title << std::endl;
                break;
                
            case SYNC_EVENT_SYNC_COMPLETED: {
                auto duration = std::chrono::steady_clock::now() - sync_start_time;
                auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(duration).count();
                
                std::cout << "✅ Sync completed in " << ms << "ms" << std::endl;
                std::cout << "📊 Total documents: " << event->numeric_data << std::endl;
                hide_progress_bar();
                break;
            }
            
            case SYNC_EVENT_SYNC_ERROR:
                std::cerr << "🚨 Sync error: " << event->error << std::endl;
                hide_progress_bar();
                show_error_dialog(event->error);
                break;
        }
    }
};

// Register with lambda
SyncMonitor monitor;
callbacks.add_callback([&monitor](const SyncEventData* event) {
    monitor.handle_event(event);
});
```

### Document Activity Logging

Log all document operations for audit trails:

```rust
// Rust example for activity logging
use std::fs::OpenOptions;
use std::io::Write;
use chrono::Utc;

fn setup_activity_logging(events: &EventDispatcher) -> Result<(), Box<dyn std::error::Error>> {
    events.register_callback(
        |event_type, document_id, title, content, error, numeric_data, boolean_data, context| {
            let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
            let log_entry = match event_type {
                EventType::DocumentCreated => {
                    format!("[{}] CREATED: {} - {}", 
                           timestamp, 
                           document_id.unwrap_or("unknown"), 
                           title.unwrap_or("untitled"))
                },
                EventType::DocumentUpdated => {
                    format!("[{}] UPDATED: {} - {}", 
                           timestamp, 
                           document_id.unwrap_or("unknown"), 
                           title.unwrap_or("untitled"))
                },
                EventType::DocumentDeleted => {
                    format!("[{}] DELETED: {}", 
                           timestamp, 
                           document_id.unwrap_or("unknown"))
                },
                EventType::SyncCompleted => {
                    format!("[{}] SYNC: Completed {} documents", timestamp, numeric_data)
                },
                EventType::SyncError => {
                    format!("[{}] ERROR: {}", timestamp, error.unwrap_or("unknown error"))
                },
                _ => return, // Skip other events
            };
            
            // Write to log file
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open("sync_activity.log") 
            {
                writeln!(file, "{}", log_entry).ok();
            }
            
            // Also print to console
            println!("{}", log_entry);
        },
        std::ptr::null_mut(),
        None // Log all events
    )?;
    
    Ok(())
}
```

### Conflict Resolution UI

Handle conflicts with user interaction:

```c
// C example for conflict resolution
void conflict_handler(const SyncEventData* event, void* context) {
    if (event->event_type == SYNC_EVENT_CONFLICT_DETECTED) {
        printf("⚠️ Conflict detected for document: %s\n", event->document_id);
        
        // Show conflict resolution dialog
        ConflictResolution resolution = show_conflict_dialog(
            event->document_id,
            "Local version conflicts with server version"
        );
        
        switch (resolution) {
            case KEEP_LOCAL:
                printf("User chose to keep local version\n");
                // Force push local version
                break;
            case ACCEPT_SERVER:
                printf("User chose to accept server version\n");
                // Reload from server
                break;
            case MERGE_MANUALLY:
                printf("User chose manual merge\n");
                // Open merge editor
                open_merge_editor(event->document_id);
                break;
        }
    }
}
```

### Performance Metrics

Monitor sync performance and statistics:

```cpp
// C++ example for performance monitoring
class PerformanceMonitor {
private:
    struct {
        int documents_created = 0;
        int documents_updated = 0;
        int documents_deleted = 0;
        int sync_operations = 0;
        int sync_errors = 0;
        std::chrono::milliseconds total_sync_time{0};
    } stats;
    
    std::chrono::steady_clock::time_point sync_start;
    
public:
    void handle_event(const SyncEventData* event) {
        switch (event->event_type) {
            case SYNC_EVENT_DOCUMENT_CREATED:
                stats.documents_created++;
                break;
            case SYNC_EVENT_DOCUMENT_UPDATED:
                stats.documents_updated++;
                break;
            case SYNC_EVENT_DOCUMENT_DELETED:
                stats.documents_deleted++;
                break;
            case SYNC_EVENT_SYNC_STARTED:
                sync_start = std::chrono::steady_clock::now();
                break;
            case SYNC_EVENT_SYNC_COMPLETED:
                stats.sync_operations++;
                stats.total_sync_time += std::chrono::duration_cast<std::chrono::milliseconds>(
                    std::chrono::steady_clock::now() - sync_start
                );
                break;
            case SYNC_EVENT_SYNC_ERROR:
                stats.sync_errors++;
                break;
        }
    }
    
    void print_stats() const {
        std::cout << "\n📊 Performance Statistics:" << std::endl;
        std::cout << "Documents created: " << stats.documents_created << std::endl;
        std::cout << "Documents updated: " << stats.documents_updated << std::endl;
        std::cout << "Documents deleted: " << stats.documents_deleted << std::endl;
        std::cout << "Sync operations: " << stats.sync_operations << std::endl;
        std::cout << "Sync errors: " << stats.sync_errors << std::endl;
        if (stats.sync_operations > 0) {
            auto avg_time = stats.total_sync_time.count() / stats.sync_operations;
            std::cout << "Average sync time: " << avg_time << "ms" << std::endl;
        }
    }
};
```

For comprehensive documentation and more examples, see [EVENT_CALLBACKS.md](EVENT_CALLBACKS.md).

## Prerequisites

1. PostgreSQL running (use Docker Compose or local installation)
2. Database created and accessible

## Running the Examples

### 1. Start the Sync Server

First, start the server:

```bash
# Option 1: Using Docker (recommended)
docker-compose up -d

# Option 2: Manual setup
export DATABASE_URL="postgres://postgres:postgres@localhost/sync_db"
cargo run --bin replicant-server

# Option 3: With monitoring enabled (shows real-time activity)
export DATABASE_URL="postgres://postgres:postgres@localhost/sync_db"
MONITORING=true cargo run --bin replicant-server
```

With monitoring enabled, the server displays:
- Real-time connection events
- All messages sent/received
- JSON patch diffs when documents are updated
- Conflict detection alerts
- Colorized output for easy debugging

### 2. Run the Interactive Task Manager Client

In another terminal, run the interactive client:

```bash
# Run with defaults (creates databases/alice.sqlite3)
cargo run --package replicant-client --example interactive_client

# Or specify a different database name
cargo run --package replicant-client --example interactive_client -- --database bob

# Or specify custom options
cargo run --package replicant-client --example interactive_client -- \
  --database my_tasks \
  --server ws://localhost:8080/ws \
  --token my-auth-token
```

The client provides a modern task management interface with:
- 📋 List tasks with status and priority indicators
- ➕ Create new tasks with form-based input
- ✏️  Edit tasks with guided field editing
- 🔍 View detailed task information
- ✅ Mark tasks as completed
- 🗑️  Delete tasks with confirmation
- 🔄 Check sync status
- 📱 Works offline with automatic sync when reconnected

## Example Workflow

1. **Start the server** - You'll see it's ready when it displays the listening address
2. **Start a client** - It will create a new user ID automatically
3. **Create a task** - Use the guided interface to create a task with title, description, priority, and tags
4. **Watch the server** - If monitoring is enabled, see real-time logs showing the task creation
5. **Edit the task** - Make changes through the form interface and see JSON patches in server logs
6. **Start another client** - Use the same database name to see tasks sync across clients
7. **Try different operations** - Mark tasks complete, change priorities, add tags

## Server Log Example

```
🚀 Sync Server Monitor
=====================

📊 Connecting to database: postgres://postgres:postgres@localhost/sync_db
✅ Created demo user: 123e4567-e89b-12d3-a456-426614174000

🌐 Server listening on: 0.0.0.0:8080
🔌 WebSocket endpoint: ws://0.0.0.0:8080/ws

📋 Activity Log:
────────────────────────────────────────────────────────────────────────────────
14:23:15.123 → Client connected: a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:15.456 ↓ Authenticate from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:15.457 ↑ AuthSuccess to a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:20.789 ↓ CreateDocument from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:20.790 ↑ DocumentCreated to a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:25.123 ↓ UpdateDocument from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:25.124 🔧 Patch applied to document 987fcdeb-51a2-43b7-8c9d-0e1f2a3b4c5d:
     [
       {
         "op": "replace",
         "path": "/content",
         "value": "Updated content"
       },
       {
         "op": "add",
         "path": "/tags",
         "value": ["example", "demo"]
       }
     ]
14:23:25.125 ↑ DocumentUpdated to a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

## Client Interface Example

```
🚀 JSON Database Sync Client
============================
📁 Database: databases/alice.sqlite3
👤 User ID: 123e4567-e89b-12d3-a456-426614174000
🌐 Server: ws://localhost:8080/ws

✅ Connected to sync server!

What would you like to do?
> 📋 List tasks
  ➕ Create new task
  ✏️  Edit task
  🔍 View task details
  ✅ Mark task completed
  🗑️  Delete task
  🔄 Sync status
  ❌ Exit

📋 Your Tasks:
────────────────────────────────────────────────────────────────────────────────────────────────
⏳ 🔴 987fcdeb Fix critical bug - Investigate database connection issues  📤
✅ 🟡 abc12345 Complete project documentation - Write API documentation  
🔄 🟢 def67890 Code review - Review pull request #123  
────────────────────────────────────────────────────────────────────────────────────────────────
Legend: ✅=done 🔄=progress ⏳=pending | 🔴=high 🟡=med 🟢=low | 📤=sync pending
```

## Authentication

The system supports demo mode for easy testing and development:

### Demo Mode (Default)

```bash
# Uses demo-token by default - no setup required
cargo run --package replicant-client --example interactive_client

# Server automatically creates users for demo-token
# Each client gets a unique user ID
```

### Custom Authentication

```bash
# Use your own auth token
cargo run --package replicant-client --example interactive_client -- \
  --token my-custom-token \
  --user-id 550e8400-e29b-41d4-a716-446655440000

# Server will auto-register users with custom tokens in demo mode
```

## Tips

- The client works offline - tasks show sync status indicators (📤 = pending sync)
- You can run multiple clients with the same database name to test real-time sync
- Use monitoring mode (`MONITORING=true`) to see JSON patch diffs and debug sync issues
- The task interface provides guided input - no need to write raw JSON
- Tasks support rich metadata: priorities, tags, descriptions, and completion status
- Use Ctrl+C to cleanly exit either application