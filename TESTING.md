# Testing Guide

This document describes the testing strategy and how to run tests for the JSON-DB-Sync project.

## Testing Philosophy

We follow a pragmatic testing approach:
- **Minimal unit tests** for pure functions (auth token hashing, validators)
- **Comprehensive integration tests** for real-world scenarios
- **Property-based tests** for conflict resolution
- **Load tests** for concurrent connections

## Test Structure

```
sync-server/tests_integration/
├── integration_tests.rs          # Main integration test entry point
├── integration/
│   ├── mod.rs                   # Module declarations
│   ├── helpers.rs               # Test utilities and helpers
│   ├── auth_integration.rs      # Authentication flow tests
│   ├── sync_flow_integration.rs # Document sync scenarios
│   ├── conflict_resolution_integration.rs # Conflict handling
│   ├── websocket_integration.rs # WebSocket protocol tests
│   └── concurrent_clients_integration.rs # Load and concurrency tests
├── basic_test.rs                # Core integration tests
└── full_sync_test.rs            # End-to-end sync scenarios

sync-server/tests/
└── unit_tests.rs                # Server-specific unit tests

test/
├── run_integration_tests.sh      # Docker-based integration tests
├── run_integration_tests_fast.sh # Fast integration tests with caching
└── run_integration_tests_local.sh # Local integration tests
```

## Running Tests

### Quick Reference

```bash
# Unit tests
cargo test --lib --bins

# Integration tests (local PostgreSQL - fast)
./test/run_integration_tests_local.sh

# Integration tests (Docker - consistent environment)
./test/run_integration_tests_docker.sh

# Manual integration test setup
docker-compose -f docker-compose.test.yml up -d
export RUN_INTEGRATION_TESTS=1
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
cargo test integration -- --test-threads=1
```

### Unit Tests
```bash
# Run all unit tests
cargo test --lib --bins

# Run specific test
cargo test test_token_hashing
```

### Integration Tests

#### Option 1: Local PostgreSQL (Fast)
```bash
# Run integration tests with automated setup/teardown (requires local PostgreSQL)
./test/run_integration_tests_local.sh
```

#### Option 2: Docker-based (Consistent)
```bash
# Run integration tests with Docker (consistent environment, no local PostgreSQL needed)
./test/run_integration_tests_docker.sh
```

#### Option 3: Manual setup
```bash
# Start PostgreSQL
docker-compose -f docker-compose.test.yml up -d

# Set environment variables
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
export RUN_INTEGRATION_TESTS=1

# Run integration tests
cargo test integration -- --nocapture
```

### Running Specific Test Categories
```bash
# Auth tests only
cargo test integration::auth_integration

# Conflict resolution tests
cargo test integration::conflict_resolution

# WebSocket tests
cargo test integration::websocket
```

## Writing New Tests

### Integration Test Template
```rust
use crate::integration::helpers::*;

integration_test!(test_my_scenario, |ctx: TestContext| async move {
    // Setup
    let user_id = Uuid::new_v4();
    let client = ctx.create_test_client(user_id, "demo-token").await;
    
    // Test logic
    let doc = TestContext::create_test_document(user_id, "Test Doc");
    client.create_document(doc).await.unwrap();
    
    // Assertions
    let docs = client.get_all_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
});
```

### Test Helpers

- `TestContext`: Provides server URL, database URL, and utility methods
- `create_test_client()`: Creates authenticated sync client
- `create_test_document()`: Generates test documents
- `assert_eventually()`: Waits for async conditions
- `create_authenticated_websocket()`: Direct WebSocket connection

## CI/CD Integration

Tests run automatically on:
- Push to `main` or `develop` branches
- Pull requests to `main`

GitHub Actions workflow includes:
1. Unit tests
2. Integration tests (with PostgreSQL service)
3. Docker integration tests
4. Linting (rustfmt, clippy)
5. Security audit

## Performance Testing

Load tests are included in `concurrent_clients_integration.rs`:
- Many concurrent clients test
- Rapid update scenarios
- Server under load conditions

## Debugging Tests

### Enable Debug Logging
```bash
export RUST_LOG=debug
export RUST_BACKTRACE=1
cargo test integration::failing_test -- --nocapture
```

### View Server Logs
```bash
# During Docker tests
docker-compose -f docker-compose.integration.yml logs -f sync-server-test
```

### Database Inspection
```bash
# Connect to test database
docker exec -it sync-workspace_postgres-test_1 psql -U postgres -d sync_test_db

# View test data
\dt  # List tables
SELECT * FROM users;
SELECT * FROM documents;
```

## Common Issues

### Tests Skipped
If you see "Skipping integration test", set:
```bash
export RUN_INTEGRATION_TESTS=1
```

### Database Connection Failed
Ensure PostgreSQL is running:
```bash
docker-compose -f docker-compose.test.yml ps
```

### Port Conflicts
If ports 5433 or 8081 are in use, modify `docker-compose.integration.yml`

## Test Coverage Goals

- ✅ Authentication flows (demo token, custom tokens, auto-registration)
- ✅ Basic sync operations (create, update, delete)
- ✅ Conflict resolution scenarios
- ✅ WebSocket protocol handling
- ✅ Concurrent client scenarios
- ✅ Error handling and recovery
- ✅ Performance under load
- ⬜ Network failure simulation
- ⬜ Database failure recovery
- ⬜ Memory usage under stress