#!/bin/bash

set -e

echo "🚀 Starting integration test environment (fast mode with caching)..."

# Clean up any existing containers (but preserve volumes for caching)
docker-compose -f docker-compose.integration-fast.yml down 2>/dev/null || true

# Build the test environment (will use cache if available)
echo "📦 Building Docker images..."
docker-compose -f docker-compose.integration-fast.yml build

# Start the services
echo "🔧 Starting services..."
docker-compose -f docker-compose.integration-fast.yml up -d postgres-test

# Wait for PostgreSQL
echo "⏳ Waiting for PostgreSQL to be ready..."
until docker-compose -f docker-compose.integration-fast.yml exec -T postgres-test pg_isready -U postgres; do
  sleep 1
done

# Start sync server
echo "🚀 Starting sync server..."
docker-compose -f docker-compose.integration-fast.yml up -d sync-server-test

# Wait for sync server to be ready
echo "⏳ Waiting for sync server to be ready..."
for i in {1..30}; do
  if curl -s http://localhost:8082/health >/dev/null 2>&1; then
    echo "✅ Server is ready!"
    break
  fi
  echo -n "."
  sleep 1
done
echo

# Run the tests
echo "🧪 Running integration tests..."
export RUN_INTEGRATION_TESTS=1
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
export SYNC_SERVER_URL="ws://localhost:8082"

cargo test -p sync-server --test integration --no-fail-fast -- --test-threads=1 --nocapture

TEST_EXIT_CODE=$?

# Show logs if tests failed
if [ $TEST_EXIT_CODE -ne 0 ]; then
  echo "❌ Tests failed! Showing server logs:"
  docker-compose -f docker-compose.integration-fast.yml logs sync-server-test
fi

# Clean up containers but preserve volumes
echo "🧹 Cleaning up containers (keeping cache volumes)..."
docker-compose -f docker-compose.integration-fast.yml down

# To fully clean including volumes, run:
# docker-compose -f docker-compose.integration-fast.yml down -v

exit $TEST_EXIT_CODE