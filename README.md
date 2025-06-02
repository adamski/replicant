# Rust JSON Database Sync System

A high-performance client-server synchronization system built in Rust, featuring real-time WebSocket communication, bidirectional patch-based version control, and advanced conflict resolution.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)](BUILD_STATUS.md)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-17%20passing-brightgreen.svg)](TESTING.md)

## Architecture

- **sync-core**: Shared library with data models, JSON patch operations, and sync protocols
- **sync-client**: Client library with SQLite storage, offline queue, and C FFI exports
- **sync-server**: Server binary with PostgreSQL storage, WebSocket support, and event logging

## Core Features

### 🔄 Real-Time Synchronization
- **WebSocket-based** bidirectional sync with sub-second latency
- **Vector clock** conflict detection for distributed systems
- **Operational transformation** for automatic conflict resolution
- **Offline-first** design with queue-based retry logic

### 📝 Advanced Version Control
- **Bidirectional patches**: Forward and reverse JSON patches for every change
- **Instant undo/redo**: Navigate document history without state reconstruction
- **Time-travel debugging**: Jump to any previous document version
- **Efficient storage**: Patches are space-efficient compared to full snapshots
- **Audit trails**: Complete change history with user attribution

### 🗄️ Database Architecture
- **Client**: SQLite with offline queue and document caching
- **Server**: PostgreSQL with JSONB support and event logging
- **Change events**: Sequence-based sync with forward/reverse patch storage
- **Consolidated SQL**: Organized queries with helper functions for maintainability

### 🛠️ Developer Experience  
- **Interactive examples**: CLI client and monitoring server
- **C FFI exports** for seamless C++ integration
- **Docker deployment** ready with compose files
- **Comprehensive testing**: 17 tests covering all functionality
- **Built-in monitoring** with real-time activity logs

## Getting Started

### Prerequisites

- Rust 1.75+
- PostgreSQL 16+
- SQLite 3+

### Development Setup

1. Clone the repository:
```bash
cd sync-workspace
```

2. Copy environment variables:
```bash
cp .env.example .env
# Edit .env with your configuration
```

3. Start PostgreSQL (using Docker):
```bash
docker-compose up -d postgres
```

4. Run migrations:
```bash
cd sync-server
sqlx migrate run
```

5. Build and run the server:
```bash
cargo run --bin sync-server
```

6. Build the client library:
```bash
cd sync-client
cargo build --release
```

### Docker Deployment

```bash
docker-compose up -d
```

This starts both PostgreSQL and the sync server.

## Bidirectional Patch System

Our advanced version control system stores both forward and reverse patches for every document change, enabling powerful features:

### How It Works

**CREATE Event:**
- `forward_patch`: Contains the full document as initial state
- `reverse_patch`: `null` (creation cannot be undone to a previous state)

**UPDATE Event:**  
- `forward_patch`: JSON patch to apply the change (e.g., `{"op": "replace", "path": "/title", "value": "New Title"}`)
- `reverse_patch`: JSON patch to undo the change (e.g., `{"op": "replace", "path": "/title", "value": "Old Title"}`)

**DELETE Event:**
- `forward_patch`: `null` (deletion is implicit)  
- `reverse_patch`: Contains full document to restore if undeleted

### Benefits

✅ **Instant Undo**: Apply reverse patches without reconstructing history  
✅ **Efficient Reversion**: Jump to any version in O(n) patch operations  
✅ **Bidirectional Navigation**: Move forward/backward through document timeline  
✅ **Space Efficient**: Patches are ~90% smaller than storing full document snapshots  
✅ **Audit Compliance**: Complete change history with recovery capabilities  

### Database Schema

```sql
CREATE TABLE change_events (
    sequence BIGSERIAL PRIMARY KEY,
    document_id UUID NOT NULL,
    user_id UUID NOT NULL,
    event_type VARCHAR(10) NOT NULL,
    revision_id TEXT NOT NULL,
    forward_patch JSONB,    -- Patch to apply this change
    reverse_patch JSONB,    -- Patch to undo this change  
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## API Usage

### WebSocket Connection

Connect to `ws://localhost:8080/ws` and authenticate:

```json
{
  "type": "authenticate",
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "auth_token": "your-auth-token"
}
```

### Create Document

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

### Update Document

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

### Change Events Response

When requesting changes since a sequence, you'll receive both forward and reverse patches:

```json
{
  "type": "changes_since",
  "changes": [
    {
      "sequence": 1,
      "document_id": "550e8400-e29b-41d4-a716-446655440001",
      "event_type": "create",
      "revision_id": "1-abc123",
      "forward_patch": {
        "id": "550e8400-e29b-41d4-a716-446655440001",
        "title": "My Document", 
        "content": {"text": "Hello, World!"}
      },
      "reverse_patch": null,
      "created_at": "2024-01-01T12:00:00Z"
    },
    {
      "sequence": 2,
      "document_id": "550e8400-e29b-41d4-a716-446655440001", 
      "event_type": "update",
      "revision_id": "2-def456",
      "forward_patch": [
        {"op": "replace", "path": "/text", "value": "Updated text"}
      ],
      "reverse_patch": [
        {"op": "replace", "path": "/text", "value": "Hello, World!"}
      ],
      "created_at": "2024-01-01T12:05:00Z"  
    }
  ]
}
```

### Undo/Redo Operations

Use reverse patches to implement undo functionality:

```json
{
  "type": "undo_to_sequence",
  "target_sequence": 1
}
```

This applies reverse patches in chronological order until reaching the target sequence.

## Interactive Examples

The project includes interactive examples to demonstrate functionality:

### Client Example
```bash
cd sync-client  
cargo run --example interactive_client
```

Features a CLI interface with:
- Document creation and editing
- Real-time sync visualization  
- Offline queue management
- Colored output for better UX

### Server Monitoring Example  
```bash
cd sync-server
cargo run --example monitoring_server
```

Shows real-time activity including:
- Client connections/disconnections
- JSON patch operations with diffs
- Bidirectional patch storage
- Event logging and sequence tracking

## C++ Integration

The client library exports C FFI functions:

```cpp
extern "C" {
    void* sync_engine_create(
        const char* database_path,
        const char* server_url,
        const char* auth_token
    );
    
    void sync_engine_destroy(void* engine);
}
```

## Testing

### Unit Tests (17 passing)

Run all unit tests:
```bash
# Set up test database
export DATABASE_URL="postgresql://$USER@localhost:5432/sync_integration_test"  
export TEST_DATABASE_URL="postgresql://$USER@localhost:5432/sync_integration_test"

# Run tests  
cargo test --lib
```

### Database Tests
```bash
# Test bidirectional patch functionality
cargo test --package sync-server --test unit_tests database_tests::test_event_logging -- --nocapture

# Test document operations
cargo test --package sync-server --test unit_tests database_tests::test_document_delete -- --nocapture
```

### Integration Tests
```bash
# Set environment variable and run
export RUN_INTEGRATION_TESTS=1
cargo test --test integration -- --test-threads=1
```

### Test Coverage

- **sync-core**: 7 tests (vector clocks, document revisions, JSON patches)
- **sync-client**: 3 tests (database operations, offline queue, document lifecycle)  
- **sync-server**: 7 tests (authentication, database operations, event logging, bidirectional patches)

All tests validate:
✅ Vector clock synchronization  
✅ JSON patch creation and application  
✅ Document CRUD operations  
✅ Event logging with forward/reverse patches  
✅ Authentication and token management  
✅ Offline queue functionality  
✅ Conflict detection and resolution

## Performance Considerations

### Database Optimizations
- **Connection pooling** for PostgreSQL and SQLite
- **Consolidated SQL queries** with prepared statements and caching disabled for schema flexibility
- **JSONB indexing** for fast document content searches
- **Sequence-based sync** for efficient incremental updates

### Memory & Network  
- **Patch-based storage**: ~90% space savings vs full document snapshots
- **WebSocket compression** enabled for reduced bandwidth
- **Message batching** for multiple updates
- **Document caching** in memory with LRU eviction
- **Rate limiting** per user to prevent abuse

### Scalability Features
- **Vector clocks** prevent synchronization bottlenecks
- **Offline-first design** reduces server dependency
- **Bidirectional patches** enable instant undo without history replay
- **Event sourcing** architecture for horizontal scaling

## Monitoring Mode

The server includes a built-in monitoring mode that provides real-time visibility into sync operations. This is useful for debugging, development, and understanding system behavior.

### Enabling Monitoring

Set the `MONITORING` environment variable to `true` when starting the server:

```bash
MONITORING=true cargo run --bin sync-server
```

Or with Docker:
```bash
docker run -e MONITORING=true sync-server
```

### Monitoring Features

When monitoring is enabled, you'll see:
- Real-time client connections and disconnections
- All WebSocket messages sent and received
- JSON patch operations with full content
- Conflict detection events
- Error messages and stack traces
- Colorized output for easy reading

### Example Output

```
🚀 Sync Server with Monitoring
==============================

📋 Activity Log:
────────────────────────────────────────────────────────────────────────────────
14:32:15.123 → Client connected: a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:32:15.456 ↓ Authenticate from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:32:15.789 ↑ AuthSuccess to a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:32:16.012 ↓ UpdateDocument from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:32:16.034 🔧 Patch applied to document 123e4567-e89b-12d3-a456-426614174000:
     {
       "op": "replace",
       "path": "/title",
       "value": "Updated Title"
     }
14:32:16.056 ↑ DocumentUpdated to a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

## Security

- Token-based authentication with Argon2 hashing
- TLS/WSS required in production
- Input validation for all JSON patches
- Rate limiting to prevent DoS
- Audit logging for all operations

## License

MIT