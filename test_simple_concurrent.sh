#!/bin/bash
set -e

echo "🚀 Quick concurrent test..."

# Clean up
killall sync-server 2>/dev/null || true
createdb sync_test_db_local 2>/dev/null || echo "Database exists"
psql "postgres://$(whoami)@localhost:5432/sync_test_db_local" -c "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;" > /dev/null 2>&1

# Set environment
export DATABASE_URL="postgres://$(whoami)@localhost:5432/sync_test_db_local"
export SYNC_SERVER_URL="ws://localhost:8080"
export TEST_DATABASE_URL="postgres://$(whoami)@localhost:5432/sync_test_db_local"
export RUN_INTEGRATION_TESTS=1
export RUST_LOG=info

# Run migrations
cd sync-server && sqlx migrate run --database-url "$DATABASE_URL" && cd ..

# Start server
cd sync-server && cargo run &
SERVER_PID=$!
cd ..

# Wait for server
echo "Waiting for server..."
sleep 5

# Run test
echo "Running test..."
timeout 60 cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture 2>&1 | grep -E "(CLIENT:|Broadcasting|Successfully sent|User.*connected clients|should see all|assertion)" || true

# Cleanup
kill $SERVER_PID 2>/dev/null || true
echo "Done"