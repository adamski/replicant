#!/bin/bash
set -e

echo "🔍 Diagnostic test for message propagation..."

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

# Start server with full logging to file
echo "🚀 Starting server..."
cd sync-server && RUST_LOG=debug cargo run > /tmp/server_debug.log 2>&1 &
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

# Run test and capture output
echo "🧪 Running test..."
timeout 30 cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture > /tmp/test_output.log 2>&1 || true

# Extract key information
echo ""
echo "📊 Test Summary:"
echo "==============="

echo ""
echo "1️⃣ Document Creation:"
grep -E "CLIENT: Creating document locally|CLIENT: Successfully sent CreateDocument" /tmp/test_output.log | head -10 || echo "No document creation logs found"

echo ""
echo "2️⃣ Server Broadcasting:"
grep -E "Broadcasting DocumentCreated|Successfully sent to.*clients" /tmp/server_debug.log | head -10 || echo "No broadcasting logs found"

echo "" 
echo "3️⃣ Client Reception:"
grep -E "CLIENT: Received.*DocumentCreated|CLIENT: Successfully processed" /tmp/test_output.log | head -10 || echo "No reception logs found"

echo ""
echo "4️⃣ Final State:"
grep -E "Client [0-9]+: [0-9]+ documents|Convergence timeout" /tmp/test_output.log | tail -10 || echo "No final state found"

echo ""
echo "5️⃣ Errors:"
grep -E "ERROR|WARN|Failed" /tmp/test_output.log /tmp/server_debug.log | head -10 || echo "No errors found"

# Cleanup
kill $SERVER_PID 2>/dev/null || true
rm -f /tmp/server_debug.log /tmp/test_output.log
echo ""
echo "✅ Diagnostic complete"