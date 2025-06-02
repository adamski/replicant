# Claude Code Assistant Instructions

This document contains important instructions and context for Claude when working on this project.

## Environment Variables for Testing

When running tests, the following environment variables should be set:

```bash
# For server database tests (PostgreSQL)
export DATABASE_URL="postgresql://$USER@localhost:5432/sync_integration_test"
export TEST_DATABASE_URL="postgresql://$USER@localhost:5432/sync_integration_test"

# For running integration tests
export RUN_INTEGRATION_TESTS=1

# Create test database if it doesn't exist
psql -U $USER -d postgres -c "CREATE DATABASE IF NOT EXISTS sync_test_db;"
```

## Testing Database Operations

### IMPORTANT: Choose One Database Setup

**Before running tests, choose either Docker OR local PostgreSQL** (not both simultaneously):

1. **Check port 5432**: `lsof -i :5432` to see what's running
2. **Stop conflicting services** to avoid connecting to wrong database
3. **Run migrations and tests against the SAME database**

### Running Tests with PostgreSQL

```bash
# Create a test database (if not exists)
psql -U $USER -d postgres -c "DROP DATABASE IF EXISTS sync_test_db;"
psql -U $USER -d postgres -c "CREATE DATABASE sync_test_db;"

# Run tests with DATABASE_URL
export DATABASE_URL="postgresql://$USER@localhost:5432/sync_test_db"
cargo test --package sync-server --test unit_tests

# Or run specific test
DATABASE_URL="postgresql://$USER@localhost:5432/sync_test_db" cargo test --package sync-server --test unit_tests test_event_logging -- --nocapture
```

### Important Notes

1. **Clean Database State**: Tests should clean the database before each run to avoid conflicts. The test setup includes a `cleanup_database()` function that deletes all data in the correct order respecting foreign key constraints.

2. **Migration Order**: Always run migrations before cleaning the database:
   ```rust
   db.run_migrations().await?;
   cleanup_database(&db).await?;
   ```

3. **Schema Changes**: If you change column types (like revision_id from UUID to TEXT), you may need to:
   - Drop and recreate the test database
   - Clear the cargo build cache with `cargo clean`
   - Check for any SQLx compile-time query verification issues

## Event Logging System

The event logging system tracks all document operations (CREATE, UPDATE, DELETE) in a `change_events` table for reliable synchronization.

### Key Components

1. **change_events table**: Stores all document changes with:
   - Auto-incrementing sequence number
   - User ID for filtering
   - Document ID
   - Event type (create/update/delete)
   - Revision ID (CouchDB-style format like "2-abc123")
   - JSON patch data (for updates)
   - Timestamp

2. **Database Methods**:
   - `log_change_event()`: Internal method to log events in a transaction
   - `get_changes_since()`: Retrieve events after a specific sequence number
   - `get_latest_sequence()`: Get the most recent sequence for a user

3. **CouchDB-style Revisions**: 
   - Format: `"{generation}-{hash}"` (e.g., "2-a8d73487645ef")
   - Generation increments on each update
   - Hash is first 16 chars of SHA256 of content

### Testing Event Logging

The `test_event_logging()` test verifies:
- CREATE events are logged when documents are created
- UPDATE events include JSON patch data
- DELETE events are logged with proper revision
- Sequence numbers increment correctly
- Events can be retrieved in order

## Common Issues

### SQLx Type Mismatch Errors & Docker/Local Database Confusion

If you encounter errors like "column revision_id is of type uuid but expression is of type text":

#### Root Cause Analysis

**CRITICAL DISCOVERY**: The persistent SQLx error was caused by connecting to the wrong database:

1. **Docker PostgreSQL** was running on port 5432 with the **old UUID schema**
2. **Local Postgres.app** couldn't start due to port conflict
3. **Tests connected to Docker** (localhost:5432) but we were **migrating the local database**
4. **SQLx connected to Docker's old schema** while we thought we were testing against migrated local DB

#### The Issue Chain

```
Tests run → Connect to localhost:5432 → Docker PostgreSQL (old UUID schema)
                                      ↗
Migrations run → Local PostgreSQL (new TEXT schema) ← We thought we were testing this
```

#### Solutions

**Option 1: Use Docker Only (Recommended for reproducibility)**
```bash
# Stop local PostgreSQL, use Docker exclusively
docker-compose down && docker-compose up -d postgres
DATABASE_URL="postgres://sync_user:sync_password@localhost:5432/sync_db"
# Run migrations against Docker database
sqlx migrate run --source sync-server/migrations
# Run tests against Docker database
cargo test --package sync-server --test unit_tests
```

**Option 2: Use Local PostgreSQL Only**
```bash
# Stop Docker to free port 5432
docker-compose down
# Start local PostgreSQL (Postgres.app or brew)
open -a Postgres  # or brew services start postgresql
# Create test database
psql -U $USER -d postgres -c "CREATE DATABASE sync_test_db;"
# Run tests against local database
DATABASE_URL="postgresql://$USER@localhost:5432/sync_test_db" cargo test
```

#### Port Conflict Checking

Always check what's using port 5432:
```bash
lsof -i :5432  # Shows which process (Docker vs local) is using the port
```

#### SQLx Caching Issue (Secondary)

The SQLx prepared statement caching issue (see [issue #2885](https://github.com/launchbadge/sqlx/issues/2885)) can still occur:
- **Workarounds**: Add `.persistent(false)` to queries, use explicit type casts
- **Nuclear option**: Restart the database service completely

#### Lesson Learned

**Always ensure you're connecting to the same database you're migrating!** 
- Check port conflicts with `lsof -i :5432`
- Verify database connection parameters match your test setup
- Use consistent database URLs for migrations and tests

### Database Connection Issues

- Always check PostgreSQL is running: `psql -U $USER -d postgres -c "SELECT 1;"`
- Ensure the test database exists before running tests
- Use environment variables for database configuration

## Development Workflow

1. Make schema changes in migration files
2. Drop and recreate test database
3. Run `cargo clean` if changing types
4. Run tests with `DATABASE_URL` set
5. Use `--nocapture` flag to see test output and debug messages