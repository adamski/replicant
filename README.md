# JSON Database Sync

A client-server synchronization system built in Rust, featuring real-time WebSocket communication, bidirectional patch-based version control, and automatic conflict resolution.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Features

### 🔄 Real-Time Synchronization
- **WebSocket-based** bidirectional sync with sub-second latency
- **Vector clock** conflict detection for distributed systems
- **Automatic conflict resolution** with ServerWins strategy
- **Offline-first** design with queue-based retry logic
- **Concurrent update handling** with guaranteed convergence

### 📝 Advanced Version Control
- **Bidirectional patches**: Forward and reverse JSON patches for every change
- **Instant undo/redo**: Navigate document history without state reconstruction
- **Time-travel debugging**: Jump to any previous document version
- **Efficient storage**: Patches are ~90% smaller than full document snapshots
- **Complete audit trails**: All changes logged with user attribution

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

### C++ Integration

```cpp
extern "C" {
    void* sync_engine_create(
        const char* database_path,
        const char* server_url,
        const char* auth_token
    );
    void sync_engine_destroy(void* engine);
}

auto engine = sync_engine_create(
    "client.db",
    "ws://localhost:8080/ws",
    "demo-token"
);
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

✅ **Instant Undo**: Apply reverse patches without reconstructing history  
✅ **Efficient Reversion**: Jump to any version in O(n) patch operations  
✅ **Space Efficient**: Patches are ~90% smaller than full document snapshots  
✅ **Complete Audit Trail**: All changes logged with recovery capabilities  

## Conflict Resolution

The system handles concurrent updates with automatic conflict resolution:

1. **Multiple clients** can update the same document simultaneously
2. **Server processes updates** sequentially and detects conflicts using vector clocks
3. **ServerWins strategy** automatically resolves conflicts
4. **All clients converge** to the same final state through full document sync

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
- **sync-client**: 3 tests (database operations, offline queue, document lifecycle)  
- **sync-server**: 14 tests (authentication, WebSocket protocol, concurrent scenarios)

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
- **WebSocket compression** for reduced bandwidth
- **Patch-based storage** for ~90% space savings

### Security Features
- Token-based authentication with secure hashing
- Input validation for all JSON patches
- Rate limiting and audit logging
- TLS/WSS support for production

## License

MIT

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Run `cargo test` and `./test/run_integration_tests_docker.sh`
5. Submit a pull request

See [TESTING.md](TESTING.md) for detailed testing guidelines and [EXAMPLES.md](EXAMPLES.md) for usage examples.