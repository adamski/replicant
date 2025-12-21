#!/bin/bash

# Docker-based integration test runner
# Uses docker-compose.test.yml for PostgreSQL and builds server in Docker

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🐳 Docker Integration Test Runner${NC}"
echo "=================================="

# Configuration
TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5433/sync_test_db"
SERVER_PORT=8081

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    docker-compose -f docker-compose.test.yml down -v 2>/dev/null || true
    if [ ! -z "$SERVER_PID" ] && kill -0 $SERVER_PID 2>/dev/null; then
        kill $SERVER_PID
        wait $SERVER_PID 2>/dev/null || true
    fi
}

# Set up cleanup trap
trap cleanup EXIT

echo -e "${YELLOW}📦 Starting test database...${NC}"
docker-compose -f docker-compose.test.yml up -d

# Wait for PostgreSQL to be ready
echo -e "${YELLOW}⏳ Waiting for PostgreSQL...${NC}"
for i in {1..30}; do
    if docker-compose -f docker-compose.test.yml exec -T postgres-test pg_isready -U postgres -d sync_test_db >/dev/null 2>&1; then
        echo -e "${GREEN}✅ PostgreSQL is ready${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}❌ PostgreSQL failed to start${NC}"
        exit 1
    fi
    sleep 2
done

echo -e "${YELLOW}🔨 Building server...${NC}"
cargo build --bin replicant-server

echo -e "${YELLOW}📊 Running migrations...${NC}"
cd replicant-server
DATABASE_URL="$TEST_DATABASE_URL" sqlx migrate run
cd ..

echo -e "${YELLOW}🚀 Starting sync server...${NC}"
DATABASE_URL="$TEST_DATABASE_URL" RUST_LOG=warn PORT="$SERVER_PORT" \
    cargo run --bin replicant-server &
SERVER_PID=$!

# Wait for server to start (check if port is listening)
echo -e "${YELLOW}⏳ Waiting for sync server...${NC}"
for i in {1..30}; do
    if nc -z localhost $SERVER_PORT >/dev/null 2>&1; then
        echo -e "${GREEN}✅ Sync server is ready${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}❌ Sync server failed to start${NC}"
        echo "Server logs:"
        jobs -p | xargs -I {} sh -c 'if kill -0 {} 2>/dev/null; then echo "Server is running with PID {}"; else echo "Server process {} has died"; fi'
        exit 1
    fi
    sleep 2
done

echo -e "${YELLOW}🧪 Running integration tests...${NC}"
export RUN_INTEGRATION_TESTS=1
export TEST_DATABASE_URL="$TEST_DATABASE_URL"
export SYNC_SERVER_URL="ws://localhost:$SERVER_PORT/ws"
export RUST_TEST_THREADS=1

echo -e "${YELLOW}⏳ Waiting a moment for server to fully initialize...${NC}"
sleep 3

if cargo test integration_tests --no-fail-fast -- --test-threads=1 --nocapture; then
    echo -e "${GREEN}✅ All integration tests passed!${NC}"
else
    echo -e "${RED}❌ Some integration tests failed${NC}"
    exit 1
fi