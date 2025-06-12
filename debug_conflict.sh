#!/bin/bash

# Simple debug script to understand conflict resolution
set -e

echo "🔍 Debugging Conflict Resolution Behavior"
echo "========================================"

# Start server
echo "🚀 Starting server..."
cargo run --bin sync-server > /tmp/debug_conflict.log 2>&1 &
SERVER_PID=$!
sleep 3

# Cleanup function
cleanup() {
    echo "🧹 Cleaning up..."
    kill $SERVER_PID 2>/dev/null || true
    pkill -f sync-server 2>/dev/null || true
    rm -f /tmp/debug_conflict_state.json
}
trap cleanup EXIT

# Check if server started
if ! curl -s http://localhost:8080/health > /dev/null; then
    echo "❌ Server failed to start"
    exit 1
fi

echo "✅ Server ready"
echo ""

# Run just Phase 1 to create the initial state
echo "📋 Phase 1: Creating test data..."
CONFLICT_TEST_STATE_FILE="/tmp/debug_conflict_state.json" \
CONFLICT_TEST_PHASE=phase1 \
cargo test --package sync-server --test integration conflict_phase1_setup_and_sync -- --nocapture --test-threads=1

if [ $? -ne 0 ]; then
    echo "❌ Phase 1 failed"
    exit 1
fi

echo "✅ Phase 1 complete"
echo ""

# Stop server for Phase 2
echo "🛑 Stopping server for offline phase..."
kill $SERVER_PID
sleep 2

# Run Phase 2 - offline edits
echo "📋 Phase 2: Making conflicting offline edits..."
CONFLICT_TEST_STATE_FILE="/tmp/debug_conflict_state.json" \
CONFLICT_TEST_PHASE=phase2 \
cargo test --package sync-server --test integration conflict_phase2_concurrent_offline_edits -- --nocapture --test-threads=1

if [ $? -ne 0 ]; then
    echo "❌ Phase 2 failed"
    exit 1
fi

echo "✅ Phase 2 complete"
echo ""

# Check what we have in the state file
echo "📊 Current test state:"
if [ -f "/tmp/debug_conflict_state.json" ]; then
    cat /tmp/debug_conflict_state.json | jq '.'
else
    echo "⚠️  No state file found"
fi

echo ""
echo "🎯 Now we have:"
echo "  1. A document that was initially synced"  
echo "  2. Client 1 made an offline edit"
echo "  3. Client 2 made a CONFLICTING offline edit to the same field"
echo "  4. Both changes are stored locally"
echo ""
echo "🔄 Next: Restart server and see what happens during sync..."
echo ""

# Restart server for Phase 3
echo "🚀 Restarting server for conflict resolution test..."
cargo run --bin sync-server >> /tmp/debug_conflict.log 2>&1 &
SERVER_PID=$!
sleep 3

if ! curl -s http://localhost:8080/health > /dev/null; then
    echo "❌ Server failed to restart"
    exit 1
fi

# Run Phase 3 with extra logging
echo "📋 Phase 3: Testing conflict resolution..."
RUST_LOG=debug \
CONFLICT_TEST_STATE_FILE="/tmp/debug_conflict_state.json" \
CONFLICT_TEST_PHASE=phase3 \
cargo test --package sync-server --test integration conflict_phase3_server_recovery_and_resolution -- --nocapture --test-threads=1

echo ""
echo "🎯 Conflict Resolution Test Complete!"
echo ""
echo "📊 Final Analysis:"
if [ -f "/tmp/debug_conflict_state.json" ]; then
    echo "Final state summary:"
    cat /tmp/debug_conflict_state.json | jq -r '.summary // "No summary available"'
else
    echo "⚠️  No final state available"
fi

echo ""
echo "📝 The key question: When both clients reconnect with conflicting offline edits,"
echo "   what happens? Do both edits survive, does one win, or are both lost?"