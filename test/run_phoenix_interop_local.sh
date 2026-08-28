#!/bin/bash
#
# Phoenix interop test runner (Rust client <-> stripped Elixir server).
#
# The merged replicant-server (>= #6) is a LIBRARY: it ships the sync socket
# (`lib/replicant_server/sync/socket.ex`) but no endpoint/router/HTTP server of
# its own — production hosts (entonal-web-app) mount the socket in their own
# endpoint. This harness supplies that host: it boots a minimal endpoint
# (modelled on the server's `test/support/test_endpoint.ex`) that mounts
# `ReplicantServer.Sync.Socket` on :4000, seeds ONE enrolled user + credential
# (plus one legacy nil-user credential for the negative test) by calling
# `ReplicantServer.Auth` directly, and runs the client's phoenix_integration
# suite against it on a throwaway clean database.
#
# Usage:
#   test/run_phoenix_interop_local.sh                 # run the full suite
#   test/run_phoenix_interop_local.sh test_name       # run a single test filter
#
# Requires: a running PostgreSQL, Elixir/mix, cargo, and curl.

set -euo pipefail

# --- Paths -------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLIENT_CRATE="$CLIENT_ROOT/replicant-client"

# --- Server pin --------------------------------------------------------------
# PINNED to a replicant-server SHA. It must include the claim_enrollment ->
# user_id change (PR #7, merged as 332a8ba) so the seed can export
# REPLICANT_TEST_USER_ID, plus the get_document channel op and the nil-hash
# rejection the sync suite exercises. Re-pin this as the server advances.
SERVER_REF="${REPLICANT_SERVER_REF:-dad45e5}"

# Where to prepare the server checkout. Locally we make a detached git worktree
# from a sibling replicant-server clone; in CI (no local clone) we git-clone.
SERVER_SRC="${REPLICANT_SERVER_SRC:-$CLIENT_ROOT/../replicant-server}"
SERVER_CLONE_URL="${REPLICANT_SERVER_CLONE_URL:-https://github.com/replicant-sync/replicant-server.git}"
SERVER_DIR="${REPLICANT_SERVER_DIR:-/tmp/replicant-server-interop}"

# Hex/Mix caches. Redirected to writable paths so `mix deps.get` can persist its
# registry cache even where $HOME/.hex is not writable (sandboxed shells).
export HEX_HOME="${INTEROP_HEX_HOME:-/tmp/replicant-interop-hex}"
export MIX_HOME="${INTEROP_MIX_HOME:-/tmp/replicant-interop-mix}"
mkdir -p "$HEX_HOME" "$MIX_HOME"

# --- Configuration -----------------------------------------------------------
DB_NAME="${INTEROP_DB_NAME:-replicant_server_test}"
DB_USER="${INTEROP_DB_USER:-postgres}"
DB_PASS="${INTEROP_DB_PASS:-postgres}"
DB_HOST="${INTEROP_DB_HOST:-localhost}"
SERVER_PORT="${INTEROP_SERVER_PORT:-4000}"
DATABASE_URL="ecto://$DB_USER:$DB_PASS@$DB_HOST/$DB_NAME"
SERVER_LOG="${INTEROP_SERVER_LOG:-/tmp/replicant_interop_server.log}"
TEST_EMAIL="${INTEROP_TEST_EMAIL:-integration-test@example.com}"
export MIX_ENV=test

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
log()  { echo -e "${GREEN}[$(date +'%H:%M:%S')] $1${NC}"; }
warn() { echo -e "${YELLOW}[$(date +'%H:%M:%S')] WARN: $1${NC}"; }
err()  { echo -e "${RED}[$(date +'%H:%M:%S')] ERROR: $1${NC}"; }

SERVER_PID=""
BOOT_SCRIPT=""
cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        log "Stopping server (PID $SERVER_PID)"
        kill "$SERVER_PID" 2>/dev/null || true
        sleep 1
        kill -9 "$SERVER_PID" 2>/dev/null || true
    fi
    local pids
    pids="$(lsof -ti :"$SERVER_PORT" 2>/dev/null || true)"
    [ -n "$pids" ] && kill -9 $pids 2>/dev/null || true
    [ -n "$BOOT_SCRIPT" ] && rm -f "$BOOT_SCRIPT"
}
trap cleanup EXIT INT TERM

# --- Preflight ---------------------------------------------------------------
command -v mix   >/dev/null || { err "mix not found on PATH"; exit 1; }
command -v cargo >/dev/null || { err "cargo not found on PATH"; exit 1; }
command -v curl  >/dev/null || { err "curl not found on PATH"; exit 1; }
if ! PGPASSWORD="$DB_PASS" psql -U "$DB_USER" -h "$DB_HOST" -d postgres -c "SELECT 1;" >/dev/null 2>&1; then
    err "PostgreSQL not reachable as $DB_USER@$DB_HOST"; exit 1
fi

existing="$(lsof -ti :"$SERVER_PORT" 2>/dev/null || true)"
[ -n "$existing" ] && { warn "Killing processes on port $SERVER_PORT: $existing"; kill -9 $existing 2>/dev/null || true; sleep 1; }

# --- Prepare pinned server checkout ------------------------------------------
if [ -d "$SERVER_DIR/.git" ] || [ -f "$SERVER_DIR/.git" ]; then
    log "Reusing server checkout at $SERVER_DIR (pinning to $SERVER_REF)"
    git -C "$SERVER_DIR" checkout --quiet --detach "$SERVER_REF" 2>/dev/null || \
        git -C "$SERVER_DIR" checkout --quiet "$SERVER_REF"
elif [ -d "$SERVER_SRC/.git" ]; then
    log "Creating detached worktree at $SERVER_DIR from $SERVER_SRC @ $SERVER_REF"
    git -C "$SERVER_SRC" worktree prune
    git -C "$SERVER_SRC" worktree add --force --detach "$SERVER_DIR" "$SERVER_REF"
else
    log "Cloning $SERVER_CLONE_URL into $SERVER_DIR @ $SERVER_REF"
    git clone "$SERVER_CLONE_URL" "$SERVER_DIR"
    git -C "$SERVER_DIR" checkout "$SERVER_REF"
fi

# The stripped server ships no HTTP adapter dependency (its own channel tests run
# with `server: false`). Inject Bandit so the endpoint can actually bind a WS
# port — mirroring what a production host app brings. Harness-local only.
if ! grep -q ':bandit' "$SERVER_DIR/mix.exs"; then
    log "Injecting Bandit HTTP adapter dependency (harness-only)"
    perl -0pi -e 's/(\{:jsonpatch,\s*"[^"]*"\})/$1,\n      {:bandit, "~> 1.0"}/' "$SERVER_DIR/mix.exs"
    grep -q ':bandit' "$SERVER_DIR/mix.exs" || { err "Failed to inject bandit dep"; exit 1; }
fi

# --- Build server ------------------------------------------------------------
log "Fetching + compiling server deps (MIX_ENV=test)"
( cd "$SERVER_DIR" && mix deps.get >/dev/null && mix compile >/dev/null )

# --- Clean database ----------------------------------------------------------
log "Recreating clean database '$DB_NAME'"
export DATABASE_URL
( cd "$SERVER_DIR"
  mix ecto.drop --quiet 2>/dev/null || true
  mix ecto.create --quiet
  mix ecto.migrate >/dev/null )

# --- Seed credentials --------------------------------------------------------
# One enrolled user+credential (bound user_id) via the enrollment flow, and one
# legacy nil-user credential via create_credential/1 for the negative test.
log "Seeding enrolled + legacy credentials for $TEST_EMAIL (stderr: $SERVER_LOG)"
: > "$SERVER_LOG"
SEED_LINE="$( cd "$SERVER_DIR" && TEST_EMAIL="$TEST_EMAIL" mix run -e '
  Ecto.Adapters.SQL.Sandbox.mode(ReplicantServer.Repo, :auto)
  email = System.get_env("TEST_EMAIL")
  {:ok, token} = ReplicantServer.Auth.request_enrollment(email)
  {:ok, creds} = ReplicantServer.Auth.claim_enrollment(email, token)
  {:ok, legacy} = ReplicantServer.Auth.create_credential("interop-legacy-shared")
  IO.puts("SEED #{creds.api_key} #{creds.secret} #{creds.user_id} #{legacy.api_key} #{legacy.secret}")
' 2>>"$SERVER_LOG" | grep '^SEED ' )"
read -r _ API_KEY API_SECRET TEST_USER_ID LEGACY_API_KEY LEGACY_API_SECRET <<<"$SEED_LINE"
[ -n "$API_KEY" ] && [ -n "$API_SECRET" ] && [ -n "$TEST_USER_ID" ] && \
[ -n "$LEGACY_API_KEY" ] && [ -n "$LEGACY_API_SECRET" ] || {
    err "Failed to seed credentials"; echo "$SEED_LINE"; tail -30 "$SERVER_LOG"; exit 1; }
log "Enrolled user_id=$TEST_USER_ID"

# --- Start server (minimal endpoint mounting the sync socket) ----------------
BOOT_SCRIPT="$(mktemp /tmp/replicant_interop_boot.XXXXXX.exs)"
cat > "$BOOT_SCRIPT" <<EOF
Application.put_env(:replicant_server, ReplicantServer.Sync.TestEndpoint,
  adapter: Bandit.PhoenixAdapter,
  http: [ip: {127, 0, 0, 1}, port: $SERVER_PORT],
  server: true,
  secret_key_base: "oD6r/Ez+1r8Dh1dGG7dZ8BQS3wcNOYQsXgrATKe1LCimCFRoO346xxuWJBbga1bE",
  pubsub_server: ReplicantServer.PubSub
)
Ecto.Adapters.SQL.Sandbox.mode(ReplicantServer.Repo, :auto)
{:ok, _} = ReplicantServer.Sync.TestEndpoint.start_link([])
IO.puts("ENDPOINT_STARTED")
Process.sleep(:infinity)
EOF

log "Starting minimal endpoint on port $SERVER_PORT (log: $SERVER_LOG)"
( cd "$SERVER_DIR" && exec mix run --no-halt "$BOOT_SCRIPT" ) >> "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Health-check: wait for the socket port to accept HTTP connections. The stripped
# server has no /health route, so any HTTP response (curl exit 0) means "up".
for i in $(seq 1 60); do
    if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$SERVER_PORT/" 2>/dev/null; then
        log "Server is up"; break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then err "Server exited early:"; tail -30 "$SERVER_LOG"; exit 1; fi
    sleep 1
    [ "$i" -eq 60 ] && { err "Server did not come up within 60s"; tail -30 "$SERVER_LOG"; exit 1; }
done

# --- Run interop suite -------------------------------------------------------
# INTEROP_TEST_CMD lets a consumer suite (e.g. entonal-common's
# TonalDBSyncIntegrationTest) run under this harness's boot/seed/teardown in
# place of the cargo suite. It runs with the same seeded-credential env.
log "Running ${INTEROP_TEST_CMD:-phoenix_integration suite} against clean DB"
set +e
( cd "$CLIENT_CRATE" && \
  RUN_INTEGRATION_TESTS=1 \
  REPLICANT_API_KEY="$API_KEY" \
  REPLICANT_API_SECRET="$API_SECRET" \
  REPLICANT_TEST_USER_ID="$TEST_USER_ID" \
  REPLICANT_LEGACY_API_KEY="$LEGACY_API_KEY" \
  REPLICANT_LEGACY_API_SECRET="$LEGACY_API_SECRET" \
  SYNC_SERVER_URL="ws://localhost:$SERVER_PORT/socket/websocket" \
  bash -c "${INTEROP_TEST_CMD:-cargo test --test integration ${1:+\"$1\"} -- --test-threads=1}" )
test_exit=$?
set -e

echo ""
if [ $test_exit -eq 0 ]; then
    log "Interop suite passed"
else
    err "Interop suite failed (server log tail below)"
    tail -40 "$SERVER_LOG"
fi
exit $test_exit
