#!/bin/bash

# Local integration test runner script
# This script starts the sync server locally, runs integration tests, and cleans up

set -e  # Exit on any error

# Configuration
DATABASE_URL="${DATABASE_URL:-postgresql://$USER@localhost:5432/sync_test_db_local}"
SERVER_PORT="${SERVER_PORT:-8080}"
SERVER_PID_FILE="/tmp/sync_server_test.pid"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log() {
    echo -e "${GREEN}[$(date +'%H:%M:%S')] $1${NC}"
}

warn() {
    echo -e "${YELLOW}[$(date +'%H:%M:%S')] WARNING: $1${NC}"
}

error() {
    echo -e "${RED}[$(date +'%H:%M:%S')] ERROR: $1${NC}"
}

# Cleanup function
cleanup() {
    log "Cleaning up..."
    
    # Kill server if running
    if [ -f "$SERVER_PID_FILE" ]; then
        local server_pid=$(cat "$SERVER_PID_FILE")
        if kill -0 "$server_pid" 2>/dev/null; then
            log "Stopping sync server (PID: $server_pid)"
            kill "$server_pid"
            # Wait for server to stop
            for i in {1..10}; do
                if ! kill -0 "$server_pid" 2>/dev/null; then
                    break
                fi
                sleep 1
            done
            # Force kill if still running
            if kill -0 "$server_pid" 2>/dev/null; then
                warn "Force killing server"
                kill -9 "$server_pid" 2>/dev/null || true
            fi
        fi
        rm -f "$SERVER_PID_FILE"
    fi
    
    # Kill any other sync-server processes
    pkill -f "sync-server" 2>/dev/null || true
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Check if database is accessible
log "Checking database connection..."
if ! psql "$DATABASE_URL" -c "SELECT 1;" >/dev/null 2>&1; then
    error "Cannot connect to database: $DATABASE_URL"
    error "Make sure PostgreSQL is running and the database exists"
    exit 1
fi

# Check if port is already in use
if lsof -i :$SERVER_PORT >/dev/null 2>&1; then
    error "Port $SERVER_PORT is already in use. Please stop any running servers."
    lsof -i :$SERVER_PORT
    exit 1
fi

# Build the project
log "Building sync-server..."
DATABASE_URL="$DATABASE_URL" cargo build --bin sync-server

# Start the server in background
log "Starting sync server on port $SERVER_PORT..."
DATABASE_URL="$DATABASE_URL" cargo run --bin sync-server > /tmp/sync_server_test.log 2>&1 &
server_pid=$!
echo $server_pid > "$SERVER_PID_FILE"

log "Server started with PID: $server_pid"

# Wait for server to be ready
log "Waiting for server to be ready..."
max_attempts=30
attempt=0

while [ $attempt -lt $max_attempts ]; do
    if curl -s "http://localhost:$SERVER_PORT" >/dev/null 2>&1; then
        log "Server is ready!"
        break
    fi
    
    # Check if server process is still running
    if ! kill -0 "$server_pid" 2>/dev/null; then
        error "Server process died unexpectedly"
        error "Server logs:"
        cat /tmp/sync_server_test.log
        exit 1
    fi
    
    attempt=$((attempt + 1))
    echo -n "."
    sleep 1
done

if [ $attempt -eq $max_attempts ]; then
    error "Server did not become ready in time"
    error "Server logs:"
    cat /tmp/sync_server_test.log
    exit 1
fi

# Run the integration tests
log "Running integration tests..."
if [ -n "$1" ]; then
    # Run specific test if provided
    log "Running specific test: $1"
    RUN_INTEGRATION_TESTS=1 DATABASE_URL="$DATABASE_URL" cargo test --test integration "$1" -- --nocapture
else
    # Run all integration tests
    RUN_INTEGRATION_TESTS=1 DATABASE_URL="$DATABASE_URL" cargo test --test integration -- --nocapture
fi

test_exit_code=$?

if [ $test_exit_code -eq 0 ]; then
    log "All integration tests passed! ✅"
else
    error "Some integration tests failed ❌"
    error "Server logs:"
    cat /tmp/sync_server_test.log
fi

log "Integration test run complete"
exit $test_exit_code