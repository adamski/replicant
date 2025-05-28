#!/bin/bash
set -e

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

# Start the server with debug logging
echo "🚀 Starting sync server with debug logging..."
RUST_LOG=debug,h2=info,hyper=info docker-compose -f docker-compose.integration-fast.yml up -d sync-server-test

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

# Run just the concurrent sessions test
echo "🧪 Running concurrent sessions test with debug logging..."
RUST_LOG=debug,h2=info,hyper=info RUN_INTEGRATION_TESTS=1 cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture

# Show server logs
echo "📋 Server logs:"
docker-compose -f docker-compose.integration-fast.yml logs sync-server-test | tail -50

# Cleanup
echo "🧹 Cleaning up..."
docker-compose -f docker-compose.integration-fast.yml down