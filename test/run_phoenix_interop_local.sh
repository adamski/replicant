#!/bin/bash
#
# Phoenix interop test runner (Rust client <-> Elixir server).
#
# Stands up the Elixir replicant-server against a throwaway clean database,
# generates HMAC API credentials, and runs the client's phoenix_integration
# suite against it. The clean DB proves the frozen-identity contract holds
# end-to-end (same email -> same user_id on both sides) without touching the
# developer's local dev database.
#
# Usage:
#   test/run_phoenix_interop_local.sh                 # run the full suite
#   test/run_phoenix_interop_local.sh test_name       # run a single test filter
#
# Requires: a running PostgreSQL, Elixir/mix, and cargo.

set -euo pipefail

# --- Paths -------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLIENT_CRATE="$CLIENT_ROOT/replicant-client"
SERVER_DIR="${REPLICANT_SERVER_DIR:-$CLIENT_ROOT/../replicant-server}"

# --- Configuration -----------------------------------------------------------
DB_NAME="${INTEROP_DB_NAME:-replicant_interop_test}"
DB_USER="${INTEROP_DB_USER:-postgres}"
DB_PASS="${INTEROP_DB_PASS:-postgres}"
DB_HOST="${INTEROP_DB_HOST:-localhost}"
SERVER_PORT="${INTEROP_SERVER_PORT:-4010}"
DATABASE_URL="ecto://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME"
SERVER_LOG="${INTEROP_SERVER_LOG:-/tmp/replicant_interop_server.log}"

# Basic-auth vars are only required when PHX_SERVER is set (releases). Dev-mode
# `mix phx.server` does not set it, so the web UI guard stays inactive here.

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
log()  { echo -e "${GREEN}[$(date +'%H:%M:%S')] $1${NC}"; }
warn() { echo -e "${YELLOW}[$(date +'%H:%M:%S')] WARN: $1${NC}"; }
err()  { echo -e "${RED}[$(date +'%H:%M:%S')] ERROR: $1${NC}"; }

SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        log "Stopping server (PID $SERVER_PID)"
        kill "$SERVER_PID" 2>/dev/null || true
        sleep 1
        kill -9 "$SERVER_PID" 2>/dev/null || true
    fi
    # Sweep any stragglers bound to the interop port.
    local pids
    pids="$(lsof -ti :"$SERVER_PORT" 2>/dev/null || true)"
    [ -n "$pids" ] && kill -9 $pids 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- Preflight ---------------------------------------------------------------
[ -d "$SERVER_DIR" ] || { err "Server dir not found: $SERVER_DIR (set REPLICANT_SERVER_DIR)"; exit 1; }
command -v mix   >/dev/null || { err "mix not found on PATH"; exit 1; }
command -v cargo >/dev/null || { err "cargo not found on PATH"; exit 1; }
if ! PGPASSWORD="$DB_PASS" psql -U "$DB_USER" -h "$DB_HOST" -d postgres -c "SELECT 1;" >/dev/null 2>&1; then
    err "PostgreSQL not reachable as $DB_USER@$DB_HOST"; exit 1
fi

# Free the port before we start.
existing="$(lsof -ti :"$SERVER_PORT" 2>/dev/null || true)"
[ -n "$existing" ] && { warn "Killing processes on port $SERVER_PORT: $existing"; kill -9 $existing 2>/dev/null || true; sleep 1; }

# --- Clean database ----------------------------------------------------------
log "Recreating clean database '$DB_NAME'"
export DATABASE_URL MIX_ENV=dev
( cd "$SERVER_DIR"
  mix ecto.drop --quiet 2>/dev/null || true
  mix ecto.create --quiet
  mix ecto.migrate )

# --- Start server ------------------------------------------------------------
log "Starting Elixir server on port $SERVER_PORT (log: $SERVER_LOG)"
: > "$SERVER_LOG"
( cd "$SERVER_DIR" && PORT="$SERVER_PORT" DATABASE_URL="$DATABASE_URL" MIX_ENV=dev \
    exec mix phx.server ) >> "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Wait for the port to accept connections.
for i in $(seq 1 60); do
    if lsof -ti :"$SERVER_PORT" >/dev/null 2>&1; then break; fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then err "Server exited early:"; tail -30 "$SERVER_LOG"; exit 1; fi
    sleep 1
    [ "$i" -eq 60 ] && { err "Server did not come up within 60s"; tail -30 "$SERVER_LOG"; exit 1; }
done
log "Server is up"

# --- Credentials -------------------------------------------------------------
log "Generating API credentials"
CRED_OUTPUT="$( cd "$SERVER_DIR" && DATABASE_URL="$DATABASE_URL" MIX_ENV=dev \
    mix replicant.gen.credentials --name "integration-test" 2>/dev/null )"
API_KEY="$(echo "$CRED_OUTPUT"    | grep -Eo 'rpa_[a-f0-9]+' | head -1)"
API_SECRET="$(echo "$CRED_OUTPUT" | grep -Eo 'rps_[a-f0-9]+' | head -1)"
[ -n "$API_KEY" ] && [ -n "$API_SECRET" ] || { err "Failed to parse credentials"; echo "$CRED_OUTPUT"; exit 1; }

# --- Run interop suite -------------------------------------------------------
log "Running phoenix_integration suite against clean DB"
set +e
( cd "$CLIENT_CRATE" && \
  RUN_INTEGRATION_TESTS=1 \
  REPLICANT_API_KEY="$API_KEY" \
  REPLICANT_API_SECRET="$API_SECRET" \
  SYNC_SERVER_URL="ws://localhost:$SERVER_PORT/socket/websocket" \
  cargo test --test integration ${1:+"$1"} -- --test-threads=1 )
test_exit=$?
set -e

echo ""
if [ $test_exit -eq 0 ]; then
    log "✅ Interop suite passed"
else
    err "❌ Interop suite failed (server log tail below)"
    tail -40 "$SERVER_LOG"
fi
exit $test_exit
