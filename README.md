# JSON Database Sync

A client-server synchronization system built in Rust, featuring real-time WebSocket communication, bidirectional patch-based version control, and automatic conflict resolution.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Features

### 🔄 Real-Time Synchronization
- **WebSocket-based** bidirectional sync
- **Vector clock** conflict detection for distributed systems
- **Basic conflict resolution** with server-wins fallback
- **Offline-first** design with local storage
- **Concurrent update handling** with conflict detection

### 📝 Version Control
- **JSON patches**: Forward patches for document changes
- **Document versioning**: Each document has version and revision tracking
- **Change events**: All modifications logged with sequence numbers
- **Audit trails**: Changes logged with user attribution

### 🗄️ Database Architecture
- **Client**: SQLite with offline queue and document caching
- **Server**: PostgreSQL with JSONB support and event logging
- **Change events**: Sequence-based sync with forward/reverse patch storage

## Architecture

- **sync-core**: Shared library with data models, JSON patch operations, and sync protocols
- **sync-client**: Client library with SQLite storage, offline queue, and C FFI exports
- **sync-server**: Server binary with PostgreSQL storage, WebSocket support, and event logging

## Quick Start

### Prerequisites
- Rust 1.75+
- PostgreSQL 16+
- Docker (optional)

### Option 1: Docker (Recommended)

```bash
git clone https://github.com/yourusername/json-db-sync.git
cd json-db-sync/sync-workspace
docker-compose up -d
```

This starts PostgreSQL and the sync server on port 8080.

### Option 2: Manual Setup

1. **Install dependencies:**
```bash
cargo build --workspace
```

2. **Set up PostgreSQL:**
```bash
createdb sync_db
export DATABASE_URL="postgres://user:password@localhost/sync_db"
```

3. **Run migrations:**
```bash
cd sync-server
sqlx migrate run
```

4. **Start the server:**
```bash
cargo run --bin sync-server
```

### Try the Interactive Client

```bash
# Uses demo authentication, creates database in databases/alice.sqlite3
cargo run --package sync-client --example interactive_client

# Or specify a different database name
cargo run --package sync-client --example interactive_client -- --database bob
```

The interactive client provides a task management interface with:
- ✅ Create, edit, and complete tasks
- 🏷️ Priority levels and tags
- 📋 Rich task listing with status indicators
- 🔄 Real-time sync across multiple clients
- 📱 Offline support with automatic sync when reconnected

## API Usage

### Rust Client Library

```rust
use sync_client::SyncEngine;
use serde_json::json;

// Connect to server
let engine = SyncEngine::new(
    "sqlite:client.db?mode=rwc",
    "ws://localhost:8080/ws", 
    "demo-token"
).await?;

// Create a document
let doc = engine.create_document(
    "My Document".to_string(),
    json!({
        "title": "Task title",
        "description": "Task description", 
        "status": "pending",
        "priority": "medium",
        "tags": ["work", "important"]
    })
).await?;

// Update document
engine.update_document(doc.id, json!({
    "status": "completed"
})).await?;
```

#### Rust Event Callbacks

```rust
use sync_client::events::{EventDispatcher, EventType};

// Get event dispatcher from engine
let events = engine.event_dispatcher();

// Register callback for document events
events.register_callback(
    |event_type, document_id, title, content, error, numeric_data, boolean_data, context| {
        match event_type {
            EventType::DocumentCreated => {
                println!("📄 New document: {}", title.unwrap_or("untitled"));
            },
            EventType::DocumentUpdated => {
                println!("✏️ Updated: {}", title.unwrap_or("untitled"));
            },
            EventType::SyncCompleted => {
                println!("🔄 Synced {} documents", numeric_data);
            },
            _ => {}
        }
    },
    std::ptr::null_mut(), // context
    None // filter (None = all events)
)?;

// In your main loop
loop {
    let processed = events.process_events()?;
    if processed > 0 {
        println!("Processed {} events", processed);
    }
    
    tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
}
```

### WebSocket API

Connect to `ws://localhost:8080/ws` and authenticate:

```json
{
  "type": "authenticate",
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "auth_token": "demo-token"
}
```

Create documents:
```json
{
  "type": "create_document",
  "document": {
    "id": "550e8400-e29b-41d4-a716-446655440001",
    "title": "My Document",
    "content": {"text": "Hello, World!"}
  }
}
```

Update with JSON patches:
```json
{
  "type": "update_document",
  "patch": {
    "document_id": "550e8400-e29b-41d4-a716-446655440001",
    "patch": [
      {"op": "replace", "path": "/text", "value": "Updated text"}
    ]
  }
}
```

### C/C++ Integration

The sync client provides a C API that can be used from C, C++, and other languages. Build the distribution SDK:

```bash
# Build the complete SDK with headers and libraries
./scripts/build_dist.sh

# This creates:
# - dist/include/sync_client.h    (C header)
# - dist/include/sync_client.hpp  (C++ wrapper)
# - dist/lib/                     (compiled libraries)
# - dist/examples/                (working examples)
```

#### Event Callback System

The sync client includes a comprehensive event callback system for real-time notifications about document changes, sync operations, and connection status.

**Key Features:**
- Thread-safe design with no locks needed in user code
- Support for Rust, C, and C++ applications
- Event filtering and context passing
- Offline and online event notifications

**Quick C Example:**
```c
#include "sync_client_events.h"

void my_callback(const SyncEventData* event, void* context) {
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
            printf("📄 Document created: %s\n", event->title);
            break;
        case SYNC_EVENT_SYNC_COMPLETED:
            printf("🔄 Sync completed: %llu documents\n", event->numeric_data);
            break;
    }
}

int main() {
    struct CSyncEngine* engine = sync_engine_create(
        "sqlite:client.db", "ws://localhost:8080/ws", "demo-token");
    
    // Register callback for all events
    sync_engine_register_event_callback(engine, my_callback, NULL, -1);
    
    // Main loop - process events regularly
    while (running) {
        uint32_t processed_count;
        sync_engine_process_events(engine, &processed_count);
        // ... do other work
        usleep(16000); // ~60 FPS
    }
    
    sync_engine_destroy(engine);
    return 0;
}
```

**C++ Example with Lambdas:**
```cpp
#include "sync_client_events.h"

int main() {
    // Simple RAII wrapper
    SyncEngine engine("sqlite:client.db", "ws://localhost:8080/ws", "demo-token");
    
    // Lambda callback with capture
    int event_count = 0;
    auto callback = [&event_count](const SyncEventData* event, void* context) {
        event_count++;
        std::cout << "Event #" << event_count << ": " 
                 << get_event_name(event->event_type) << std::endl;
    };
    
    // Register callback
    engine.register_callback(callback);
    
    // Main loop
    while (running) {
        engine.process_events();
        std::this_thread::sleep_for(std::chrono::milliseconds(16));
    }
    
    return 0;
}
```

For complete documentation and advanced examples, see [EVENT_CALLBACKS.md](EVENT_CALLBACKS.md).

**C API Example:**

```c
#include "sync_client.h"
#include <stdio.h>
#include <stdlib.h>

int main() {
    // Create sync engine (works both online and offline)
    struct CSyncEngine* engine = sync_engine_create(
        "sqlite:client.db?mode=rwc",
        "ws://localhost:8080/ws", 
        "demo-token"
    );
    
    if (!engine) {
        printf("Failed to create sync engine\n");
        return 1;
    }
    
    // Get version
    char* version = sync_get_version();
    if (version) {
        printf("Sync client version: %s\n", version);
        sync_string_free(version);
    }
    
    // Create a document
    char doc_id[37] = {0}; // UUID string + null terminator
    enum CSyncResult result = sync_engine_create_document(
        engine,
        "My Document",
        "{\"content\":\"Hello World\",\"type\":\"note\",\"priority\":\"medium\"}",
        doc_id
    );
    
    if (result == Success) {
        printf("Created document: %s\n", doc_id);
        
        // Update the document
        result = sync_engine_update_document(
            engine,
            doc_id,
            "{\"content\":\"Hello Updated World\",\"type\":\"note\",\"priority\":\"high\"}"
        );
        
        if (result == Success) {
            printf("Updated document successfully\n");
        }
    }
    
    // Clean up
    sync_engine_destroy(engine);
    return 0;
}
```

**C++ Integration:**

```cpp
#include "sync_client.hpp"
#include <iostream>

int main() {
    try {
        // Works both online and offline
        SyncClient::sync_engine engine(
            "sqlite:client.db?mode=rwc",
            "ws://localhost:8080/ws", 
            "demo-token"
        );
        
        std::cout << "Sync client version: " << SyncClient::sync_engine::get_version() << std::endl;
        
        // Create a document
        auto doc_id = engine.create_document(
            "My Document",
            R"({"content":"Hello World","type":"note","priority":"medium"})"
        );
        
        std::cout << "Created document: " << doc_id << std::endl;
        
        // Update the document
        engine.update_document(
            doc_id,
            R"({"content":"Hello Updated World","type":"note","priority":"high"})"
        );
        
        std::cout << "Updated document successfully" << std::endl;
        
    } catch (const SyncClient::sync_exception& e) {
        std::cerr << "Sync error: " << e.what() << std::endl;
        return 1;
    }
    
    return 0;
}
```

**Building and Linking:**

Using the distribution SDK (recommended):

```cmake
# CMakeLists.txt
cmake_minimum_required(VERSION 3.15)
project(MyProject)

set(CMAKE_CXX_STANDARD 11)

# Find the sync client SDK
find_package(sync_client REQUIRED PATHS /path/to/sync-workspace/dist)

# Your C++ application
add_executable(my_app main.cpp)
target_link_libraries(my_app sync_client)
```

Or compile directly:

```bash
# Build the distribution first
./scripts/build_dist.sh

# Compile your C/C++ code (example for macOS/Linux)
gcc -I./dist/include your_code.c -L./dist/lib -lsync_client -framework Security -o your_program

# For C++
g++ -std=c++11 -I./dist/include your_code.cpp -L./dist/lib -lsync_client -framework Security -o your_program
```

## Bidirectional Patch System

The system stores both forward and reverse patches for every document change:

### How It Works

**CREATE Event:**
- `forward_patch`: Contains the full document as initial state
- `reverse_patch`: `null` (creation cannot be undone)

**UPDATE Event:**  
- `forward_patch`: JSON patch to apply the change
- `reverse_patch`: JSON patch to undo the change

**DELETE Event:**
- `forward_patch`: `null` (deletion is implicit)  
- `reverse_patch`: Contains full document to restore if undeleted

### Benefits

✅ **Complete Audit Trail**: All changes logged with recovery capabilities  
✅ **Efficient Storage**: JSON patches store only the differences between versions
✅ **Version History**: Forward and reverse patch support for future undo functionality  

## Conflict Resolution

The system handles concurrent updates with basic conflict detection:

1. **Multiple clients** can update the same document simultaneously
2. **Server processes updates** sequentially and detects conflicts using vector clocks
3. **Server-wins fallback** for conflict resolution (clients accept server state)
4. **Conflict detection** implemented with vector clock comparison

## Testing

### Unit Tests
```bash
cargo test --lib --bins
```

### Integration Tests
```bash
# Local setup with PostgreSQL (fast)
./test/run_integration_tests_local.sh

# Docker-based setup (consistent environment)
./test/run_integration_tests_docker.sh

# Manual setup
docker-compose -f docker-compose.test.yml up -d
export RUN_INTEGRATION_TESTS=1
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
cargo test integration -- --test-threads=1
```

### Test Coverage
- **sync-core**: 7 tests (vector clocks, document revisions, JSON patches)
- **sync-client**: 1 test (basic functionality)  
- **sync-server**: 7 tests (authentication, WebSocket protocol, concurrent scenarios)

## Authentication

The system supports demo mode for easy testing:

- **Demo token**: Use `demo-token` with any user ID
- **Auto-registration**: Server automatically creates users for demo tokens
- **Extensible design**: Authentication system ready for production enhancements

### Production Considerations

For production deployment, you'll need to implement:

**Core Authentication Flow:**
1. **User Registration**: Secure user creation with password hashing (system already uses Argon2)
2. **Login/Session Management**: JWT or session tokens with expiry and refresh mechanisms
3. **Token Validation**: Secure token verification with proper error handling

**Security Requirements:**
- **Session Storage**: Use Redis or distributed cache for session management
- **Rate Limiting**: Prevent brute force attacks on authentication endpoints
- **Password Security**: Already implemented with Argon2 hashing
- **TLS/WSS**: Use secure WebSocket connections in production
- **Audit Logging**: Track authentication events and failed attempts

**Scalability Features:**
- **Stateless Sessions**: Consider JWT for horizontal scaling
- **Token Refresh**: Implement refresh tokens for better UX
- **Multi-Factor Authentication**: Add TOTP or SMS verification
- **OAuth2 Integration**: Support third-party authentication providers

The current implementation provides a solid foundation with secure hashing and extensible architecture.

## Performance & Security

### Optimizations
- **Connection pooling** for PostgreSQL and SQLite
- **JSONB indexing** for fast document searches
- **Patch-based storage** for efficient change tracking

### Security Features
- Token-based authentication with secure hashing (Argon2)
- Input validation for JSON patches
- Demo token support for development

## License

MIT

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Run `cargo test` and `./test/run_integration_tests_docker.sh`
5. Submit a pull request

See [TESTING.md](TESTING.md) for detailed testing guidelines and [EXAMPLES.md](EXAMPLES.md) for usage examples.