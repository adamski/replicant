#!/bin/bash
set -e

# Clean up any existing containers
docker-compose -f docker-compose.integration-fast.yml down -v

# Start the test environment
echo "🚀 Starting test environment for concurrent sessions test..."
docker-compose -f docker-compose.integration-fast.yml up -d postgres-test

# Wait for postgres
echo "⏳ Waiting for PostgreSQL..."
for i in {1..30}; do
    if docker-compose -f docker-compose.integration-fast.yml exec -T postgres-test pg_isready -U postgres > /dev/null 2>&1; then
        echo "✅ PostgreSQL is ready"
        break
    fi
    echo -n "."
    sleep 1
done

# Start the server with info logging to see our new logs
echo "🚀 Starting sync server with info logging..."
RUST_LOG=info docker-compose -f docker-compose.integration-fast.yml up -d sync-server-test

# Wait for server
echo "⏳ Waiting for sync server..."
for i in {1..30}; do
    if curl -s http://localhost:8082 > /dev/null 2>&1; then
        echo "✅ Sync server is ready"
        break
    fi
    echo -n "."
    sleep 1
done

# Start following server logs in background
echo "📋 Following server logs..."
docker-compose -f docker-compose.integration-fast.yml logs -f sync-server-test &
LOG_PID=$!

# Run the test
echo "🧪 Running concurrent sessions test..."
RUST_LOG=info RUN_INTEGRATION_TESTS=1 cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture

# Kill log following
kill $LOG_PID 2>/dev/null || true

# Cleanup
echo "🧹 Cleaning up..."
docker-compose -f docker-compose.integration-fast.yml down