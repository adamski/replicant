#!/bin/bash

# Offline/Online Sync Integration Test Script
# This script tests the sync system's ability to handle offline scenarios
# by controlling the server lifecycle during the test

set -e  # Exit on any error

# Configuration
DATABASE_NAME="${DATABASE_NAME:-sync_offline_test_db}"
DATABASE_USER="${DATABASE_USER:-$USER}"
DATABASE_URL="postgresql://$DATABASE_USER@localhost:5432/$DATABASE_NAME"
SERVER_PORT="${SERVER_PORT:-8080}"
SERVER_PID_FILE="/tmp/sync_server_offline_test.pid"
SERVER_LOG_FILE="/tmp/sync_server_offline_test.log"
TEST_STATE_FILE="/tmp/sync_offline_test_state.json"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
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

info() {
    echo -e "${BLUE}[$(date +'%H:%M:%S')] INFO: $1${NC}"
}

phase() {
    echo -e "${MAGENTA}[$(date +'%H:%M:%S')] ═══ PHASE: $1 ═══${NC}"
}

# Kill all processes using a specific port
kill_port_processes() {
    local port=$1
    local pids=$(lsof -ti :$port 2>/dev/null || true)
    if [ -n "$pids" ]; then
        for pid in $pids; do
            kill -9 $pid 2>/dev/null || true
        done
        sleep 1
    fi
}

# Kill all replicant-server processes
kill_all_servers() {
    info "Killing any existing sync servers..."
    pkill -f "replicant-server" 2>/dev/null || true
    kill_port_processes $SERVER_PORT
    rm -f "$SERVER_PID_FILE"
    sleep 1
}

# Start the sync server
start_server() {
    log "Starting sync server..."
    
    # Start server in background
    DATABASE_URL="$DATABASE_URL" cargo run --bin replicant-server > "$SERVER_LOG_FILE" 2>&1 &
    local server_pid=$!
    echo $server_pid > "$SERVER_PID_FILE"
    
    # Wait for server to be ready
    log "Waiting for server to be ready..."
    local max_wait=30
    local waited=0
    while [ $waited -lt $max_wait ]; do
        if curl -s http://localhost:$SERVER_PORT/health >/dev/null 2>&1; then
            log "Server is ready (PID: $server_pid)"
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    
    error "Server failed to start within ${max_wait} seconds"
    tail -20 "$SERVER_LOG_FILE"
    return 1
}

# Stop the sync server
stop_server() {
    log "Stopping sync server..."
    if [ -f "$SERVER_PID_FILE" ]; then
        local server_pid=$(cat "$SERVER_PID_FILE")
        if kill -0 "$server_pid" 2>/dev/null; then
            kill "$server_pid" 2>/dev/null || true
            sleep 2
            # Force kill if still running
            if kill -0 "$server_pid" 2>/dev/null; then
                kill -9 "$server_pid" 2>/dev/null || true
            fi
        fi
        rm -f "$SERVER_PID_FILE"
    fi
    kill_port_processes $SERVER_PORT
}

# Setup database
setup_database() {
    log "Setting up test database..."
    
    # Drop and recreate database
    psql -U "$DATABASE_USER" -d postgres -c "DROP DATABASE IF EXISTS $DATABASE_NAME;" 2>/dev/null || true
    psql -U "$DATABASE_USER" -d postgres -c "CREATE DATABASE $DATABASE_NAME;"
    
    # Run migrations
    DATABASE_URL="$DATABASE_URL" sqlx migrate run --source replicant-server/migrations
}

# Cleanup function
cleanup() {
    log "Cleaning up..."
    stop_server
    rm -f "$TEST_STATE_FILE"
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

# Main test execution
log "🚀 Starting Offline/Online Sync Integration Test"
info "This test verifies sync recovery after network disconnection"

# Initial setup
phase "SETUP"
kill_all_servers
setup_database

# Build the project
log "Building replicant-server..."
DATABASE_URL="$DATABASE_URL" cargo build --bin replicant-server --release

log "Building test binary..."
cargo build --package replicant-server --test integration_tests --release

# Phase 1: Initial online sync
phase "1: INITIAL ONLINE SYNC"
start_server

# Run the test with phase 1
info "Creating clients and initial documents..."
export RUN_INTEGRATION_TESTS=1
export SYNC_SERVER_URL="ws://localhost:$SERVER_PORT"
export TEST_DATABASE_URL="$DATABASE_URL"
export OFFLINE_TEST_PHASE="phase1"
export OFFLINE_TEST_STATE_FILE="$TEST_STATE_FILE"

if ! cargo test --package replicant-server --test integration_tests integration_tests::test_offline_sync_phases::phase1_initial_sync -- --exact --nocapture; then
    error "Phase 1 failed"
    tail -50 "$SERVER_LOG_FILE"
    exit 1
fi

log "✓ Phase 1 complete - Initial sync successful"

# Phase 2: Offline operations
phase "2: OFFLINE OPERATIONS"
stop_server
info "Server is now offline - simulating network outage"

# Run offline operations
export OFFLINE_TEST_PHASE="phase2"

if ! cargo test --package replicant-server --test integration_tests integration_tests::test_offline_sync_phases::phase2_offline_changes -- --exact --nocapture; then
    error "Phase 2 failed"
    exit 1
fi

log "✓ Phase 2 complete - Offline changes made"

# Phase 3: Reconnection and sync recovery
phase "3: RECONNECTION AND SYNC RECOVERY"
start_server
info "Server is back online - testing sync recovery"

# Run sync recovery test
export OFFLINE_TEST_PHASE="phase3"

if ! cargo test --package replicant-server --test integration_tests integration_tests::test_offline_sync_phases::phase3_sync_recovery -- --exact --nocapture; then
    error "Phase 3 failed"
    tail -50 "$SERVER_LOG_FILE"
    exit 1
fi

log "✓ Phase 3 complete - Sync recovery successful"

# Final verification
phase "VERIFICATION"
info "Running final verification..."

export OFFLINE_TEST_PHASE="verify"
if ! cargo test --package replicant-server --test integration_tests integration_tests::test_offline_sync_phases::phase4_verification -- --exact --nocapture; then
    error "Verification failed"
    tail -50 "$SERVER_LOG_FILE"
    exit 1
fi

# Success!
echo ""
log "✅ OFFLINE/ONLINE SYNC TEST PASSED!"
info "All clients successfully recovered and synced after network disconnection"
echo ""

# Show summary from state file if it exists
if [ -f "$TEST_STATE_FILE" ]; then
    info "Test Summary:"
    jq -r '.summary // empty' "$TEST_STATE_FILE" 2>/dev/null || true
fi