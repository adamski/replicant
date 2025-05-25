# Quick Start Guide

## Prerequisites
- Rust 1.75+
- PostgreSQL 16+
- Docker (optional)

## Running with Docker (Recommended)

1. Clone the repository:
```bash
git clone git@gitlab.com:node-audio/json-db-sync.git
cd json-db-sync
```

2. Start the services:
```bash
docker-compose up -d
```

This starts PostgreSQL and the sync server on port 8080.

## Manual Setup

1. Install dependencies:
```bash
cargo build --workspace
```

2. Set up PostgreSQL:
```bash
createdb sync_db
export DATABASE_URL="postgres://user:password@localhost/sync_db"
```

3. Run migrations:
```bash
cd sync-server
sqlx migrate run
```

4. Start the server:
```bash
cargo run --bin sync-server
```

## Client Integration

### Rust Client
```rust
use sync_client::SyncEngine;

let engine = SyncEngine::new(
    "client.db",
    "ws://localhost:8080/ws",
    "auth_token"
).await?;

// Create a document
let doc = engine.create_document(
    "My Document".to_string(),
    json!({ "content": "Hello!" })
).await?;
```

### C++ Integration
```cpp
extern "C" {
    void* sync_engine_create(
        const char* db_path,
        const char* server_url, 
        const char* auth_token
    );
    void sync_engine_destroy(void* engine);
}

auto engine = sync_engine_create(
    "client.db",
    "ws://localhost:8080/ws",
    "auth_token"
);
```

## Testing

Run all tests:
```bash
cargo test --workspace
```

Run integration tests with PostgreSQL:
```bash
./test_integration.sh
```

## WebSocket API

Connect to `ws://localhost:8080/ws` and send:

```json
{
    "type": "authenticate",
    "user_id": "550e8400-e29b-41d4-a716-446655440000",
    "auth_token": "your-token"
}
```

Then create documents:
```json
{
    "type": "create_document",
    "document": {
        "id": "550e8400-e29b-41d4-a716-446655440001",
        "title": "My Document",
        "content": {"text": "Hello!"}
    }
}
```