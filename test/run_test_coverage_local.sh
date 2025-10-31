#!/bin/bash

set -e  # Exit on any error

# Configuration
DATABASE_NAME="${DATABASE_NAME:-sync_test_db_local}"
DATABASE_USER="${DATABASE_USER:-$USER}"
DATABASE_URL="postgresql://$DATABASE_USER@localhost:5432/$DATABASE_NAME"
SERVER_PORT="${SERVER_PORT:-8080}"
SERVER_PID_FILE="/tmp/sync_server_test.pid"
SERVER_LOG_FILE="/tmp/sync_server_test.log"

# Test execution timeout (longer for sequential execution with full teardown)
TEST_TIMEOUT="${TEST_TIMEOUT:-600}" # 10 minutes for full suite

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
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

# Kill all processes using a specific port
kill_port_processes() {
    local port=$1
    info "Checking for processes on port $port..."

    local pids=$(lsof -ti :$port 2>/dev/null || true)
    if [ -n "$pids" ]; then
        warn "Found processes on port $port: $pids"
        for pid in $pids; do
            warn "Killing process $pid..."
            kill -9 $pid 2>/dev/null || true
        done
        sleep 1
    fi
}

# Kill all sync-server processes
kill_sync_servers() {
    info "Checking for running sync-server processes..."

    local pids=$(pgrep -f "sync-server" 2>/dev/null || true)
    if [ -n "$pids" ]; then
        warn "Found sync-server processes: $pids"
        for pid in $pids; do
            warn "Killing sync-server process $pid..."
            kill -9 $pid 2>/dev/null || true
        done
        sleep 1
    fi
}

# Database cleanup and setup
setup_database() {
    log "Setting up test database..."

    # Check if postgres is running
    if ! psql -U "$DATABASE_USER" -d postgres -c "SELECT 1;" >/dev/null 2>&1; then
        error "PostgreSQL is not running or not accessible"
        error "Make sure PostgreSQL is running and you can connect as user: $DATABASE_USER"
        exit 1
    fi

    # Drop existing database (if exists) and create new one
    info "Dropping existing test database (if exists)..."
    psql -U "$DATABASE_USER" -d postgres -c "DROP DATABASE IF EXISTS $DATABASE_NAME;" 2>/dev/null || true

    info "Creating fresh test database..."
    psql -U "$DATABASE_USER" -d postgres -c "CREATE DATABASE $DATABASE_NAME;"

    # Run migrations
    log "Running database migrations..."
    DATABASE_URL="$DATABASE_URL" sqlx migrate run --source sync-server/migrations
}

# Cleanup function
cleanup() {
    log "Cleaning up..."

    # Kill server if running
    if [ -f "$SERVER_PID_FILE" ]; then
        local server_pid=$(cat "$SERVER_PID_FILE")
        if kill -0 "$server_pid" 2>/dev/null; then
            log "Stopping sync server (PID: $server_pid)"
            kill "$server_pid" 2>/dev/null || true
            sleep 1
            # Force kill if still running
            if kill -0 "$server_pid" 2>/dev/null; then
                warn "Force killing server"
                kill -9 "$server_pid" 2>/dev/null || true
            fi
        fi
        rm -f "$SERVER_PID_FILE"
    fi

    # Kill any remaining sync-server processes
    kill_sync_servers
}

# Set trap to cleanup on exit
trap cleanup EXIT INT TERM

log "🚀 Starting robust integration test run with full isolation..."
info "Each test will get a fresh database and server instance for complete isolation"
info "Tests run sequentially to prevent database/server conflicts"

# Step 1: Kill any existing processes
kill_port_processes $SERVER_PORT
kill_sync_servers

# Step 2: Setup initial database (each test will recreate as needed)
setup_database


# Step 3: Install cargo-llvm-cov if it's missing
if cargo llvm-cov --version &> /dev/null; then
    echo "✅ cargo-llvm-cov is already installed."
else
    echo "❌ cargo-llvm-cov not found. Installing now..."

    # Check if cargo is available before attempting to install
    if ! command -v cargo &> /dev/null; then
        echo "🚨 Error: 'cargo' (Rust package manager) not found. Please install Rust and Cargo first."
        exit 1
    fi

    # Install the tool using cargo install
    if cargo +stable install cargo-llvm-cov --locked; then
        echo "🎉 Successfully installed cargo-llvm-cov."
    else
        echo "🚨 Error: Installation of cargo-llvm-cov failed."
        exit 1
    fi
fi

# Step 4: Verify installation
if cargo llvm-cov --version &> /dev/null; then
    echo "Verification successful."
else
    echo "Verification failed, even after attempted installation."
    exit 1
fi
# Step 5: Run the coverage tests (each test runs sequentially)
log "Running integration tests..."

# Export environment variables for tests
export RUN_INTEGRATION_TESTS=1
export SYNC_SERVER_URL="ws://localhost:$SERVER_PORT"
export TEST_DATABASE_URL="$DATABASE_URL"
export RUST_TEST_THREADS=1

# Run all tests sequentially for complete isolation
log "Running all integration tests sequentially (required for full teardown isolation)"
cargo llvm-cov --workspace --bin sync-server --test "*" --html  --open -- --test-threads 1

test_exit_code=$?

# Step 6: Report results
echo ""
if [ $test_exit_code -eq 0 ]; then
    log "✅ All integration tests passed!"
else
    error "❌ Some integration tests failed"
    warn "Server logs (last 100 lines):"
    tail -100 "$SERVER_LOG_FILE"
fi

log "Code coverage test run complete"
exit $test_exit_code