#!/bin/bash
set -e

echo "🔍 Testing message flow with detailed logging..."

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

# Start server in background with logging
echo "🚀 Starting server..."
cd sync-server && cargo run > /tmp/server.log 2>&1 &
SERVER_PID=$!
cd ..

# Wait for server
echo "⏳ Waiting for server..."
for i in {1..15}; do
    if curl -s http://localhost:8080 > /dev/null 2>&1; then
        echo "✅ Server ready"
        break
    fi
    sleep 1
done

# Run test with filtered output
echo "🧪 Running test..."
cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture 2>&1 | \
    grep -E "(CLIENT:|SERVER:|Broadcasting|Successfully sent|Creating document|Received.*Created|get_all_documents|should see all)" | \
    head -100

echo ""
echo "📋 Server logs:"
tail -50 /tmp/server.log | grep -E "(Broadcasting|Successfully sent|connected clients)" || true

# Cleanup
kill $SERVER_PID 2>/dev/null || true
rm -f /tmp/server.log
echo "✅ Done"