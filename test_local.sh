#!/bin/bash
set -e

# Local testing script - no Docker required
echo "🚀 Setting up local test environment..."

# Check if PostgreSQL is running locally
if ! pg_isready -h localhost -p 5432 > /dev/null 2>&1; then
    echo "❌ PostgreSQL not running locally. Please start it with:"
    echo "   brew services start postgresql"
    echo "   or: pg_ctl -D /usr/local/var/postgres start"
    exit 1
fi

echo "✅ PostgreSQL is running"

# Create test database if it doesn't exist
createdb sync_test_db_local 2>/dev/null || echo "Database sync_test_db_local already exists"

# Set environment variables for local testing
export DATABASE_URL="postgres://$(whoami)@localhost:5432/sync_test_db_local"
export SYNC_SERVER_URL="ws://localhost:8080"
export TEST_DATABASE_URL="postgres://$(whoami)@localhost:5432/sync_test_db_local"
export RUN_INTEGRATION_TESTS=1
export RUST_LOG=info

echo "🗄️  Database URL: $DATABASE_URL"

# Clean up any existing test data
psql "$DATABASE_URL" -c "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;" > /dev/null 2>&1

# Run migrations
echo "🔄 Running migrations..."
cd sync-server
sqlx migrate run --database-url "$DATABASE_URL"
cd ..

# Start the sync server in background
echo "🚀 Starting sync server locally..."
cd sync-server
cargo run &
SERVER_PID=$!
cd ..

# Wait for server to be ready (server runs on port 8080 by default)
echo "⏳ Waiting for sync server..."
for i in {1..30}; do
    if curl -s http://localhost:8080 > /dev/null 2>&1; then
        echo "✅ Sync server is ready"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "❌ Sync server failed to start"
        kill $SERVER_PID 2>/dev/null || true
        exit 1
    fi
    echo -n "."
    sleep 1
done

# Run the test
echo "🧪 Running concurrent sessions test locally..."
cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture

# Cleanup
echo "🧹 Cleaning up..."
kill $SERVER_PID 2>/dev/null || true
echo "✅ Done"