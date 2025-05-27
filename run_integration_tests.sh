#!/bin/bash

set -e

echo "🚀 Starting integration test environment..."

# Clean up any existing containers
docker-compose -f docker-compose.integration.yml down -v 2>/dev/null || true

# Build the test environment
echo "📦 Building Docker images..."
docker-compose -f docker-compose.integration.yml build

# Start the services
echo "🔧 Starting services..."
docker-compose -f docker-compose.integration.yml up -d postgres-test

# Wait for PostgreSQL
echo "⏳ Waiting for PostgreSQL to be ready..."
until docker-compose -f docker-compose.integration.yml exec -T postgres-test pg_isready -U postgres; do
  sleep 1
done

# Start sync server
echo "🚀 Starting sync server..."
docker-compose -f docker-compose.integration.yml up -d sync-server-test

# Wait for sync server to be ready
echo "⏳ Waiting for sync server to be ready..."
sleep 5

# Run the tests
echo "🧪 Running integration tests..."
export RUN_INTEGRATION_TESTS=1
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
export SYNC_SERVER_URL="ws://localhost:8082"

cargo test -p sync-server --test integration --no-fail-fast -- --nocapture

TEST_EXIT_CODE=$?

# Show logs if tests failed
if [ $TEST_EXIT_CODE -ne 0 ]; then
  echo "❌ Tests failed! Showing server logs:"
  docker-compose -f docker-compose.integration.yml logs sync-server-test
fi

# Clean up
echo "🧹 Cleaning up..."
docker-compose -f docker-compose.integration.yml down -v

exit $TEST_EXIT_CODE