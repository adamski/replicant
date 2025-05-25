#!/bin/bash

echo "Starting PostgreSQL for tests..."
docker-compose -f docker-compose.test.yml up -d

echo "Waiting for PostgreSQL to be ready..."
sleep 5

# Export test database URL
export TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"

echo "Running integration tests..."
cargo test --workspace -- --nocapture

# Clean up
echo "Stopping PostgreSQL..."
docker-compose -f docker-compose.test.yml down

echo "Integration tests complete!"