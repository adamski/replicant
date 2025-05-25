# Rust JSON Database Sync System

A high-performance client-server synchronization system built in Rust, featuring real-time WebSocket communication, conflict resolution, and offline support.

## Architecture

- **sync-core**: Shared library with data models and sync protocols
- **sync-client**: Client library with SQLite storage and C FFI exports
- **sync-server**: Server binary with PostgreSQL storage and WebSocket support

## Features

- Real-time bidirectional sync via WebSockets
- JSON document storage with JSONB support
- Vector clock-based conflict detection
- Operational transformation for conflict resolution
- Offline queue with retry logic
- C FFI exports for C++ integration
- Docker deployment ready

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

Run all tests:
```bash
cargo test --workspace
```

Run integration tests:
```bash
cargo test --test sync_integration -- --test-threads=1
```

## Performance Considerations

- Connection pooling for databases
- Message batching for multiple updates
- WebSocket compression enabled
- Document caching in memory
- Rate limiting per user

## Security

- Token-based authentication with Argon2 hashing
- TLS/WSS required in production
- Input validation for all JSON patches
- Rate limiting to prevent DoS
- Audit logging for all operations

## License

MIT