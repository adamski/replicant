#!/bin/bash
set -e

echo "🚀 Starting services..."
docker-compose -f docker-compose.integration-fast.yml up -d

echo "⏳ Waiting for services..."
sleep 10

echo "🧪 Running test..."
RUST_LOG=info RUN_INTEGRATION_TESTS=1 cargo test --test integration test_concurrent_sessions -- --test-threads=1 --nocapture

echo "📋 Server logs:"
docker-compose -f docker-compose.integration-fast.yml logs sync-server-test | grep -E "(connected clients|Broadcasting|Successfully sent|First client|Added client|has [0-9]+ connected|Failed to send)" || true

echo "🧹 Cleaning up..."
docker-compose -f docker-compose.integration-fast.yml down