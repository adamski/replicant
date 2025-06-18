# Integration Tests

This directory contains integration tests for the JSON DB Sync system. All tests are written in Python for consistency and maintainability.

## Test Suite Overview

### Core Tests

**`test_persistent_client_reconnection.py`** - **Primary Test**
- Tests the main fix for persistent sync engine reconnection
- Simulates the exact Task List scenario with long-running sync engines
- Verifies that offline edits are properly synced after server restart
- Uses daemon mode to maintain the same sync engine instance throughout the test

**`test_real_world_reconnection.py`** - **Real-World Validation**
- Validates real-world usage patterns for sync engines
- Tests reconnection behavior with actual client applications
- Ensures compatibility with applications like Task List TUI

**`test_offline_update_sync.py`** - **Basic Offline Sync**
- Verifies basic offline edit synchronization
- Tests document updates when server is unavailable
- Validates that changes are queued and uploaded upon reconnection

**`test_two_client_offline_sync.py`** - **Multi-Client Sync**
- Tests synchronization between multiple clients
- Verifies that offline changes propagate correctly between clients
- Validates conflict resolution and document consistency

## Running Tests

### Prerequisites

1. **PostgreSQL**: Tests require a running PostgreSQL instance
   ```bash
   # macOS with Homebrew
   brew services start postgresql
   
   # Or start manually
   postgres -D /usr/local/var/postgres
   ```

2. **Python Dependencies**: Tests use built-in Python libraries only
   - No additional pip packages required
   - Compatible with Python 3.8+

### Running Individual Tests

```bash
# Run the primary persistent client test
cd tests
python test_persistent_client_reconnection.py

# Run real-world validation
python test_real_world_reconnection.py

# Run basic offline sync test
python test_offline_update_sync.py

# Run multi-client sync test
python test_two_client_offline_sync.py
```

### Running All Tests

```bash
# Run all tests in sequence
cd tests
for test in test_*.py; do
    echo "Running $test..."
    python "$test"
    echo "---"
done
```

## Test Architecture

### Database Setup
- Each test creates its own temporary database
- Tests clean up automatically on completion
- Database names follow pattern: `sync_test_<test_name>`

### Client Management
- Tests use the `simple_sync_test` example binary in daemon mode
- Persistent clients maintain the same sync engine instance
- All processes are properly terminated after tests

### Server Management
- Tests start and stop their own server instances
- Server logs are captured for debugging
- Port 8080 is used for WebSocket connections

## Key Validation Points

1. **Persistent Sync Engines**: Verify that the same sync engine instance handles both normal operations and post-reconnection sync
2. **Offline Edit Queuing**: Ensure offline changes are properly stored and uploaded
3. **Automatic Reconnection**: Validate that clients detect server restarts and reconnect
4. **Cross-Client Propagation**: Verify that changes made by one client appear on others
5. **Heartbeat Detection**: Confirm that silent disconnections are detected via ping/pong

## Debugging

### Log Files
- Server logs: Captured in test output
- Client logs: Available via `RUST_LOG=debug` environment variable

### Common Issues
- **Port conflicts**: Ensure no other services are using port 8080
- **Database permissions**: Verify PostgreSQL user has CREATE DATABASE privileges
- **Process cleanup**: Tests attempt to kill orphaned processes automatically

### Manual Testing
Use the `simple_sync_test` example for manual validation:

```bash
# Start in daemon mode for persistent testing
cargo run --example simple_sync_test -- daemon --database test_db --user test@example.com

# Then send commands via stdin:
CREATE:Test Task:A test task
STATUS
QUIT
```